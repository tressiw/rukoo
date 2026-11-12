//! One encoder thread per video file. The main thread hands it frames through a queue of four;
//! when the encoder falls behind, the oldest frame goes. Frames held back on a still screen
//! (`hold`) replace each other; the writer puts the newest in at its next re-submit. The
//! platform's writer (`platform::VideoWriter`) lives and dies on this thread, and a finish that
//! hangs costs only this thread: `end` gives up after its timeout and the file stays readable to
//! its last fragment.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::platform::{VideoError, VideoMaker, VideoWriter, WriterOptions};
use crate::record::Ev;

/// Frames waiting for the encoder.
pub const QUEUE: usize = 4;
/// How long the sink writer may take to start (243 ms measured with NVENC).
pub const START_WAIT: Duration = Duration::from_secs(10);

/// What a new file needs.
pub struct VideoStart {
    pub path: PathBuf,
    /// Relative to the meeting folder, for the journal.
    pub file: String,
    pub seq: u64,
    /// Frame size; the video's own size follows `caps::video_size`.
    pub width: u32,
    pub height: u32,
    pub options: WriterOptions,
}

struct State<F> {
    frames: VecDeque<(F, u64)>,
    held: Option<(F, u64)>,
    tick: Option<u64>,
    end: bool,
    dropped: u64,
}

impl<F> Default for State<F> {
    fn default() -> State<F> {
        State {
            frames: VecDeque::new(),
            held: None,
            tick: None,
            end: false,
            dropped: 0,
        }
    }
}

struct Shared<F> {
    state: Mutex<State<F>>,
    ready: Condvar,
}

/// A running video file.
pub struct VideoThread<W: VideoWriter> {
    shared: Arc<Shared<W::Picture>>,
    done: Receiver<Result<u64, VideoError>>,
    end_t: Arc<AtomicU64>,
    pub seq: u64,
    pub file: String,
    pub path: PathBuf,
    /// Meeting time of media time 0.
    pub t: u64,
    pub width: u32,
    pub height: u32,
    pub video_width: u32,
    pub video_height: u32,
    pub encoder: String,
    pub hardware: bool,
}

/// Starts the thread and its writer, and waits until the encoder runs.
pub fn start<W: VideoWriter>(
    spec: VideoStart,
    mut maker: Box<dyn VideoMaker<File = W>>,
    to_main: SyncSender<Ev<W::Picture>>,
) -> Result<VideoThread<W>, VideoError> {
    let shared = Arc::new(Shared {
        state: Mutex::new(State::default()),
        ready: Condvar::new(),
    });
    let start_t = spec.options.start_t;
    let end_t = Arc::new(AtomicU64::new(start_t));
    let (ready_tx, ready_rx) = mpsc::channel();
    let (done_tx, done) = mpsc::channel();
    let (thread_shared, thread_end_t, seq) = (shared.clone(), end_t.clone(), spec.seq);
    let VideoStart {
        path,
        file,
        width,
        height,
        options,
        ..
    } = spec;
    let thread_path = path.clone();
    std::thread::Builder::new()
        .name(format!("video-{seq}"))
        .spawn(move || {
            let writer = match maker.create(&thread_path, width, height, options) {
                Ok(writer) => {
                    let (name, hardware) = writer.encoder();
                    let _ = ready_tx.send(Ok((name.to_string(), hardware, writer.video_size())));
                    writer
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            run(
                writer,
                &thread_shared,
                &to_main,
                seq,
                &thread_end_t,
                done_tx,
            );
        })
        .map_err(|e| VideoError::Encoder(format!("can't start the video thread: {e}")))?;
    match ready_rx.recv_timeout(START_WAIT) {
        Ok(Ok((encoder, hardware, (video_width, video_height)))) => Ok(VideoThread {
            shared,
            done,
            end_t,
            seq,
            file,
            path,
            t: start_t,
            width,
            height,
            video_width,
            video_height,
            encoder,
            hardware,
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            shared.state.lock().unwrap_or_else(|e| e.into_inner()).end = true;
            shared.ready.notify_all();
            Err(VideoError::Encoder(
                "the encoder took too long to start".into(),
            ))
        }
    }
}

fn run<W: VideoWriter>(
    mut writer: W,
    shared: &Shared<W::Picture>,
    to_main: &SyncSender<Ev<W::Picture>>,
    seq: u64,
    end_t: &AtomicU64,
    done: mpsc::Sender<Result<u64, VideoError>>,
) {
    let mut failed = false;
    loop {
        let (frames, held, tick, end) = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            while state.frames.is_empty() && state.tick.is_none() && !state.end {
                state = shared.ready.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            (
                std::mem::take(&mut state.frames),
                state.held.take(),
                state.tick.take(),
                state.end,
            )
        };
        if !failed {
            let mut result = Ok(());
            for (picture, boot) in frames {
                result = writer.write(&picture, boot);
                if result.is_err() {
                    break;
                }
            }
            if let Some((picture, boot)) = held {
                writer.hold(&picture, boot);
            }
            if let (Ok(()), Some(boot)) = (&result, tick) {
                result = writer.tick(boot);
            }
            // The file ends on the newest picture, and `end_t` covers it.
            if result.is_ok() && end {
                result = writer.flush();
            }
            end_t.store(writer.end_t(), Ordering::SeqCst);
            if let Err(error) = result {
                failed = true;
                report(shared, to_main, Ev::VideoError { seq, error });
            }
        }
        if end {
            let result = if failed {
                Ok(writer.abandon())
            } else {
                writer.finish()
            };
            let _ = done.send(result);
            return;
        }
    }
}

/// Sends an error to the main thread. The main thread may be waiting in `end` for this very
/// thread, so once the file is ending a full queue isn't waited for.
fn report<F>(shared: &Shared<F>, to_main: &SyncSender<Ev<F>>, mut event: Ev<F>) {
    loop {
        match to_main.try_send(event) {
            Ok(()) | Err(mpsc::TrySendError::Disconnected(_)) => return,
            Err(mpsc::TrySendError::Full(back)) => {
                if shared.state.lock().unwrap_or_else(|e| e.into_inner()).end {
                    return;
                }
                event = back;
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

impl<W: VideoWriter> VideoThread<W> {
    /// Queues a frame; with four waiting, the oldest goes.
    pub fn send(&self, picture: W::Picture, boot: u64) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.frames.len() >= QUEUE {
            state.frames.pop_front();
            state.dropped += 1;
        }
        if state.held.as_ref().is_some_and(|(_, at)| *at <= boot) {
            state.held = None;
        }
        state.frames.push_back((picture, boot));
        self.shared.ready.notify_one();
    }

    /// Keeps a frame of a still screen as the newest picture, for the writer's next re-submit.
    /// It isn't a sample of its own, and a newer one replaces it.
    pub fn hold(&self, picture: W::Picture, boot: u64) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.held = Some((picture, boot));
    }

    /// Lets the writer put in the held frame, or its last frame again, after a quarter second
    /// without a sample.
    pub fn tick(&self, boot_now: u64) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.tick = Some(boot_now);
        self.shared.ready.notify_one();
    }

    /// The meeting time where the written video ends, so far.
    pub fn end_t(&self) -> u64 {
        self.end_t.load(Ordering::SeqCst)
    }

    /// Frames dropped because the encoder fell behind.
    pub fn dropped(&self) -> u64 {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dropped
    }

    /// Writes what is queued, finalizes the file and returns its size, with the meeting time
    /// where the written video ends. That time is read once the thread wrote the frames still
    /// queued, so it covers them. After `timeout` the thread is left behind, the file stays as it
    /// is and the time is as far as the thread got.
    pub fn end(self, timeout: Duration) -> (Result<u64, VideoError>, u64) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.end = true;
        }
        self.shared.ready.notify_all();
        let result = match self.done.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => Err(VideoError::Encoder(format!(
                "finishing the video took longer than {} s",
                timeout.as_secs()
            ))),
        };
        (result, self.end_t())
    }
}
