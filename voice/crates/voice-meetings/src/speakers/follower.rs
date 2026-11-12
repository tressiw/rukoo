//! The speaker follower: one thread per meeting labelled by Nemotron on this computer.
//!
//! It reads the recorded call segments by meeting time, as the pipeline does, and feeds them to a
//! diarizer stream in at most 1 s pieces. The stream's frame probabilities go through the
//! binarizer, its turn events through the stream clock into meeting time, and the turns into
//! `transcript.db`, together with the settled position: the meeting time before which every turn
//! is final. The pipeline cuts call windows only up to that position.
//!
//! A stream that fails is replaced by a new one, a new epoch, which first hears a reel of the
//! voices the meeting already knows (`matching.rs`), so ids stay put. Three failures in ten
//! minutes, or a runtime that can't run here, stop labelling for the meeting; recording and
//! transcription go on. The follower shares nothing with the pipeline but `transcript.db` and
//! its status.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use voice_core::speakers::{
    DiarError, DiarErrorKind, DiarStream, Diarizer, Probs, CHUNK_FRAMES, MODEL_FRAME_SAMPLES,
    RIGHT_CONTEXT_FRAMES,
};
use voice_protocol::{MeetingState, Track};

use crate::library::MeetingDir;
use crate::pipeline::{dictating, has_open_segment, read_track, track_horizon, DRIFT_GAP};
use crate::reader::{TrackReader, PER_MS};
use crate::speakers::binarizer::{BinarizerParams, LiveBinarizer, TurnEvent};
use crate::speakers::clock::StreamClock;
use crate::speakers::matching::{build_reel, match_labels};
use crate::speakers::turns::Turn;
use crate::speakers::{LabelState, LabelStatus};
use crate::transcript::{LabelBatch, MapRow, Transcript, TurnRow, SETTLED_KEY};
use crate::watcher::TimelineWatcher;

/// At most 1 s of audio per push.
pub(crate) const PUSH_MAX: u64 = 16_000;
/// Wait for at least 0.1 s unless the audio ends there.
pub(crate) const PUSH_MIN: u64 = 1_600;
/// The sleep when nothing is ready.
pub(crate) const POLL: Duration = Duration::from_millis(100);
/// Zeros pushed once at a settled gap, so the stream commits and settles the audio before it
/// wherever that audio ends in a chunk: a whole chunk, the frame of right context the runtime
/// waits for, and the 0.525 s the binarizer settles behind the committed frames, in whole frames
/// of 80 ms. 46 frames, 3.68 s.
pub(crate) const PAD: u64 =
    (CHUNK_FRAMES as u64 + RIGHT_CONTEXT_FRAMES as u64 + SETTLE_FRAMES) * MODEL_FRAME_SAMPLES;
/// The binarizer's settle lag, `max(min_on, pad_onset + min_off)` = 0.525 s, in 80 ms frames.
const SETTLE_FRAMES: u64 = 7;
/// The pipeline stops waiting for labels when the settled position hasn't moved for this long
/// while audio waits.
pub(crate) const STALL: Duration = Duration::from_secs(20);
/// The waits before a new stream after a failure.
pub(crate) const BACKOFF: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
/// This many failures within `FAILURE_WINDOW` stop labelling for the meeting.
pub(crate) const MAX_FAILURES: u32 = 3;
pub(crate) const FAILURE_WINDOW: Duration = Duration::from_secs(600);
/// Labels wait while the user dictates, up to two minutes, as recognition does.
pub(crate) const DICTATION_YIELD_MS: u64 = 120_000;
/// Mistral's stream: 100 ms pieces, at most twice real time, 30 s for its last answers, and
/// reconnects after 2 s doubling up to 60 s.
pub(crate) const STREAM_CHUNK: u64 = 1_600;
pub(crate) const STREAM_SPEED: f64 = 2.0;
pub(crate) const STREAM_DRAIN: Duration = Duration::from_secs(30);
pub(crate) const STREAM_RETRY_FIRST: Duration = Duration::from_secs(2);
pub(crate) const STREAM_RETRY_MAX: Duration = Duration::from_secs(60);
/// In a development copy's log: worker calls slower than this get a line of their own, a
/// failed one comes with how long the calls before it took, and a summary comes once a minute.
const SLOW_CALL: Duration = Duration::from_secs(1);
const CALLS_BEFORE_A_FAILURE: usize = 20;
const CALL_SUMMARY: Duration = Duration::from_secs(60);

/// The follower's timings, so tests can shrink them. The defaults are the constants above.
#[derive(Clone, Debug, PartialEq)]
pub struct FollowerTiming {
    pub push_max: u64,
    pub push_min: u64,
    pub poll: Duration,
    pub pad: u64,
    pub stall: Duration,
    pub backoff: Vec<Duration>,
    pub max_failures: u32,
    pub failure_window: Duration,
    pub dictation_yield_ms: u64,
    pub stream_chunk: u64,
    pub stream_speed: f64,
    pub stream_drain: Duration,
    pub stream_retry_first: Duration,
    pub stream_retry_max: Duration,
    /// Logs how long the worker's calls take, in a development copy: a worker that slows down
    /// under load looks different from one that hangs.
    pub log_calls: bool,
}

impl Default for FollowerTiming {
    fn default() -> FollowerTiming {
        FollowerTiming {
            push_max: PUSH_MAX,
            push_min: PUSH_MIN,
            poll: POLL,
            pad: PAD,
            stall: STALL,
            backoff: BACKOFF.to_vec(),
            max_failures: MAX_FAILURES,
            failure_window: FAILURE_WINDOW,
            dictation_yield_ms: DICTATION_YIELD_MS,
            stream_chunk: STREAM_CHUNK,
            stream_speed: STREAM_SPEED,
            stream_drain: STREAM_DRAIN,
            stream_retry_first: STREAM_RETRY_FIRST,
            stream_retry_max: STREAM_RETRY_MAX,
            log_calls: false,
        }
    }
}

/// How long the worker's calls took, for a development copy's log.
struct CallLog {
    since: Instant,
    /// Calls since `since`, and the audio they pushed.
    took: Vec<Duration>,
    samples: u64,
    /// The last calls, newest last.
    recent: VecDeque<Duration>,
}

impl CallLog {
    fn new(now: Instant) -> CallLog {
        CallLog {
            since: now,
            took: Vec::new(),
            samples: 0,
            recent: VecDeque::new(),
        }
    }

    /// One worker call that pushed `samples` and took `took`: the lines to log.
    fn lines(
        &mut self,
        call: &str,
        samples: usize,
        took: Duration,
        failed: Option<&DiarError>,
        now: Instant,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let ms = |d: &Duration| d.as_millis();
        if let Some(error) = failed {
            let before: Vec<String> = self.recent.iter().map(|d| ms(d).to_string()).collect();
            lines.push(format!(
                "speaker worker {call} of {samples} samples failed after {} ms ({:?}); the {} calls before it took {} ms",
                ms(&took),
                error.kind,
                before.len(),
                before.join(", ")
            ));
        } else if took >= SLOW_CALL {
            lines.push(format!(
                "speaker worker {call} of {samples} samples took {} ms",
                ms(&took)
            ));
        }
        self.took.push(took);
        self.samples += samples as u64;
        self.recent.push_back(took);
        if self.recent.len() > CALLS_BEFORE_A_FAILURE {
            self.recent.pop_front();
        }
        let span = now.duration_since(self.since);
        if span >= CALL_SUMMARY {
            let mut sorted = self.took.clone();
            sorted.sort();
            let busy: Duration = sorted.iter().sum();
            lines.push(format!(
                "speaker worker in the last {} s: {} calls, {:.1} s of audio in {:.1} s, median {} ms, slowest {} ms",
                span.as_secs(),
                sorted.len(),
                self.samples as f64 / 16_000.0,
                busy.as_secs_f64(),
                ms(&sorted[sorted.len() / 2]),
                ms(&sorted[sorted.len() - 1])
            ));
            self.since = now;
            self.took.clear();
            self.samples = 0;
        }
        lines
    }
}

/// What a follower thread tells the pipeline and Settings.
pub(crate) struct FollowerShared {
    inner: Mutex<SharedInner>,
    stop: AtomicBool,
    /// Held while the follower writes to `transcript.db`, and by a stop, so once
    /// [`FollowerShared::request_stop`] returns the follower writes nothing more.
    writing: Mutex<()>,
}

struct SharedInner {
    status: LabelStatus,
    /// When the settled position or the state last changed.
    progress: Instant,
    /// The last failure, for the log.
    error: Option<String>,
}

/// A copy of a follower's status at one moment.
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub state: LabelState,
    pub settled_ms: Option<u64>,
    pub progress: Instant,
    pub error: Option<String>,
}

impl FollowerShared {
    pub(crate) fn new(mode: &str) -> Arc<FollowerShared> {
        Arc::new(FollowerShared {
            inner: Mutex::new(SharedInner {
                status: LabelStatus {
                    mode: mode.to_owned(),
                    state: LabelState::Starting,
                    reason: None,
                    epoch: 0,
                    speakers: 0,
                    settled_ms: None,
                    worker_pid: None,
                },
                progress: Instant::now(),
                error: None,
            }),
            stop: AtomicBool::new(false),
            writing: Mutex::new(()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SharedInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn status(&self) -> LabelStatus {
        self.lock().status.clone()
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        let inner = self.lock();
        Snapshot {
            state: inner.status.state,
            settled_ms: inner.status.settled_ms,
            progress: inner.progress,
            error: inner.error.clone(),
        }
    }

    pub(crate) fn set_state(&self, state: LabelState) {
        let mut inner = self.lock();
        if inner.status.state != state {
            inner.status.state = state;
            inner.progress = Instant::now();
        }
    }

    pub(crate) fn fail(&self, reason: &str, error: String) {
        let mut inner = self.lock();
        inner.status.state = LabelState::Failed;
        inner.status.reason = Some(reason.to_owned());
        inner.status.worker_pid = None;
        inner.error = Some(error);
        inner.progress = Instant::now();
    }

    pub(crate) fn note_error(&self, error: String) {
        self.lock().error = Some(error);
    }

    pub(crate) fn set_settled(&self, settled_ms: u64) {
        let mut inner = self.lock();
        if inner.status.settled_ms != Some(settled_ms) {
            inner.status.settled_ms = Some(settled_ms);
            inner.progress = Instant::now();
        }
    }

    /// Counts as progress without a change: the follower waits on purpose.
    pub(crate) fn touch(&self) {
        self.lock().progress = Instant::now();
    }

    pub(crate) fn update(&self, f: impl FnOnce(&mut LabelStatus)) {
        f(&mut self.lock().status);
    }

    pub(crate) fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Asks the thread to stop. Waits for a write that is under way, so after this returns the
    /// follower never writes again, even when a worker call keeps its thread busy for a while.
    pub(crate) fn request_stop(&self) {
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Leave to write to `transcript.db`, held until the guard drops; `None` once the follower
    /// was asked to stop. Never taken twice on one thread.
    pub(crate) fn writing(&self) -> Option<std::sync::MutexGuard<'_, ()>> {
        let guard = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        (!self.stopping()).then_some(guard)
    }

    /// Sleeps for `wait`, waking early when asked to stop. False when stopping.
    pub(crate) fn sleep(&self, wait: Duration) -> bool {
        let until = Instant::now() + wait;
        while !self.stopping() {
            let now = Instant::now();
            if now >= until {
                return true;
            }
            thread::sleep((until - now).min(Duration::from_millis(10)));
        }
        false
    }
}

/// A running follower. Dropping it asks the thread to stop without waiting.
pub(crate) struct FollowerHandle {
    shared: Arc<FollowerShared>,
    thread: Option<JoinHandle<()>>,
}

impl FollowerHandle {
    pub(crate) fn spawn(
        name: String,
        shared: Arc<FollowerShared>,
        body: impl FnOnce(Arc<FollowerShared>) + Send + 'static,
    ) -> FollowerHandle {
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name(name)
                .spawn(move || body(shared))
        };
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(e) => {
                shared.fail(
                    "runtimeFailed",
                    format!("Voice couldn't start speaker labels: {e}"),
                );
                None
            }
        };
        FollowerHandle { shared, thread }
    }

    pub(crate) fn shared(&self) -> &Arc<FollowerShared> {
        &self.shared
    }

    /// Stops the thread and waits up to 2 s for it; a diarizer call that hangs is left to its
    /// own timeout, and the stream it holds ends its worker when the thread ends.
    pub(crate) fn stop(mut self) {
        self.shared.request_stop();
        if let Some(thread) = self.thread.take() {
            let until = Instant::now() + Duration::from_secs(2);
            while !thread.is_finished() && Instant::now() < until {
                thread::sleep(Duration::from_millis(5));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for FollowerHandle {
    fn drop(&mut self) {
        self.shared.request_stop();
    }
}

/// Starts followers.
pub(crate) struct Follower;

impl Follower {
    /// Labels the call track of `dir` with streams from `diarizer` until the recording stopped
    /// and every call sample was heard. `shared` comes from [`FollowerShared::new`], so the
    /// pipeline can hold on to it before the thread runs.
    pub(crate) fn start(
        dir: MeetingDir,
        diarizer: Arc<dyn Diarizer>,
        timing: FollowerTiming,
        shared: Arc<FollowerShared>,
    ) -> FollowerHandle {
        let name = format!("speakers-{}", dir.id());
        FollowerHandle::spawn(name, shared, move |shared| {
            run(dir, diarizer, timing, shared);
        })
    }
}

/// Why an epoch ended early.
enum Fail {
    Diar(DiarError),
    Store(String),
}

impl From<DiarError> for Fail {
    fn from(e: DiarError) -> Fail {
        Fail::Diar(e)
    }
}

impl From<rusqlite::Error> for Fail {
    fn from(e: rusqlite::Error) -> Fail {
        Fail::Store(format!("the transcript store failed: {e}"))
    }
}

impl From<io::Error> for Fail {
    fn from(e: io::Error) -> Fail {
        Fail::Store(format!("the meeting folder can't be read: {e}"))
    }
}

impl Fail {
    fn message(&self) -> String {
        match self {
            Fail::Diar(e) => e.message.clone(),
            Fail::Store(m) => m.clone(),
        }
    }
}

/// How an epoch ended without a failure.
enum End {
    /// The recording stopped and every call sample was heard.
    Finished,
    /// Asked to stop, or the meeting is gone.
    Stopped,
}

struct Ctx {
    dir: MeetingDir,
    diarizer: Arc<dyn Diarizer>,
    timing: FollowerTiming,
    shared: Arc<FollowerShared>,
    transcript: Transcript,
    watcher: TimelineWatcher,
    lengths: HashMap<(String, u64), u64>,
    /// In a development copy only.
    calls: Option<CallLog>,
}

fn run(
    dir: MeetingDir,
    diarizer: Arc<dyn Diarizer>,
    timing: FollowerTiming,
    shared: Arc<FollowerShared>,
) {
    let transcript = loop {
        if shared.stopping() || !dir.path().is_dir() {
            return;
        }
        match Transcript::open(dir.path()) {
            Ok(t) => break t,
            Err(e) => {
                shared.note_error(format!("the transcript store failed: {e}"));
                shared.sleep(timing.poll * 10);
            }
        }
    };
    if let Ok(Some(settled)) = transcript.meta_u64(SETTLED_KEY) {
        shared.set_settled(settled);
    }
    let mut ctx = Ctx {
        watcher: TimelineWatcher::new(dir.path()),
        dir,
        diarizer,
        calls: timing.log_calls.then(|| CallLog::new(Instant::now())),
        timing,
        shared,
        transcript,
        lengths: HashMap::new(),
    };
    let mut failures: Vec<Instant> = Vec::new();
    let mut reason = None;
    loop {
        if ctx.shared.stopping() || !ctx.dir.path().is_dir() {
            return;
        }
        let ended = ctx.epoch(&mut reason);
        match ended {
            Ok(End::Finished) => {
                ctx.shared.update(|s| s.worker_pid = None);
                ctx.shared.set_state(LabelState::Finished);
                return;
            }
            Ok(End::Stopped) => return,
            // A follower that was replaced while a worker call ran leaves the store to the new
            // one: closing turns now would cut the new follower's open turns.
            Err(_) if ctx.shared.stopping() => return,
            Err(Fail::Diar(e))
                if matches!(
                    e.kind,
                    DiarErrorKind::Cpu | DiarErrorKind::Runtime | DiarErrorKind::Model
                ) =>
            {
                // The runtime can't run here; another stream wouldn't either.
                eprintln!(
                    "voice-meetings: {}: speaker labels can't run: {}",
                    ctx.dir.id(),
                    e.message
                );
                ctx.shared.fail("runtimeFailed", e.message);
                return;
            }
            Err(fail) => {
                let message = fail.message();
                eprintln!(
                    "voice-meetings: {}: speaker labels stopped: {message}",
                    ctx.dir.id()
                );
                // The next stream hears everything after the settled position again.
                {
                    let Some(_writing) = ctx.shared.writing() else {
                        return;
                    };
                    if let Ok(settled) = ctx.transcript.meta_u64(SETTLED_KEY) {
                        let _ = ctx.transcript.close_open_turns(settled.unwrap_or(0));
                    }
                }
                let now = Instant::now();
                failures.push(now);
                failures.retain(|t| now.duration_since(*t) < ctx.timing.failure_window);
                if failures.len() as u32 >= ctx.timing.max_failures {
                    ctx.shared.fail("runtimeFailed", message);
                    return;
                }
                ctx.shared.note_error(message);
                ctx.shared.update(|s| s.worker_pid = None);
                ctx.shared.set_state(LabelState::Restarting);
                let backoff = &ctx.timing.backoff;
                let wait = backoff
                    .get(failures.len() - 1)
                    .or(backoff.last())
                    .copied()
                    .unwrap_or_default();
                if !ctx.shared.sleep(wait) {
                    return;
                }
                reason = Some("crash");
            }
        }
    }
}

impl Ctx {
    /// Reads new timeline lines. False when the meeting folder is gone.
    fn poll(&mut self) -> Result<bool, Fail> {
        if !self.dir.path().is_dir() {
            return Ok(false);
        }
        self.watcher.poll()?;
        Ok(true)
    }

    fn reader(&mut self) -> TrackReader {
        read_track(
            self.dir.path(),
            Track::Call,
            self.watcher.mapper(),
            &mut self.lengths,
        )
    }

    fn record(&mut self, batch: LabelBatch, labeller: &Labeller) -> Result<(), Fail> {
        // A worker call can outlast the stop of a replaced follower; what it returns then
        // belongs to no follower, and the loop ends at its next look.
        let Some(_writing) = self.shared.writing() else {
            return Ok(());
        };
        if !batch.is_empty() {
            self.transcript.record_labels(&batch)?;
        }
        if let Some(settled) = batch.settled {
            self.shared.set_settled(settled);
        }
        let speakers = labeller.known.len() as u32;
        self.shared.update(|s| s.speakers = speakers);
        Ok(())
    }

    fn push(
        &mut self,
        stream: &mut dyn DiarStream,
        labeller: &mut Labeller,
        pcm: &[i16],
    ) -> Result<(), Fail> {
        let probs = self.timed("push", pcm.len(), || stream.push(pcm))?;
        let batch = labeller.take(probs)?;
        self.record(batch, labeller)?;
        if labeller.matched {
            self.shared.set_state(LabelState::Listening);
        }
        Ok(())
    }

    /// A worker call, and in a development copy a note of how long it took.
    fn timed<T>(
        &mut self,
        call: &str,
        samples: usize,
        f: impl FnOnce() -> Result<T, DiarError>,
    ) -> Result<T, DiarError> {
        let started = Instant::now();
        let result = f();
        if let Some(log) = &mut self.calls {
            let now = Instant::now();
            for line in log.lines(call, samples, now - started, result.as_ref().err(), now) {
                eprintln!("voice-meetings: {}: {line}", self.dir.id());
            }
        }
        result
    }

    /// Pushes `len` zeros that belong to no meeting time.
    fn pad(&mut self, stream: &mut dyn DiarStream, labeller: &mut Labeller) -> Result<(), Fail> {
        let mut left = self.timing.pad;
        while left > 0 {
            let len = left.min(self.timing.push_max.max(1));
            labeller.clock.pad(len);
            self.push(stream, labeller, &vec![0i16; len as usize])?;
            left -= len;
        }
        Ok(())
    }

    /// One stream, from opening it to the end of the recording or a failure.
    fn epoch(&mut self, reason: &mut Option<&'static str>) -> Result<End, Fail> {
        let starting = reason.is_none();
        self.shared.set_state(if starting {
            LabelState::Starting
        } else {
            LabelState::Restarting
        });
        let mut stream = self.diarizer.open()?;
        let geometry = stream.geometry();
        let pid = stream.worker_pid();
        // Opening loads the model and can outlast the stop of a replaced follower.
        if self.shared.stopping() || !self.poll()? {
            return Ok(End::Stopped);
        }
        let reader = self.reader();
        let (settled_ms, stored, reel, number) = {
            let Some(_writing) = self.shared.writing() else {
                return Ok(End::Stopped);
            };
            let epoch_reason = match reason {
                Some(r) => *r,
                None if self.transcript.next_epoch()? == 1 => "start",
                None => "restart",
            };
            // The stream before this one, in this or an earlier run, ends at the settled
            // position.
            let settled_ms = self.transcript.meta_u64(SETTLED_KEY)?.unwrap_or(0);
            self.transcript.close_open_turns(settled_ms)?;
            let stored = self.transcript.turns(Track::Call, 0, u64::MAX)?;
            let reel = if stored.is_empty() && settled_ms == 0 {
                Vec::new()
            } else {
                build_reel(&stored, settled_ms, &reader)
            };
            let number =
                self.transcript
                    .open_epoch(Track::Call, "nemotron", settled_ms, epoch_reason)?;
            (settled_ms, stored, reel, number)
        };
        *reason = Some("crash");
        let next = self.transcript.highest_speaker_number()? + 1;
        let mut labeller = Labeller::new(number, settled_ms, next, stored, &reel, geometry);
        self.shared.update(|s| {
            s.epoch = number;
            s.worker_pid = pid;
            s.speakers = labeller.known.len() as u32;
        });
        self.shared.set_settled(settled_ms);
        self.shared.set_state(if labeller.matched {
            LabelState::Listening
        } else {
            LabelState::Matching
        });

        for (a, b) in reel {
            let mut at = a;
            while at < b {
                if self.shared.stopping() {
                    return Ok(End::Stopped);
                }
                let len = (b - at).min(self.timing.push_max.max(1));
                let pcm = reader.read(at, at + len)?;
                labeller.clock.append(at, len);
                self.push(stream.as_mut(), &mut labeller, &pcm)?;
                // The settled position can't move while the reel plays. Each push that returns
                // is progress, so a slow re-feed doesn't release the gate; a call that hangs
                // still does.
                self.shared.touch();
                at += len;
            }
        }

        let mut fed = settled_ms * PER_MS;
        loop {
            if self.shared.stopping() || !self.poll()? {
                return Ok(End::Stopped);
            }
            let reader = self.reader();
            let mapper = self.watcher.mapper();
            let stopped = mapper.stop().is_some();
            if !stopped && dictating(mapper, self.timing.dictation_yield_ms) {
                // Dictation comes first; the recorded audio waits on disk meanwhile.
                self.shared.touch();
                self.shared.sleep(Duration::from_millis(50));
                continue;
            }
            let paused = mapper.state() == Some(MeetingState::Paused);
            let open = has_open_segment(mapper, Track::Call);
            let horizon = track_horizon(mapper, Track::Call, &reader, stopped);
            if reader.covered(fed) {
                let span_end = reader.span_end(fed, 0).unwrap_or(fed);
                let len = (span_end - fed).min(self.timing.push_max.max(1));
                let span_closed = stopped || !open || reader.next_audio(span_end).is_some();
                if len < self.timing.push_min && !(span_closed && fed + len == span_end) {
                    self.shared.sleep(self.timing.poll);
                    continue;
                }
                let pcm = reader.read(fed, fed + len)?;
                labeller.clock.append(fed, len);
                self.push(stream.as_mut(), &mut labeller, &pcm)?;
                fed += len;
                continue;
            }
            // `fed` is at a gap or at the end of what was recorded so far.
            match reader.next_audio(fed) {
                Some(next) => {
                    if next - fed >= DRIFT_GAP && labeller.clock.last_is_real() {
                        self.pad(stream.as_mut(), &mut labeller)?;
                    }
                    fed = next;
                }
                None if stopped && fed >= reader.end() => {
                    self.shared.set_state(LabelState::CatchingUp);
                    let probs = self.timed("finish", 0, || stream.finish())?;
                    let mut batch = labeller.take(probs)?;
                    labeller.close(&mut batch);
                    // With the last turns, so a pipeline after this one knows the labels are
                    // done and opens no worker for them.
                    batch.finished = true;
                    self.record(batch, &labeller)?;
                    return Ok(End::Finished);
                }
                None => {
                    let settled_gap = (paused && !open) || fed + DRIFT_GAP <= horizon;
                    if settled_gap && labeller.clock.last_is_real() {
                        self.pad(stream.as_mut(), &mut labeller)?;
                    } else {
                        // Everything recorded was pushed and padded: the labels wait for audio,
                        // not for the worker, however long a pause, a lost device or a break
                        // between the recorder's writes lasts. Without this the gate let the
                        // windows after a long pause, and the last ones before the stop, go
                        // before the resumed audio had turns. What is left as a stall is a
                        // worker call that doesn't return.
                        self.shared.touch();
                        self.shared.sleep(self.timing.poll);
                    }
                }
            }
        }
    }
}

/// A turn the stream reported a start for and no end yet.
struct OpenTurn {
    /// Stream samples.
    start: u64,
    /// The stored row: speaker and meeting ms start. `None` while it started inside the reel.
    row: Option<(String, u64)>,
}

/// Turns one stream's probabilities into stored turns with stable ids.
struct Labeller {
    epoch: u64,
    /// Meeting ms where this stream's stored turns start: the settled position it began at.
    feed_from: u64,
    /// Stream samples of the reel; turns inside it aren't stored.
    reel_end: u64,
    /// Whether the stream's labels have their ids. True at once without a reel.
    matched: bool,
    clock: StreamClock,
    binarizer: LiveBinarizer,
    speakers: usize,
    /// Frames taken so far.
    frames: u64,
    /// The stored turns the reel is matched against.
    stored: Vec<Turn>,
    /// Events held until the labels have ids.
    pending: Vec<TurnEvent>,
    /// Each label's speech inside the reel, stream samples.
    reel_turns: HashMap<usize, Vec<(u64, u64)>>,
    reel_open: HashMap<usize, u64>,
    map: HashMap<usize, String>,
    next: u64,
    open: HashMap<usize, OpenTurn>,
    settled_ms: u64,
    /// The ids the meeting has, for the status.
    known: HashSet<String>,
}

fn stream_samples(seconds: f64) -> u64 {
    (seconds.max(0.0) * 16_000.0).round() as u64
}

impl Labeller {
    fn new(
        epoch: u64,
        feed_from: u64,
        next: u64,
        stored: Vec<Turn>,
        reel: &[(u64, u64)],
        geometry: voice_core::speakers::StreamGeometry,
    ) -> Labeller {
        let reel_end: u64 = reel.iter().map(|(a, b)| b - a).sum();
        let known = stored.iter().map(|t| t.speaker.clone()).collect();
        Labeller {
            epoch,
            feed_from,
            reel_end,
            matched: reel_end == 0,
            clock: StreamClock::new(),
            binarizer: LiveBinarizer::new(
                geometry.speakers,
                geometry.seconds_per_frame,
                BinarizerParams::default(),
            ),
            speakers: geometry.speakers,
            frames: 0,
            stored,
            pending: Vec::new(),
            reel_turns: HashMap::new(),
            reel_open: HashMap::new(),
            map: HashMap::new(),
            next,
            open: HashMap::new(),
            settled_ms: feed_from,
            known,
        }
    }

    /// Feeds newly committed frames and returns what to store.
    fn take(&mut self, probs: Probs) -> Result<LabelBatch, DiarError> {
        let mut batch = LabelBatch::default();
        if probs.frames == 0 {
            self.settle(&mut batch);
            return Ok(batch);
        }
        if probs.speakers != self.speakers || probs.values.len() != probs.frames * probs.speakers {
            return Err(DiarError::new(
                DiarErrorKind::Failed,
                "The speaker label worker sent probabilities of the wrong shape.",
            ));
        }
        if probs.first_frame > self.frames {
            return Err(DiarError::new(
                DiarErrorKind::Behind,
                "Speaker labels fell behind the runtime's retained frames.",
            ));
        }
        // Frames taken already are skipped: the runtime never revises a committed frame.
        let skip = ((self.frames - probs.first_frame) as usize).min(probs.frames);
        let fresh = &probs.values[skip * self.speakers..];
        self.frames += (probs.frames - skip) as u64;
        for event in self.binarizer.feed(fresh) {
            self.event(event, &mut batch);
        }
        self.settle(&mut batch);
        Ok(batch)
    }

    /// The stream finished: closes every turn, matches if the reel never settled, and settles
    /// everything that was pushed.
    fn close(&mut self, batch: &mut LabelBatch) {
        for event in self.binarizer.close() {
            self.event(event, batch);
        }
        if !self.matched {
            self.do_match(batch);
        }
        if let Some(end) = self.clock.at(self.clock.sent(), true) {
            let ms = end.div_ceil(PER_MS);
            if ms > self.settled_ms {
                self.settled_ms = ms;
                batch.settled = Some(ms);
            }
        }
    }

    /// The stream position of a turn event. The runtime's last frames at `finish` reach past
    /// the audio pushed, by up to a frame, so a voice still talking at Stop ends past it; such an
    /// event belongs at the end of what was sent, where the stream clock still maps it.
    fn position(&self, event: &TurnEvent) -> u64 {
        stream_samples(event.seconds).min(self.clock.sent())
    }

    fn event(&mut self, event: TurnEvent, batch: &mut LabelBatch) {
        if self.matched {
            self.apply(event, batch);
            return;
        }
        let at = self.position(&event);
        if event.starting {
            if at < self.reel_end {
                self.reel_open.insert(event.speaker, at);
            }
        } else if let Some(start) = self.reel_open.remove(&event.speaker) {
            self.reel_turns
                .entry(event.speaker)
                .or_default()
                .push((start, at.min(self.reel_end)));
        }
        self.pending.push(event);
    }

    fn settle(&mut self, batch: &mut LabelBatch) {
        let settled = stream_samples(self.binarizer.settled_seconds()).min(self.clock.sent());
        if !self.matched && settled >= self.reel_end {
            self.do_match(batch);
        }
        if !self.matched {
            return;
        }
        if let Some(meeting) = self.clock.at(settled, true) {
            let ms = meeting / PER_MS;
            if ms > self.settled_ms {
                self.settled_ms = ms;
                batch.settled = Some(ms);
            }
        }
    }

    /// Gives the reel's labels the stored ids they overlap, then stores the held events.
    fn do_match(&mut self, batch: &mut LabelBatch) {
        let mut reel: HashMap<usize, Vec<(u64, u64)>> = HashMap::new();
        let open: Vec<(usize, u64)> = self.reel_open.iter().map(|(&l, &s)| (l, s)).collect();
        let ranges = self
            .reel_turns
            .iter()
            .flat_map(|(&label, list)| list.iter().map(move |&r| (label, r)))
            .chain(
                open.into_iter()
                    .map(|(label, start)| (label, (start, self.reel_end))),
            );
        for (label, (a, b)) in ranges {
            reel.entry(label)
                .or_default()
                .extend(self.clock.split(a, b));
        }
        let mut matched: Vec<(usize, String)> =
            match_labels(&reel, &self.stored).into_iter().collect();
        matched.sort();
        for (label, id) in matched {
            batch.map.push(MapRow {
                epoch: self.epoch,
                label: label.to_string(),
                speaker: id.clone(),
                how: "matched".into(),
            });
            self.map.insert(label, id);
        }
        self.matched = true;
        for event in std::mem::take(&mut self.pending) {
            self.apply(event, batch);
        }
        // Turns that began in the reel and go on are stored from where the live audio starts.
        let mut started: Vec<usize> = self
            .open
            .iter()
            .filter(|(_, o)| o.row.is_none())
            .map(|(&l, _)| l)
            .collect();
        started.sort();
        for label in started {
            let id = self.id_for(label, batch);
            batch.turns.push(TurnRow {
                speaker: id.clone(),
                start: self.feed_from,
                end: None,
                epoch: self.epoch,
            });
            if let Some(open) = self.open.get_mut(&label) {
                open.row = Some((id, self.feed_from));
            }
        }
    }

    fn id_for(&mut self, label: usize, batch: &mut LabelBatch) -> String {
        if let Some(id) = self.map.get(&label) {
            return id.clone();
        }
        let id = format!("call-{}", self.next);
        self.next += 1;
        batch.map.push(MapRow {
            epoch: self.epoch,
            label: label.to_string(),
            speaker: id.clone(),
            how: "new".into(),
        });
        self.map.insert(label, id.clone());
        self.known.insert(id.clone());
        id
    }

    fn meeting_ms(&self, at: u64, ending: bool) -> u64 {
        let meeting = self
            .clock
            .at(at, ending)
            .or_else(|| self.clock.at(at, !ending))
            .unwrap_or(self.feed_from * PER_MS);
        if ending {
            meeting.div_ceil(PER_MS)
        } else {
            meeting / PER_MS
        }
    }

    fn apply(&mut self, event: TurnEvent, batch: &mut LabelBatch) {
        let label = event.speaker;
        let at = self.position(&event);
        if event.starting {
            if at < self.reel_end {
                self.open.insert(
                    label,
                    OpenTurn {
                        start: at,
                        row: None,
                    },
                );
                return;
            }
            let start = self.meeting_ms(at, false).max(self.feed_from);
            let id = self.id_for(label, batch);
            batch.turns.push(TurnRow {
                speaker: id.clone(),
                start,
                end: None,
                epoch: self.epoch,
            });
            self.open.insert(
                label,
                OpenTurn {
                    start: at,
                    row: Some((id, start)),
                },
            );
            return;
        }
        let Some(open) = self.open.remove(&label) else {
            return;
        };
        let end = self.meeting_ms(at, true);
        match open.row {
            Some((id, start)) => batch.turns.push(TurnRow {
                speaker: id,
                start,
                end: Some(end.max(start)),
                epoch: self.epoch,
            }),
            None if at > self.reel_end && open.start < self.reel_end => {
                let id = self.id_for(label, batch);
                batch.turns.push(TurnRow {
                    speaker: id,
                    start: self.feed_from,
                    end: Some(end.max(self.feed_from)),
                    epoch: self.epoch,
                });
            }
            // Wholly inside the reel: only for matching.
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::speakers::StreamGeometry;

    fn geometry() -> StreamGeometry {
        StreamGeometry {
            speakers: 2,
            seconds_per_frame: 0.01,
        }
    }

    /// Frames of two speakers: `on[s]` lists (first frame, last frame) per speaker.
    fn probs(first: u64, frames: usize, on: &[&[(usize, usize)]]) -> Probs {
        let mut values = vec![0.0f32; frames * 2];
        for (s, runs) in on.iter().enumerate() {
            for &(a, b) in runs.iter() {
                for f in a..b.min(frames) {
                    values[f * 2 + s] = 0.95;
                }
            }
        }
        Probs {
            first_frame: first,
            frames,
            speakers: 2,
            values,
        }
    }

    #[test]
    fn without_a_reel_new_labels_get_new_ids_at_their_first_turn() {
        let mut l = Labeller::new(1, 0, 1, Vec::new(), &[], geometry());
        assert!(l.matched);
        // 3 s of stream from meeting 10 s on; speaker 1 talks 0.5-2 s.
        l.clock.append(160_000, 48_000);
        let batch = l.take(probs(0, 300, &[&[], &[(50, 200)]])).unwrap();
        assert_eq!(
            batch.map,
            vec![MapRow {
                epoch: 1,
                label: "1".into(),
                speaker: "call-1".into(),
                how: "new".into()
            }]
        );
        // The start (0.5 s less 0.229 s padding) and the end (2 s plus 0.079 s).
        assert_eq!(
            batch.turns,
            vec![
                TurnRow {
                    speaker: "call-1".into(),
                    start: 10_271,
                    end: None,
                    epoch: 1
                },
                TurnRow {
                    speaker: "call-1".into(),
                    start: 10_271,
                    end: Some(12_079),
                    epoch: 1
                },
            ]
        );
        assert_eq!(batch.settled, Some(12_475));
    }

    /// The runtime commits whole chunks once their right context came in; after the padding,
    /// the binarizer's settled position must reach the end of the audio before the gap,
    /// wherever in a chunk that audio ends.
    #[test]
    fn the_padding_settles_audio_that_ends_anywhere_in_a_chunk() {
        let p = BinarizerParams::default();
        let lag = p.min_on.max(p.pad_onset + p.min_off);
        assert!(SETTLE_FRAMES as f64 * 0.08 >= lag, "{lag}");
        let chunk = CHUNK_FRAMES as u64 * MODEL_FRAME_SAMPLES;
        let right = RIGHT_CONTEXT_FRAMES as u64 * MODEL_FRAME_SAMPLES;
        for end in (0..3 * chunk).step_by(160) {
            let pushed = end + PAD;
            let committed = pushed.saturating_sub(right) / chunk * chunk;
            let settled = committed as f64 / 16_000.0 - lag;
            assert!(
                settled >= end as f64 / 16_000.0,
                "audio to {end} settles at {settled} s"
            );
        }
        // The 3.12 s it was before leaves audio that ends 6.0 s into a stream short, as the
        // worker showed.
        let committed = (96_000 + 49_920 - right) / chunk * chunk;
        assert!((committed as f64 / 16_000.0 - lag) < 6.0);
    }

    #[test]
    fn the_call_log_notes_slow_calls_the_run_up_to_a_failure_and_each_minute() {
        let start = Instant::now();
        let mut log = CallLog::new(start);
        let ms = Duration::from_millis;
        // Quick calls log nothing until the minute is up.
        for i in 0..3 {
            let at = start + Duration::from_secs(i);
            assert!(log.lines("push", 16_000, ms(100), None, at).is_empty());
        }
        assert_eq!(
            log.lines(
                "push",
                16_000,
                ms(4_200),
                None,
                start + Duration::from_secs(10)
            ),
            ["speaker worker push of 16000 samples took 4200 ms"]
        );
        let timeout = DiarError::new(DiarErrorKind::Timeout, "no answer");
        assert_eq!(
            log.lines("push", 8_000, ms(30_000), Some(&timeout), start + Duration::from_secs(45)),
            ["speaker worker push of 8000 samples failed after 30000 ms (Timeout); the 4 calls before it took 100, 100, 100, 4200 ms"]
        );
        assert_eq!(
            log.lines("finish", 0, ms(300), None, start + Duration::from_secs(61)),
            ["speaker worker in the last 61 s: 6 calls, 4.5 s of audio in 34.8 s, median 300 ms, slowest 30000 ms"]
        );
        // A new minute starts empty; the run-up keeps the last 20 calls.
        for i in 0..25 {
            log.lines("push", 1_600, ms(i), None, start + Duration::from_secs(62));
        }
        let line = &log.lines(
            "push",
            1_600,
            ms(1),
            Some(&timeout),
            start + Duration::from_secs(63),
        )[0];
        assert!(line.contains("the 20 calls before it took 5, 6,"), "{line}");
    }

    #[test]
    fn frames_already_taken_are_skipped_and_a_gap_is_behind() {
        let mut l = Labeller::new(1, 0, 1, Vec::new(), &[], geometry());
        l.clock.append(0, 16_000);
        l.take(probs(0, 50, &[&[], &[]])).unwrap();
        // The same 50 frames and 10 new ones.
        l.take(probs(0, 60, &[&[], &[]])).unwrap();
        assert_eq!(l.frames, 60);
        let behind = l.take(probs(70, 5, &[&[], &[]])).unwrap_err();
        assert_eq!(behind.kind, DiarErrorKind::Behind);
        let shape = l
            .take(Probs {
                first_frame: 60,
                frames: 2,
                speakers: 3,
                values: vec![0.0; 6],
            })
            .unwrap_err();
        assert_eq!(shape.kind, DiarErrorKind::Failed);
    }

    #[test]
    fn the_reel_matches_labels_and_stores_only_what_follows_it() {
        // call-4 spoke at 10-12 s; the reel feeds 10-12 s, then the live audio from 30 s on.
        let stored = vec![Turn {
            speaker: "call-4".into(),
            start: 10_000,
            end: 12_000,
        }];
        let reel = [(160_000, 192_000)];
        let mut l = Labeller::new(2, 30_000, 5, stored, &reel, geometry());
        assert!(!l.matched);
        l.clock.append(160_000, 32_000);
        l.clock.append(480_000, 32_000);
        // This stream calls call-4 "0", which talks through the reel into the live audio until
        // 3 s of stream (31 s of meeting), and a new voice "1" at 3.5-4 s.
        let batch = l
            .take(probs(0, 400, &[&[(0, 300)], &[(350, 400)]]))
            .unwrap();
        assert!(l.matched);
        assert_eq!(batch.map[0].speaker, "call-4");
        assert_eq!(batch.map[0].how, "matched");
        // The turn that ran past the reel is stored from 30 s.
        assert!(batch.turns.contains(&TurnRow {
            speaker: "call-4".into(),
            start: 30_000,
            end: Some(31_079),
            epoch: 2
        }));
        l.clock.append(512_000, 16_000);
        let batch = l.take(probs(400, 100, &[&[], &[(0, 30)]])).unwrap();
        let mut new = l.map.values().cloned().collect::<Vec<_>>();
        new.sort();
        assert_eq!(new, vec!["call-4", "call-5"]);
        assert!(batch.turns.iter().all(|t| t.start >= 30_000));
    }
}
