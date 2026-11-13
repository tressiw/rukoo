//! The JPEG and OCR threads. The main thread queues the frames it decided to keep. The JPEG
//! thread writes each one (`.jpg.tmp`, synced, renamed) and hands it to the OCR thread, which
//! reads its text and hands the `frame` line back for the journal. OCR can fall behind, or hang
//! in the platform's recognition call, without slowing capture and without holding more than a
//! few frames' pixels: with `QUEUE` frames waiting for OCR, the oldest is journaled with its JPEG and
//! `ocr: "skipped"` (spec 2.5), and with `JPEG_QUEUE` frames waiting for their JPEG the main
//! thread keeps its next frame for later (`accepts`). So at most `HELD` frames' pixels are
//! held, whatever OCR does.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use voice_screen_files::frames::clean_text;
use voice_screen_files::journal::Line;
use voice_screen_files::paths;

use crate::platform::{FrameIo, Jpeg, Pixels, Reader};
use crate::record::Ev;

/// Frames with a JPEG waiting for OCR before the oldest skips it.
pub const QUEUE: usize = 8;
/// Frames waiting for their JPEG before the main thread keeps its next one for later.
pub const JPEG_QUEUE: usize = 2;
/// The most frames whose pixels the threads hold: both queues and one in each thread's hands.
pub const HELD: usize = JPEG_QUEUE + QUEUE + 2;

pub struct FrameJob {
    pub id: String,
    pub t: u64,
    pub pixels: Pixels,
}

/// What became of a job.
#[derive(Debug)]
pub enum FrameDone {
    /// The JPEG is on disk; the `frame` line goes into the journal.
    Saved(Line),
    /// The JPEG couldn't be written. `disk_full` drops the frames (spec 2.6).
    Failed {
        id: String,
        t: u64,
        disk_full: bool,
        error: String,
    },
}

/// A frame whose JPEG is on disk, before its text is read.
#[derive(Clone, Debug)]
struct Written {
    id: String,
    t: u64,
    file: String,
    width: u32,
    height: u32,
    bytes: usize,
    jpeg_ms: u128,
}

impl Written {
    fn line(self, text: String, status: &str, ocr_ms: u128, language: Option<String>) -> Line {
        // By id, size and timing only: never the text (section 7 reads the timings from here).
        crate::log(format_args!(
            "frame {}: {}x{}, {} bytes, jpeg {} ms, ocr {ocr_ms} ms ({status})",
            self.id, self.width, self.height, self.bytes, self.jpeg_ms
        ));
        Line::Frame {
            t: self.t,
            id: self.id,
            file: self.file,
            width: self.width,
            height: self.height,
            text,
            ocr: status.into(),
            language,
        }
    }

    fn skipped(self, language: Option<String>) -> FrameDone {
        FrameDone::Saved(self.line(String::new(), "skipped", 0, language))
    }
}

struct OcrJob {
    written: Written,
    pixels: Pixels,
}

#[derive(Default)]
struct State {
    /// Waiting for their JPEG.
    jpeg: VecDeque<FrameJob>,
    jpeg_busy: bool,
    /// The JPEG thread ended: nothing more reaches the OCR queue.
    jpeg_ended: bool,
    /// JPEG on disk, waiting for their text.
    ocr: VecDeque<OcrJob>,
    /// The frame the OCR thread reads, and whether the thread is handing its line back already.
    /// `skip_ocr` takes one that isn't, so a hung OCR call doesn't cost a frame its line.
    ocr_hand: Option<(Written, bool)>,
    closing: bool,
    skip_ocr: bool,
    /// The OCR language, once the OCR thread has its engine; skipped frames carry it too.
    language: Option<String>,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.ready.wait(state).unwrap_or_else(|e| e.into_inner())
    }
}

pub struct FrameWorker {
    shared: Arc<Shared>,
    meeting: PathBuf,
    handles: Vec<JoinHandle<()>>,
    /// Closes when both threads ended, so `join` can wait with a limit.
    finished: Option<mpsc::Receiver<()>>,
}

impl FrameWorker {
    /// Starts the threads. `write_fail_at` is the `write-fail=frames@<ms>` test switch: from that
    /// meeting time every JPEG write fails as a full disk; `io` is the platform's encoding, disk
    /// and text reading.
    pub fn start<F: Clone + Send + 'static>(
        meeting: PathBuf,
        to_main: SyncSender<Ev<F>>,
        write_fail_at: Option<u64>,
        io: FrameIo,
    ) -> FrameWorker {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
        });
        let (finished_tx, finished) = mpsc::channel::<()>();
        let mut handles = Vec::new();

        let FrameIo {
            setup,
            jpeg,
            disk,
            reader,
        } = io;
        let ocr_setup = setup.clone();
        let (jpeg_shared, jpeg_main, jpeg_finished, jpeg_meeting) = (
            shared.clone(),
            to_main.clone(),
            finished_tx.clone(),
            meeting.clone(),
        );
        let jpeg = std::thread::Builder::new()
            .name("frames-jpeg".into())
            .spawn(move || {
                // Dropped when the thread ends, also by a panic: `join` stops waiting, and the OCR
                // thread knows nothing more comes.
                let _finished = jpeg_finished;
                let _ended = Ended(&jpeg_shared);
                setup.init_thread();
                run_jpeg(
                    &jpeg_meeting,
                    &jpeg_shared,
                    &jpeg_main,
                    write_fail_at,
                    jpeg.as_ref(),
                    disk.as_ref(),
                );
            });
        match jpeg {
            Ok(handle) => handles.push(handle),
            Err(_) => shared.state().jpeg_ended = true,
        }

        let ocr_shared = shared.clone();
        let ocr = std::thread::Builder::new()
            .name("frames-ocr".into())
            .spawn(move || {
                let _finished = finished_tx;
                ocr_setup.init_thread();
                let reader = reader();
                run_ocr(&ocr_shared, &to_main, reader.as_ref());
            });
        // Without an OCR thread the queue overflows into skipped frames, which keep their JPEG.
        if let Ok(handle) = ocr {
            handles.push(handle);
        }
        FrameWorker {
            shared,
            meeting,
            handles,
            finished: Some(finished),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.shared.state()
    }

    /// Whether `send` takes a frame now. The main thread asks before it reads a frame back from
    /// the graphics card; when the JPEG thread is behind it keeps the frame for a later offer.
    pub fn accepts(&self) -> bool {
        self.state().jpeg.len() < JPEG_QUEUE
    }

    /// Queues a frame for its JPEG, unless `JPEG_QUEUE` wait already. Returns whether it did.
    pub fn send(&self, job: FrameJob) -> bool {
        let mut state = self.state();
        if state.jpeg.len() >= JPEG_QUEUE {
            return false;
        }
        state.jpeg.push_back(job);
        self.shared.ready.notify_all();
        true
    }

    /// Jobs queued or in hand: the frames whose pixels the threads hold, at most `HELD`.
    pub fn pending(&self) -> usize {
        let state = self.state();
        state.jpeg.len()
            + usize::from(state.jpeg_busy)
            + state.ocr.len()
            + usize::from(state.ocr_hand.is_some())
    }

    /// Forgets queued frames from `t` on, as after a pause or a stop. A forgotten frame whose
    /// JPEG is written loses the file too.
    pub fn cancel_from(&self, t: u64) {
        let mut state = self.state();
        state.jpeg.retain(|job| job.t < t);
        let (keep, gone): (VecDeque<OcrJob>, VecDeque<OcrJob>) =
            state.ocr.drain(..).partition(|job| job.written.t < t);
        state.ocr = keep;
        drop(state);
        self.remove(gone);
    }

    /// Forgets every queued frame, as when frames are dropped, with the JPEGs already written.
    pub fn clear(&self) {
        let mut state = self.state();
        state.jpeg.clear();
        let gone: Vec<OcrJob> = state.ocr.drain(..).collect();
        drop(state);
        self.remove(gone);
    }

    fn remove(&self, jobs: impl IntoIterator<Item = OcrJob>) {
        for job in jobs {
            let _ = fs::remove_file(self.meeting.join(&job.written.file));
        }
    }

    /// From now on frames keep their JPEG and skip OCR, as when the stop drain runs late. The
    /// frames waiting for OCR, and the one in OCR unless its line is on its way, come back at
    /// once as skipped, for the caller to journal.
    pub fn skip_ocr(&self) -> Vec<FrameDone> {
        let mut state = self.state();
        state.skip_ocr = true;
        let language = state.language.clone();
        let mut done: Vec<FrameDone> = Vec::new();
        if matches!(state.ocr_hand, Some((_, false))) {
            if let Some((written, _)) = state.ocr_hand.take() {
                done.push(written.skipped(language.clone()));
            }
        }
        let waiting: Vec<OcrJob> = state.ocr.drain(..).collect();
        self.shared.ready.notify_all();
        drop(state);
        done.extend(
            waiting
                .into_iter()
                .map(|job| job.written.skipped(language.clone())),
        );
        done
    }

    /// Lets the threads end once their queues are empty.
    pub fn close(&mut self) {
        self.state().closing = true;
        self.shared.ready.notify_all();
        self.handles.clear();
    }

    /// Lets the threads end and waits for them, so the OCR engine and bitmaps are released on
    /// their own threads before the process exits. A thread still busy after `timeout`, such as
    /// one stuck in OCR, is left behind.
    pub fn join(&mut self, timeout: Duration) {
        self.state().closing = true;
        self.shared.ready.notify_all();
        if let Some(finished) = self.finished.take() {
            if finished.recv_timeout(timeout) == Err(mpsc::RecvTimeoutError::Timeout) {
                crate::log("a frame thread didn't end in time; leaving it");
                self.handles.clear();
                return;
            }
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for FrameWorker {
    fn drop(&mut self) {
        self.close();
    }
}

/// Marks the JPEG thread ended when it ends, also by a panic.
struct Ended<'a>(&'a Shared);

impl Drop for Ended<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state();
        state.jpeg_ended = true;
        state.jpeg_busy = false;
        self.0.ready.notify_all();
    }
}

/// Hands an event to the main thread; false once it is gone.
fn emit<F: Send>(to_main: &SyncSender<Ev<F>>, done: FrameDone) -> bool {
    let mut event = Ev::FrameDone(done);
    loop {
        match to_main.try_send(event) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            Err(mpsc::TrySendError::Full(back)) => {
                event = back;
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn run_jpeg<F: Send>(
    meeting: &Path,
    shared: &Shared,
    to_main: &SyncSender<Ev<F>>,
    write_fail_at: Option<u64>,
    jpeg: &dyn Jpeg,
    disk: &dyn crate::platform::Disk,
) {
    loop {
        let job = {
            let mut state = shared.state();
            // The last job counted as busy until its line or its OCR job was out, so the stop
            // drain never sees nothing pending while a frame is on its way.
            state.jpeg_busy = false;
            loop {
                if let Some(job) = state.jpeg.pop_front() {
                    state.jpeg_busy = true;
                    break job;
                }
                if state.closing {
                    return;
                }
                state = shared.wait(state);
            }
        };
        let FrameJob { id, t, pixels } = job;
        let written = match write_jpeg(meeting, &id, t, &pixels, write_fail_at, jpeg, disk) {
            Ok(written) => written,
            Err(failed) => {
                if !emit(to_main, *failed) {
                    return;
                }
                continue;
            }
        };
        let mut state = shared.state();
        if state.skip_ocr {
            let language = state.language.clone();
            drop(state);
            if !emit(to_main, written.skipped(language)) {
                return;
            }
            continue;
        }
        // A full OCR queue: its oldest frame goes into the journal without text.
        let overflow = if state.ocr.len() >= QUEUE {
            state.ocr.pop_front()
        } else {
            None
        };
        state.ocr.push_back(OcrJob { written, pixels });
        let language = state.language.clone();
        shared.ready.notify_all();
        drop(state);
        if let Some(old) = overflow {
            if !emit(to_main, old.written.skipped(language)) {
                return;
            }
        }
    }
}

fn run_ocr<F: Send>(shared: &Shared, to_main: &SyncSender<Ev<F>>, reader: &dyn Reader) {
    let language = reader.language();
    shared.state().language = language.clone();
    loop {
        let OcrJob { written, pixels } = {
            let mut state = shared.state();
            loop {
                if let Some(job) = state.ocr.pop_front() {
                    state.ocr_hand = Some((job.written.clone(), false));
                    break job;
                }
                if state.jpeg_ended {
                    return;
                }
                state = shared.wait(state);
            }
        };
        let started = Instant::now();
        let (text, status) = if !reader.available() {
            (String::new(), "none")
        } else {
            match reader.read_pixels(&pixels) {
                Ok(lines) => (clean_text(&lines), "ok"),
                Err(_) => (String::new(), "failed"),
            }
        };
        drop(pixels);
        let ocr_ms = started.elapsed().as_millis();
        // Still ours, unless `skip_ocr` journaled it as skipped meanwhile.
        let mine = {
            let mut state = shared.state();
            match &mut state.ocr_hand {
                Some((hand, emitting)) if hand.id == written.id => {
                    *emitting = true;
                    true
                }
                _ => false,
            }
        };
        if mine {
            let sent = emit(
                to_main,
                FrameDone::Saved(written.line(text, status, ocr_ms, language.clone())),
            );
            shared.state().ocr_hand = None;
            if !sent {
                return;
            }
        }
    }
}

/// Encodes and writes the JPEG of a frame. The error is the job's `FrameDone::Failed`.
fn write_jpeg(
    meeting: &Path,
    id: &str,
    t: u64,
    pixels: &Pixels,
    write_fail_at: Option<u64>,
    jpeg: &dyn Jpeg,
    disk: &dyn crate::platform::Disk,
) -> Result<Written, Box<FrameDone>> {
    let failed = |disk_full: bool, error: String| {
        Box::new(FrameDone::Failed {
            id: id.to_string(),
            t,
            disk_full,
            error,
        })
    };
    let file = paths::frame_file(id);
    let path = meeting.join(&file);
    let started = Instant::now();
    let (bytes, width, height) = match jpeg.encode(pixels) {
        Ok(encoded) => encoded,
        Err(e) => return Err(failed(false, e)),
    };
    let jpeg_ms = started.elapsed().as_millis();
    let fault = write_fail_at.is_some_and(|at| t >= at);
    if let Err(e) = write_synced(&path, &bytes, fault) {
        return Err(failed(disk.is_disk_full(&e), e.to_string()));
    }
    Ok(Written {
        id: id.to_string(),
        t,
        file,
        width,
        height,
        bytes: bytes.len(),
        jpeg_ms,
    })
}

/// `<path>.tmp`, synced, then renamed to `path`; nothing stays on failure.
fn write_synced(path: &Path, bytes: &[u8], fault: bool) -> io::Result<()> {
    let tmp = path.with_extension("jpg.tmp");
    let result = (|| {
        if fault {
            // The platform's full-disk error: ERROR_DISK_FULL on Windows, ENOSPC on Unix.
            return Err(crate::disk::test_disk_full_error());
        }
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn temp_meeting(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voice-screen-worker-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(paths::screen_dir(&dir).join("frames")).unwrap();
        dir
    }

    #[test]
    fn a_failed_write_leaves_nothing() {
        let dir = std::env::temp_dir().join(format!("voice-screen-worker-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f-000001000.jpg");
        let e = write_synced(&path, b"jpeg", true).unwrap_err();
        assert!(crate::disk::is_disk_full(&e), "{e}");
        assert!(!path.exists() && !path.with_extension("jpg.tmp").exists());
        write_synced(&path, b"jpeg", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"jpeg");
        assert!(!path.with_extension("jpg.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// An OCR that doesn't return until the test lets it, like a `RecognizeAsync(...).join()`
    /// that hangs.
    struct Stalled {
        released: Arc<(Mutex<bool>, Condvar)>,
        calls: Arc<AtomicUsize>,
    }

    impl Reader for Stalled {
        fn available(&self) -> bool {
            true
        }

        fn language(&self) -> Option<String> {
            Some("en-US".into())
        }

        fn read_pixels(&self, _pixels: &Pixels) -> Result<Vec<String>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let (lock, cv) = &*self.released;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = cv.wait(released).unwrap();
            }
            Ok(vec!["Slide text".into()])
        }

        fn read_image(
            &self,
            _file: &[u8],
        ) -> Result<(crate::platform::ImageText, (u32, u32)), String> {
            Err("no image".into())
        }
    }

    /// The platform's parts stand-in for these tests: a JPEG that is its pixels' length, a disk
    /// that never fails, and no thread setup. No graphics API is involved, so the threads run on
    /// every platform.
    fn fake_io(reader: impl Reader + 'static) -> FrameIo {
        use crate::platform::{Disk, ImageText, ThreadInit};

        struct Setup;

        impl ThreadInit for Setup {
            fn init_thread(&self) {}
        }

        struct FakeJpeg;

        impl Jpeg for FakeJpeg {
            fn encode(&self, pixels: &Pixels) -> Result<(Vec<u8>, u32, u32), String> {
                Ok((
                    vec![0xFF, 0xD8, pixels.bgra.len() as u8],
                    pixels.width,
                    pixels.height,
                ))
            }
        }

        struct FakeDisk;

        impl Disk for FakeDisk {
            fn free_bytes(
                &self,
                _path: &std::path::Path,
                _test_file: Option<&std::path::Path>,
            ) -> Option<u64> {
                Some(1 << 40)
            }

            fn is_disk_full(&self, e: &io::Error) -> bool {
                e.raw_os_error() == Some(112)
            }
        }

        let _ = ImageText::None;
        FrameIo {
            setup: Arc::new(Setup),
            jpeg: Arc::new(FakeJpeg),
            disk: Arc::new(FakeDisk),
            reader: Box::new(move || Box::new(reader)),
        }
    }

    fn job(n: u64) -> FrameJob {
        let t = 1_000 + n * 2_000;
        FrameJob {
            id: paths::frame_id(t),
            t,
            pixels: Pixels {
                bgra: vec![(n * 7) as u8; 64 * 48 * 4],
                width: 64,
                height: 48,
            },
        }
    }

    /// Takes every `frame` line the threads sent so far.
    fn collect(rx: &mpsc::Receiver<Ev<()>>, lines: &mut BTreeMap<String, (String, String)>) {
        while let Ok(event) = rx.try_recv() {
            match event {
                Ev::FrameDone(FrameDone::Saved(Line::Frame { id, file, ocr, .. })) => {
                    assert!(
                        lines.insert(id.clone(), (file, ocr)).is_none(),
                        "{id} twice"
                    );
                }
                Ev::FrameDone(other) => panic!("{other:?}"),
                _ => panic!("not a frame event"),
            }
        }
    }

    fn wait_for(
        rx: &mpsc::Receiver<Ev<()>>,
        lines: &mut BTreeMap<String, (String, String)>,
        count: usize,
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while lines.len() < count {
            assert!(
                Instant::now() < deadline,
                "{} of {count} lines",
                lines.len()
            );
            collect(rx, lines);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// With OCR hung, the main thread keeps saving frames the way `Screen::save` does: the
    /// threads hold at most `HELD` frames' pixels, and every frame past the OCR queue gets its
    /// JPEG and a `skipped` line at once. Once OCR comes back the rest are read.
    #[test]
    fn a_hung_ocr_holds_few_frames_and_skips_the_rest() {
        let meeting = temp_meeting("hung-ocr");
        let released = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let reader = Stalled {
            released: released.clone(),
            calls: calls.clone(),
        };
        let (tx, rx) = mpsc::sync_channel::<Ev<()>>(64);
        let mut worker = FrameWorker::start(meeting.clone(), tx, None, fake_io(reader));
        let total = 40u64;
        let (mut sent, mut most) = (0u64, 0usize);
        let mut lines = BTreeMap::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        while sent < total {
            assert!(Instant::now() < deadline, "only {sent} frames taken");
            if worker.accepts() {
                assert!(worker.send(job(sent)));
                sent += 1;
            }
            most = most.max(worker.pending());
            collect(&rx, &mut lines);
            std::thread::sleep(Duration::from_millis(2));
        }
        // One frame is in OCR and QUEUE wait for it; every other one is journaled, skipped.
        let skipped = total as usize - QUEUE - 1;
        wait_for(&rx, &mut lines, skipped);
        assert!(most <= HELD, "the threads held {most} frames' pixels");
        let deadline = Instant::now() + Duration::from_secs(10);
        while worker.pending() != QUEUE + 1 {
            assert!(Instant::now() < deadline, "{} pending", worker.pending());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(lines.len(), skipped);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        for (id, (file, ocr)) in &lines {
            assert_eq!(ocr, "skipped", "{id}");
            assert!(meeting.join(file).is_file(), "{id} has no JPEG");
        }

        // OCR comes back: the frames it waited for are read.
        {
            let (lock, cv) = &*released;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
        wait_for(&rx, &mut lines, total as usize);
        let read = lines.values().filter(|(_, ocr)| ocr == "ok").count();
        assert_eq!(read, QUEUE + 1);
        let expected: Vec<String> = (0..total).map(|n| job(n).id).collect();
        assert_eq!(lines.keys().cloned().collect::<Vec<_>>(), {
            let mut ids = expected;
            ids.sort();
            ids
        });
        for (file, _) in lines.values() {
            assert!(meeting.join(file).is_file(), "{file}");
        }
        assert_eq!(worker.pending(), 0);
        worker.join(Duration::from_secs(5));
        fs::remove_dir_all(&meeting).unwrap();
    }

    /// At the stop, OCR still hung: `skip_ocr` hands back the frame in OCR and those waiting,
    /// each with its JPEG, and the late answer of the hung call adds no second line.
    #[test]
    fn skip_ocr_journals_the_frames_a_hung_ocr_holds() {
        let meeting = temp_meeting("skip-ocr");
        let released = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let reader = Stalled {
            released: released.clone(),
            calls: calls.clone(),
        };
        let (tx, rx) = mpsc::sync_channel::<Ev<()>>(64);
        let mut worker = FrameWorker::start(meeting.clone(), tx, None, fake_io(reader));
        for n in 0..3 {
            while !worker.accepts() {
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(worker.send(job(n)));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        // Frame 0 in OCR, 1 and 2 waiting for it, the JPEG thread idle.
        while {
            let state = worker.state();
            state.ocr.len() < 2 || state.jpeg_busy || calls.load(Ordering::SeqCst) == 0
        } {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut lines = BTreeMap::new();
        for done in worker.skip_ocr() {
            match done {
                FrameDone::Saved(Line::Frame { id, file, ocr, .. }) => {
                    assert_eq!(ocr, "skipped");
                    assert!(meeting.join(&file).is_file());
                    lines.insert(id, (file, ocr));
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(lines.len(), 3);
        assert_eq!(worker.pending(), 0);
        // A frame sent now keeps its JPEG and skips OCR.
        assert!(worker.send(job(3)));
        wait_for(&rx, &mut lines, 4);
        assert_eq!(lines[&job(3).id].1, "skipped");
        {
            let (lock, cv) = &*released;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
        worker.join(Duration::from_secs(5));
        collect(&rx, &mut lines);
        assert_eq!(lines.len(), 4);
        fs::remove_dir_all(&meeting).unwrap();
    }
}
