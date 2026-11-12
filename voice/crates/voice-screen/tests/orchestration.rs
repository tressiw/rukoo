//! The screen process's orchestration without a screen: a fake backend produces frames, drinks
//! its clock and writes fake video, and `record` runs the whole session — the journal, the frame
//! and drop rules, the video file and the stop — on every platform. Nothing here touches a
//! graphics API, so the split between the orchestration and a platform is covered on a Mac and on
//! Linux CI as well as on Windows.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use voice_screen::args::{Mode, RecordOptions};
use voice_screen::platform::{
    Clocks, DeviceInfo, Disk, Displays, Frame, FrameIo, ImageText, Jpeg, Pixels, Platform,
    PlatformError, Reader, Source, ThreadInit, Thumbs, VideoError, VideoMaker, VideoWriter,
    WriterOptions,
};
use voice_screen::record::{record, CaptureStop, Ev};
use voice_screen_files::clock::Now;
use voice_screen_files::frames::Thumb;
use voice_screen_files::monitors::{MonitorInfo, WindowInfo};
use voice_screen_files::paths;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const MONITOR: &str = r"\\.\FAKE1";
/// The fake clock's boot time at the start, in 100 ns units.
const BOOT0: u64 = 1_000_000;
/// The wall clock at `BOOT0`.
const WALL0: u64 = 1_790_000_000_000;

/// A clock that only moves when the fake source takes a frame, so a test never waits on it.
struct FakeClock {
    boot: AtomicU64,
}

impl FakeClock {
    fn new() -> FakeClock {
        FakeClock {
            boot: AtomicU64::new(BOOT0),
        }
    }

    fn advance(&self, units: u64) -> u64 {
        self.boot.fetch_add(units, Ordering::SeqCst) + units
    }
}

impl Clocks for FakeClock {
    fn boot_100ns(&self) -> u64 {
        self.boot.load(Ordering::SeqCst)
    }

    fn awake_100ns(&self) -> u64 {
        self.boot.load(Ordering::SeqCst)
    }

    fn now(&self) -> Now {
        self.at_boot(self.boot_100ns())
    }

    fn at_boot(&self, boot: u64) -> Now {
        Now {
            boot: Some(boot),
            wall: WALL0 + boot.saturating_sub(BOOT0) / 10_000,
        }
    }
}

#[derive(Clone)]
struct FakeDisplays {
    /// Whether this process may capture the screen: false is a revoked permission.
    allowed: Arc<AtomicBool>,
    /// How many exclusions `z_order` was given, so a test can tell they reach it.
    windows: Arc<AtomicU32>,
    /// The monitor's size; a test changes it as a display's mode changes.
    size: Arc<Mutex<(u32, u32)>>,
}

impl Displays for FakeDisplays {
    fn list(&self, _test: Option<&voice_screen::args::TestSpec>) -> Vec<MonitorInfo> {
        let (width, height) = *self.size.lock().unwrap();
        vec![MonitorInfo {
            id: MONITOR.into(),
            name: "Fake".into(),
            x: 0,
            y: 0,
            width,
            height,
            primary: true,
        }]
    }

    fn z_order(&self, exclude_pids: &[u32]) -> Vec<WindowInfo> {
        self.windows
            .store(exclude_pids.len() as u32, Ordering::SeqCst);
        Vec::new()
    }

    fn cursor_monitor(&self) -> Option<String> {
        Some(MONITOR.into())
    }

    fn can_capture(&self) -> bool {
        true
    }

    fn missing_permission(&self) -> Option<String> {
        (!self.allowed.load(Ordering::SeqCst))
            .then(|| "Screen Recording permission is off in System Settings".into())
    }
}

struct PlentyDisk;

impl Disk for PlentyDisk {
    fn free_bytes(&self, _path: &Path, test_file: Option<&Path>) -> Option<u64> {
        match test_file {
            Some(file) => fs::read_to_string(file).ok()?.trim().parse().ok(),
            None => Some(1 << 40),
        }
    }

    fn is_disk_full(&self, _e: &std::io::Error) -> bool {
        false
    }
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

/// A computer without text recognition.
struct NoText;

impl Reader for NoText {
    fn available(&self) -> bool {
        false
    }

    fn language(&self) -> Option<String> {
        None
    }

    fn read_pixels(&self, _pixels: &Pixels) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    fn read_image(&self, _file: &[u8]) -> Result<(ImageText, (u32, u32)), String> {
        Err("no image".into())
    }
}

struct NoSetup;

impl ThreadInit for NoSetup {
    fn init_thread(&self) {}
}

/// A thumbnail that drifts on every frame, so the screen counts as changing and the frame rules
/// have something to save.
struct FakeThumbs {
    width: u32,
    height: u32,
    taken: u32,
}

impl Thumbs for FakeThumbs {
    type Picture = ();

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn thumb(&mut self, _picture: &()) -> Result<Thumb, PlatformError> {
        let (w, h) = (self.width / 8, self.height / 8);
        let luma = (self.taken.wrapping_mul(40) % 256) as u8;
        self.taken += 1;
        Ok(Thumb {
            width: w,
            height: h,
            luma: vec![luma; (w * h) as usize],
        })
    }
}

#[derive(Clone)]
struct FakeDevice {
    video: bool,
}

/// The calls the video writer saw, for the test to check.
type Calls = Arc<Mutex<Vec<&'static str>>>;

struct FakeWriter {
    path: PathBuf,
    start_t: u64,
    start_boot: Option<u64>,
    last_boot: u64,
    calls: Calls,
}

impl FakeWriter {
    fn note(&self, what: &'static str) {
        self.calls.lock().unwrap().push(what);
    }

    fn bytes(&self) -> u64 {
        fs::metadata(&self.path).map_or(0, |m| m.len())
    }
}

impl VideoWriter for FakeWriter {
    type Picture = ();

    fn write(&mut self, _picture: &(), boot: u64) -> Result<(), VideoError> {
        self.start_boot.get_or_insert(boot);
        self.last_boot = boot;
        self.note("write");
        let mut bytes = fs::read(&self.path).unwrap_or_default();
        bytes.push(1);
        fs::write(&self.path, bytes).map_err(|e| VideoError::Disk(e.to_string()))
    }

    fn hold(&mut self, _picture: &(), _boot: u64) {
        self.note("hold");
    }

    fn tick(&mut self, _boot_now: u64) -> Result<(), VideoError> {
        Ok(())
    }

    fn flush(&mut self) -> Result<(), VideoError> {
        self.note("flush");
        Ok(())
    }

    fn finish(self) -> Result<u64, VideoError> {
        self.note("finish");
        Ok(self.bytes())
    }

    fn abandon(&mut self) -> u64 {
        self.note("abandon");
        self.bytes()
    }

    fn encoder(&self) -> (&str, bool) {
        ("Fake H.264 encoder", false)
    }

    fn video_size(&self) -> (u32, u32) {
        (WIDTH, HEIGHT)
    }

    fn end_t(&self) -> u64 {
        match self.start_boot {
            Some(start) => self.start_t + self.last_boot.saturating_sub(start) / 10_000,
            None => self.start_t,
        }
    }
}

struct FakeMaker {
    calls: Calls,
}

impl VideoMaker for FakeMaker {
    type File = FakeWriter;

    fn create(
        &mut self,
        path: &Path,
        _width: u32,
        _height: u32,
        options: WriterOptions,
    ) -> Result<FakeWriter, VideoError> {
        self.calls.lock().unwrap().push("create");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| VideoError::Disk(e.to_string()))?;
        }
        // Fail-if-exists, as a real video file is created.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| VideoError::Encoder(e.to_string()))?;
        Ok(FakeWriter {
            path: path.to_path_buf(),
            start_t: options.start_t,
            start_boot: None,
            last_boot: options.start_t,
            calls: self.calls.clone(),
        })
    }
}

/// Frames every 20 ms on a clock that moves 50 ms with each one, until it is stopped. With
/// `stop_after` it stops itself after that many frames, as ScreenCaptureKit does, and puts the
/// permission off when `revokes` says this stop takes it away.
struct FakeSource {
    clock: Arc<FakeClock>,
    to_main: std::sync::mpsc::SyncSender<Ev<()>>,
    allowed: Arc<AtomicBool>,
    stop_after: Option<u32>,
    revoke_on_stop: bool,
    says_permission: bool,
    /// Every start fails, saying the permission is gone, while the preflight still says yes.
    refuses_for_permission: bool,
    /// After this many frames the permission goes, and the frames keep coming, as macOS does.
    revoke_after: Option<u32>,
    /// The stop is the user's, from the system's control.
    by_user: bool,
    starts: Arc<AtomicU32>,
    /// The stop never reaches the queue, as when it is full; `health` has it.
    loses_notice: bool,
    kept: Arc<Mutex<Option<CaptureStop>>>,
    /// The pids `exclude` gave the source.
    excluded: Arc<Mutex<Option<Vec<u32>>>>,
    worker: Option<(std::thread::JoinHandle<()>, Arc<AtomicBool>)>,
}

impl Source for FakeSource {
    fn start(&mut self, _monitor: &MonitorInfo, generation: u64) -> Result<(), PlatformError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.refuses_for_permission {
            return Err(PlatformError {
                code: "SCError -3801".into(),
                permission: true,
                ..PlatformError::new("the user declined the capture")
            });
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (clock, to_main, flag) = (self.clock.clone(), self.to_main.clone(), stop.clone());
        let (allowed, stop_after, revoke, says) = (
            self.allowed.clone(),
            self.stop_after,
            self.revoke_on_stop,
            self.says_permission,
        );
        let (loses, kept) = (self.loses_notice, self.kept.clone());
        let revoke_after = self.revoke_after;
        let by_user = self.by_user;
        let handle = std::thread::Builder::new()
            .name("fake-screen".into())
            .spawn(move || {
                let mut sent = 0u32;
                while !flag.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(20));
                    if stop_after.is_some_and(|n| sent >= n) {
                        if revoke {
                            allowed.store(false, Ordering::SeqCst);
                        }
                        let stop = CaptureStop {
                            why: "the capture stopped".into(),
                            permission_lost: says,
                            by_user,
                        };
                        if loses {
                            *kept.lock().unwrap() = Some(stop);
                        } else {
                            let _ = to_main.try_send(Ev::CaptureStopped(generation, stop));
                        }
                        return;
                    }
                    if revoke_after == Some(sent) {
                        allowed.store(false, Ordering::SeqCst);
                    }
                    let frame = Frame {
                        picture: (),
                        boot: clock.advance(50 * 10_000),
                        width: WIDTH,
                        height: HEIGHT,
                        dirty: Some(1.0),
                        generation,
                    };
                    sent += 1;
                    if to_main.try_send(Ev::Frame(frame)).is_err() {
                        return;
                    }
                }
            })
            .map_err(|e| PlatformError::new(format!("can't start the fake source: {e}")))?;
        self.worker = Some((handle, stop));
        Ok(())
    }

    fn stop(&mut self) {
        if let Some((handle, stop)) = self.worker.take() {
            stop.store(true, Ordering::SeqCst);
            let _ = handle.join();
        }
        *self.kept.lock().unwrap() = None;
    }

    fn health(&mut self) -> Option<CaptureStop> {
        self.kept.lock().unwrap().take()
    }

    fn exclude(&mut self, pids: &[u32]) {
        *self.excluded.lock().unwrap() = Some(pids.to_vec());
    }
}

impl Drop for FakeSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
struct FakePlatform {
    clock: Arc<FakeClock>,
    calls: Calls,
    /// Whether new devices have the video support the video needs.
    video: bool,
    /// Whether this process may capture the screen.
    allowed: Arc<AtomicBool>,
    displays: FakeDisplays,
    /// The capture stops itself after this many frames, as ScreenCaptureKit does when the
    /// display goes or the permission is revoked mid-recording.
    stop_after: Option<u32>,
    /// Whether that stop takes the permission with it, as the preflight reports it.
    revoke_on_stop: bool,
    /// Whether the stop itself says that the permission is gone.
    says_permission: bool,
    /// Every start is refused for the permission.
    refuses_for_permission: bool,
    /// The capture's stop notice is lost; only `health` has it.
    loses_notice: bool,
    /// The permission goes after this many frames, and the capture goes on.
    revoke_after: Option<u32>,
    /// The capture's stop says the user stopped it.
    by_user: bool,
    /// How many times a capture was started.
    starts: Arc<AtomicU32>,
    /// What the source's `exclude` was last given.
    excluded: Arc<Mutex<Option<Vec<u32>>>>,
}

impl FakePlatform {
    fn new(video: bool) -> FakePlatform {
        let allowed = Arc::new(AtomicBool::new(true));
        FakePlatform {
            clock: Arc::new(FakeClock::new()),
            calls: Arc::new(Mutex::new(Vec::new())),
            video,
            displays: FakeDisplays {
                allowed: allowed.clone(),
                windows: Arc::new(AtomicU32::new(0)),
                size: Arc::new(Mutex::new((WIDTH, HEIGHT))),
            },
            allowed,
            stop_after: None,
            revoke_on_stop: false,
            says_permission: false,
            refuses_for_permission: false,
            starts: Arc::new(AtomicU32::new(0)),
            loses_notice: false,
            revoke_after: None,
            by_user: false,
            excluded: Arc::new(Mutex::new(None)),
        }
    }

    /// A capture the user stops from the system's control after `frames` frames.
    fn stopped_by_the_user(mut self, frames: u32) -> FakePlatform {
        self.stop_after = Some(frames);
        self.says_permission = true;
        self.by_user = true;
        self
    }

    /// The permission goes after `frames` frames while the capture keeps delivering.
    fn revoking_silently(mut self, frames: u32) -> FakePlatform {
        self.revoke_after = Some(frames);
        self
    }

    /// The capture's stop notice never reaches the queue.
    fn losing_the_notice(mut self) -> FakePlatform {
        self.loses_notice = true;
        self
    }

    /// A capture whose every start is refused for the permission while the preflight says yes.
    fn refusing_for_permission(mut self) -> FakePlatform {
        self.refuses_for_permission = true;
        self
    }

    /// A capture that stops itself after `frames` frames, as when the user revokes Screen
    /// Recording; with `revoke`, the permission is gone with it.
    fn stopping(mut self, frames: u32, revoke: bool) -> FakePlatform {
        self.stop_after = Some(frames);
        self.revoke_on_stop = revoke;
        self
    }

    /// A capture whose stop says the permission is gone while the preflight still answers yes, as
    /// it can for a grant revoked a moment ago.
    fn stopping_for_permission(mut self, frames: u32) -> FakePlatform {
        self.stop_after = Some(frames);
        self.says_permission = true;
        self
    }

    /// A capture without the Screen Recording permission.
    fn without_permission(mut self) -> FakePlatform {
        self.allowed = Arc::new(AtomicBool::new(false));
        self.displays.allowed = self.allowed.clone();
        self
    }

    /// What `z_order` last saw, for the test to read.
    fn windows(&self) -> Arc<AtomicU32> {
        self.displays.windows.clone()
    }
}

impl Platform for FakePlatform {
    type Picture = ();
    type Device = FakeDevice;
    type Thumbs = FakeThumbs;
    type Video = FakeWriter;

    fn init(&self) {}

    fn thread_init(&self) -> Arc<dyn ThreadInit> {
        Arc::new(NoSetup)
    }

    fn clocks(&self) -> &dyn Clocks {
        self.clock.as_ref()
    }

    fn displays(&self) -> &dyn Displays {
        &self.displays
    }

    fn disk(&self) -> Arc<dyn Disk> {
        Arc::new(PlentyDisk)
    }

    fn jpeg(&self) -> Arc<dyn Jpeg> {
        Arc::new(FakeJpeg)
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::new(NoText)
    }

    fn locked(&self) -> bool {
        false
    }

    fn frame_io(&self) -> FrameIo {
        FrameIo {
            setup: self.thread_init(),
            jpeg: self.jpeg(),
            disk: self.disk(),
            reader: Box::new(|| Box::new(NoText)),
        }
    }

    fn create_device(&self, want_video: bool, _warp: bool) -> Result<FakeDevice, PlatformError> {
        Ok(FakeDevice {
            video: want_video && self.video,
        })
    }

    fn device_info(&self, device: &FakeDevice) -> DeviceInfo {
        DeviceInfo {
            video: device.video,
            warp: false,
        }
    }

    fn device_removed(&self, _device: &FakeDevice) -> bool {
        false
    }

    fn device_memory(&self, _device: &FakeDevice) -> Option<u64> {
        None
    }

    fn source(
        &self,
        _device: FakeDevice,
        _options: &voice_screen::platform::SourceOptions,
        to_main: std::sync::mpsc::SyncSender<Ev<()>>,
    ) -> Box<dyn Source> {
        Box::new(FakeSource {
            clock: self.clock.clone(),
            to_main,
            allowed: self.allowed.clone(),
            stop_after: self.stop_after,
            revoke_on_stop: self.revoke_on_stop,
            says_permission: self.says_permission,
            refuses_for_permission: self.refuses_for_permission,
            revoke_after: self.revoke_after,
            by_user: self.by_user,
            starts: self.starts.clone(),
            loses_notice: self.loses_notice,
            kept: Arc::new(Mutex::new(None)),
            excluded: self.excluded.clone(),
            worker: None,
        })
    }

    fn thumbs(
        &self,
        _device: &FakeDevice,
        width: u32,
        height: u32,
    ) -> Result<FakeThumbs, PlatformError> {
        Ok(FakeThumbs {
            width,
            height,
            taken: 0,
        })
    }

    fn pixels(&self, _device: &FakeDevice, frame: &Frame<()>) -> Result<Pixels, PlatformError> {
        Ok(Pixels {
            bgra: vec![0; (frame.width * frame.height * 4) as usize],
            width: frame.width,
            height: frame.height,
        })
    }

    fn video_maker(
        &self,
        _device: &FakeDevice,
        _first: &(),
    ) -> Option<Box<dyn VideoMaker<File = FakeWriter>>> {
        Some(Box::new(FakeMaker {
            calls: self.calls.clone(),
        }))
    }
}

/// A data folder with a meeting in it, and its recorder timeline.
struct Meeting {
    root: PathBuf,
    dir: PathBuf,
}

impl Meeting {
    fn new(name: &str) -> Meeting {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "voice-screen-orchestration-{name}-{}-{nanos}",
            std::process::id()
        ));
        let dir = root.join("meetings").join("m-20261111-120000-abcd");
        fs::create_dir_all(dir.join("screen").join("frames")).unwrap();
        fs::write(
            dir.join("meeting.json"),
            r#"{"id":"m-20261111-120000-abcd","screen":"video","version":1}"#,
        )
        .unwrap();
        let meeting = Meeting { root, dir };
        meeting.line(&format!(
            r#"{{"kind":"session","t":0,"wall":{WALL0},"boot":{BOOT0},"session":1,"pid":1,"resumed":false,"version":1,"recorder":"test"}}"#
        ));
        meeting
    }

    fn line(&self, line: &str) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("timeline.jsonl"))
            .unwrap();
        writeln!(file, "{line}").unwrap();
    }

    fn opts(&self) -> RecordOptions {
        RecordOptions {
            meeting: self.dir.clone(),
            mode: Mode::Video,
            resume: false,
            monitor: None,
            exclude_pids: Vec::new(),
            test: None,
        }
    }

    fn journal(&self) -> Vec<serde_json::Value> {
        let text =
            fs::read_to_string(self.dir.join("screen").join("journal.jsonl")).unwrap_or_default();
        let complete = match text.rfind('\n') {
            Some(end) => &text[..end],
            None => "",
        };
        complete
            .lines()
            .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}")))
            .collect()
    }

    fn wait_for(
        &self,
        what: &str,
        done: impl Fn(&[serde_json::Value]) -> bool,
    ) -> Vec<serde_json::Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let lines = self.journal();
            if done(&lines) {
                return lines;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {lines:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Meeting {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn kinds(lines: &[serde_json::Value]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The environment `orphan_after` reads is per process, so tests that set it take turns.
static ENV: Mutex<()> = Mutex::new(());

/// Appends `<crate> <test>: <reason>` to the file `RUKOO_VOICE_SKIP_LOG` names. A skip prints to
/// stderr, which libtest hides for a passing test; CI's `skip-log.mjs report` step reads this file
/// back. Without the variable nothing is written.
fn log_skip(test: &str, reason: &str) {
    use std::io::Write;
    let Some(path) = std::env::var_os("RUKOO_VOICE_SKIP_LOG").filter(|p| !p.is_empty()) else {
        return;
    };
    // One write of the whole line, so tests running in parallel threads don't interleave.
    let line = format!("{} {test}: {reason}\n", env!("CARGO_PKG_NAME"));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// True, after printing why, when CI's host probe found the host too stalled for timing windows
/// (`voice/scripts/host-probe.mjs` sets `RUKOO_VOICE_SLOW_HOST` for the test step). Windows and
/// Linux CI never set it.
fn skip_on_slow_host(test: &str) -> bool {
    let slow = std::env::var_os("RUKOO_VOICE_SLOW_HOST").is_some_and(|v| !v.is_empty());
    if slow {
        let reason =
            "RUKOO_VOICE_SLOW_HOST is set, this host stalls longer than its timing windows allow";
        eprintln!("skipped {test}: {reason}");
        log_skip(test, reason);
    }
    slow
}

/// A whole session with a fake backend: the journal lines a meeting's screen folder gets, the
/// frames, the video file and its `video_end`, and the stop the recorder asks for.
#[test]
fn a_session_runs_to_the_recorders_stop() {
    // The session is paced in real time against a fake clock and the wait below has a 15 s deadline;
    // a host that runs it at under half speed times out with nothing wrong.
    if skip_on_slow_host("a_session_runs_to_the_recorders_stop") {
        return;
    }
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("stop");
    let platform = FakePlatform::new(true);
    let calls = platform.calls.clone();
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));

    meeting.wait_for("frames and a video file", |lines| {
        kinds(lines).iter().filter(|k| *k == "frame").count() >= 2
            && lines.iter().any(|line| line["kind"] == "video")
    });
    meeting.line(r#"{"kind":"stop","t":4000,"wall":1790000001000,"reason":"user"}"#);
    handle.join().unwrap().expect("record failed");

    let lines = meeting.journal();
    let kinds = kinds(&lines);
    assert_eq!(
        kinds.first().map(String::as_str),
        Some("session"),
        "{kinds:?}"
    );
    assert_eq!(lines[0]["mode"], "video");
    assert_eq!(lines[0]["anchor"], "boot");
    assert!(kinds.contains(&"monitor".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"capture".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"video_end".to_string()), "{kinds:?}");
    let stop = lines.last().unwrap();
    assert_eq!(stop["kind"], "stop", "{kinds:?}");
    assert_eq!(stop["reason"], "meeting");
    let frame = lines.iter().find(|line| line["kind"] == "frame").unwrap();
    assert_eq!(
        (frame["width"].as_u64(), frame["height"].as_u64()),
        (Some(u64::from(WIDTH)), Some(u64::from(HEIGHT)))
    );
    assert_eq!(frame["ocr"], "none");
    assert!(meeting.dir.join(frame["file"].as_str().unwrap()).is_file());
    let video = lines.iter().find(|line| line["kind"] == "video").unwrap();
    assert_eq!(video["encoder"], "Fake H.264 encoder");
    assert!(meeting.dir.join(video["file"].as_str().unwrap()).is_file());
    assert!(
        !meeting.dir.join("screen").join("screen.json").exists(),
        "screen.json stays after a stop"
    );
    let calls = calls.lock().unwrap().clone();
    for call in ["create", "write", "finish"] {
        assert!(calls.contains(&call), "{call} wasn't called: {calls:?}");
    }
}

/// No timeline line and nobody asking the status for the orphan window: the screen process stops
/// itself, because the screen is never recorded without the audio.
#[test]
fn a_silent_recorder_ends_the_session() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("orphan");
    std::env::set_var("RUKOO_VOICE_DATA_DIR", &meeting.root);
    std::env::set_var("VOICE_SCREEN_TEST_ORPHAN_MS", "500");
    let platform = FakePlatform::new(true);
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));

    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the session never stopped: {:?}",
            meeting.journal()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap().expect("record failed");
    std::env::remove_var("RUKOO_VOICE_DATA_DIR");
    std::env::remove_var("VOICE_SCREEN_TEST_ORPHAN_MS");

    let lines = meeting.journal();
    let stop = lines.last().unwrap();
    assert_eq!(stop["kind"], "stop", "{:?}", kinds(&lines));
    assert_eq!(stop["reason"], "interrupted");
    assert!(!paths::screen_dir(&meeting.dir).join("screen.json").exists());
}

/// A journal that has already stopped refuses a new session before it writes anything.
#[test]
fn a_stopped_journal_is_refused() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("refused");
    fs::write(
        meeting.dir.join("screen").join("journal.jsonl"),
        "{\"kind\":\"stop\",\"t\":10,\"reason\":\"user\"}\n",
    )
    .unwrap();
    let error = record(FakePlatform::new(true), meeting.opts()).unwrap_err();
    assert_eq!(error.code, voice_screen::exit::CONFLICT, "{error}");
}

/// The user revokes Screen Recording mid-meeting: the capture stops, both parts are dropped, the
/// files keep what reached them and the session ends cleanly. The audio is the recorder's and is
/// never touched from here.
#[test]
fn a_revoked_screen_permission_drops_the_screen_cleanly() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("revoked");
    let platform = FakePlatform::new(true).stopping(3, true);
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the session never stopped: {:?}",
            meeting.journal()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap().expect("record failed");

    let lines = meeting.journal();
    let kinds = kinds(&lines);
    let dropped: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|line| line["kind"] == "dropped")
        .collect();
    // Video first, then the frames, as everywhere else a part of the screen goes.
    assert_eq!(
        dropped
            .iter()
            .map(|line| (line["what"].as_str(), line["reason"].as_str()))
            .collect::<Vec<_>>(),
        [
            (Some("video"), Some("device")),
            (Some("frames"), Some("device"))
        ],
        "{kinds:?}"
    );
    assert_eq!(
        lines.last().unwrap()["kind"],
        "stop",
        "the session didn't stop: {kinds:?}"
    );
    assert_eq!(lines.last().unwrap()["reason"], "dropped");
    // The journal says why the drops follow, before them.
    let at = |kind: &str| lines.iter().position(|line| line["kind"] == kind);
    assert!(
        at("no_permission").unwrap() < at("dropped").unwrap(),
        "{kinds:?}"
    );
    assert!(!paths::screen_dir(&meeting.dir).join("screen.json").exists());
    // The files that were written stay where they are: every frame's JPEG is on disk and the
    // video file ended with `error`, before the drops.
    for frame in lines.iter().filter(|line| line["kind"] == "frame") {
        let file = frame["file"].as_str().unwrap();
        assert!(meeting.dir.join(file).is_file(), "{file}");
    }
    let end = lines
        .iter()
        .find(|line| line["kind"] == "video_end")
        .expect("no video_end");
    assert_eq!(end["reason"], "error");
}

/// A stop that names the permission ends the screen part even when the preflight still says yes:
/// macOS can report a grant revoked a moment ago as granted, and trying again would only fail.
#[test]
fn a_stop_that_names_the_permission_drops_the_screen() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("named-permission");
    let platform = FakePlatform::new(true).stopping_for_permission(3);
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the session never stopped: {:?}",
            meeting.journal()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap().expect("record failed");
    let lines = meeting.journal();
    assert!(lines.iter().any(|line| line["kind"] == "no_permission"));
    assert!(lines.iter().any(|line| line["kind"] == "dropped"));
    assert_eq!(lines.last().unwrap()["reason"], "dropped");
}

/// A start the platform refuses for the permission ends the screen part at once, even while the
/// preflight still says yes: it isn't tried again every second for the rest of the meeting.
#[test]
fn a_start_refused_for_permission_drops_the_screen() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("start-refused");
    let platform = FakePlatform::new(true).refusing_for_permission();
    let starts = platform.starts.clone();
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the session never stopped: {:?}",
            meeting.journal()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap().expect("record failed");
    let lines = meeting.journal();
    let kinds = kinds(&lines);
    let at = |kind: &str| kinds.iter().position(|k| k == kind);
    assert!(
        at("no_permission").unwrap() < at("dropped").unwrap(),
        "{kinds:?}"
    );
    assert!(!kinds.contains(&"capture".to_string()), "{kinds:?}");
    assert_eq!(lines.last().unwrap()["reason"], "dropped");
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "started again after the refusal"
    );
}

/// A capture that stops every time it starts, with its display and permission in place, is tried
/// again after a growing wait, and after ten failures in a row the screen part ends with a reason
/// of its own: it doesn't open a video file a second for the rest of the meeting.
#[test]
fn a_capture_that_keeps_stopping_ends_the_screen() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("keeps-stopping");
    std::env::set_var("RUKOO_VOICE_DATA_DIR", &meeting.root);
    std::env::set_var("VOICE_SCREEN_TEST_CAPTURE_RETRY_MS", "5");
    let platform = FakePlatform::new(true).stopping(0, false);
    let starts = platform.starts.clone();
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the session never stopped: {:?}",
            meeting.journal()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let result = handle.join().unwrap();
    std::env::remove_var("RUKOO_VOICE_DATA_DIR");
    std::env::remove_var("VOICE_SCREEN_TEST_CAPTURE_RETRY_MS");
    result.expect("record failed");
    let lines = meeting.journal();
    let kinds = kinds(&lines);
    let dropped: Vec<(Option<&str>, Option<&str>)> = lines
        .iter()
        .filter(|line| line["kind"] == "dropped")
        .map(|line| (line["what"].as_str(), line["reason"].as_str()))
        .collect();
    assert_eq!(
        dropped,
        [
            (Some("video"), Some("capture")),
            (Some("frames"), Some("capture"))
        ],
        "{kinds:?}"
    );
    assert!(!kinds.contains(&"no_permission".to_string()), "{kinds:?}");
    assert_eq!(lines.last().unwrap()["reason"], "dropped");
    assert_eq!(starts.load(Ordering::SeqCst), 10);
}

/// A capture that stopped with the permission still in place is a display that went and came
/// back: the session records again instead of giving up the screen.
#[test]
fn a_capture_that_stops_with_its_permission_starts_again() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("restart");
    let platform = FakePlatform::new(true).stopping(3, false);
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    meeting.wait_for("a second capture", |lines| {
        kinds(lines).iter().filter(|k| *k == "capture").count() >= 2
    });
    meeting.line(r#"{"kind":"stop","t":4000,"wall":1790000001000,"reason":"user"}"#);
    handle.join().unwrap().expect("record failed");
    let lines = meeting.journal();
    assert!(
        !lines.iter().any(|line| line["kind"] == "dropped"),
        "{:?}",
        kinds(&lines)
    );
    assert!(!lines.iter().any(|line| line["kind"] == "no_permission"));
    assert_eq!(lines.last().unwrap()["reason"], "meeting");
}

/// A display that changes its mode mid-meeting is captured again at its new size: a `monitor`
/// line with reason `resize`, the video file ended for it, and a new capture.
#[test]
fn a_monitor_that_changes_its_size_is_captured_again() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("resized");
    let platform = FakePlatform::new(true);
    let size = platform.displays.size.clone();
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    meeting.wait_for("a video file", |lines| {
        lines.iter().any(|line| line["kind"] == "video")
    });
    *size.lock().unwrap() = (WIDTH * 2, HEIGHT * 2);
    let lines = meeting.wait_for("a second capture", |lines| {
        kinds(lines).iter().filter(|k| *k == "capture").count() >= 2
    });
    meeting.line(r#"{"kind":"stop","t":60000,"wall":1790000060000,"reason":"user"}"#);
    handle.join().unwrap().expect("record failed");
    let resize = lines
        .iter()
        .find(|line| line["kind"] == "monitor" && line["reason"] == "resize")
        .expect("no monitor line for the new size");
    assert_eq!(
        (resize["width"].as_u64(), resize["height"].as_u64()),
        (Some(u64::from(WIDTH * 2)), Some(u64::from(HEIGHT * 2)))
    );
    let ended = lines
        .iter()
        .find(|line| line["kind"] == "video_end")
        .expect("the video file didn't end");
    assert_eq!(ended["reason"], "resize");
    let kinds = kinds(&lines);
    let at = |kind: &str, from: usize| {
        kinds[from..]
            .iter()
            .position(|k| k == kind)
            .map(|n| n + from)
    };
    let resized = lines
        .iter()
        .position(|line| line["reason"] == "resize" && line["kind"] == "monitor")
        .unwrap();
    assert!(at("capture", resized).is_some(), "{kinds:?}");
}

/// Screen Recording switched off mid-meeting while the stream goes on delivering, as macOS lets
/// it: the session notices the grant is gone by itself, stops capturing and ends the screen part
/// with `no_permission`, and no frame after that is saved.
#[test]
fn a_grant_revoked_under_a_running_capture_ends_it() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("silent-revoke");
    let platform = FakePlatform::new(true).revoking_silently(5);
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the session never noticed the revoke: {:?}",
            meeting.journal()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap().expect("record failed");
    let lines = meeting.journal();
    let kinds = kinds(&lines);
    let at = |kind: &str| kinds.iter().position(|k| k == kind);
    let revoked = at("no_permission").expect("no no_permission line");
    assert!(revoked < at("dropped").unwrap(), "{kinds:?}");
    assert!(
        !kinds[revoked..]
            .iter()
            .any(|k| k == "frame" || k == "capture"),
        "{kinds:?}"
    );
    assert_eq!(lines.last().unwrap()["reason"], "dropped");
}

/// The user stops the capture from the system's control: the screen part ends as for a lost
/// permission, and the journal says what happened instead of `no_permission`.
#[test]
fn a_capture_the_user_stopped_ends_with_its_own_line() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("user-stopped");
    let platform = FakePlatform::new(true).stopped_by_the_user(3);
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(Instant::now() < deadline, "{:?}", meeting.journal());
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap().expect("record failed");
    let lines = meeting.journal();
    let kinds = kinds(&lines);
    let at = |kind: &str| kinds.iter().position(|k| k == kind);
    assert!(
        at("stopped_by_user").unwrap() < at("dropped").unwrap(),
        "{kinds:?}"
    );
    assert!(at("no_permission").is_none(), "{kinds:?}");
    assert_eq!(lines.last().unwrap()["reason"], "dropped");
}

/// A stop whose notice the queue lost is still noticed: the tick reads the source's health, so
/// a capture that ended doesn't leave a recording that looks live. Here the permission went with
/// it, and the screen part ends as it does when the notice arrives.
#[test]
fn a_stop_whose_notice_was_lost_is_noticed() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("lost-notice");
    let platform = FakePlatform::new(true)
        .stopping_for_permission(3)
        .losing_the_notice();
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !handle.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the session never noticed the stop: {:?}",
            meeting.journal()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.join().unwrap().expect("record failed");
    let lines = meeting.journal();
    assert!(lines.iter().any(|line| line["kind"] == "no_permission"));
    assert_eq!(lines.last().unwrap()["reason"], "dropped");
}

/// The exclusions the command line names reach the display choice, so a meeting started from
/// Rukoo picks the display behind Rukoo's window.
#[test]
fn the_exclusions_reach_the_display_choice() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("exclusions");
    let platform = FakePlatform::new(true);
    let seen = platform.windows();
    let mut opts = meeting.opts();
    opts.exclude_pids = vec![4821, 77];
    let handle = std::thread::spawn(move || record(platform, opts));
    meeting.wait_for("capture", |lines| {
        kinds(lines).iter().any(|k| k == "capture")
    });
    meeting.line(r#"{"kind":"stop","t":4000,"wall":1790000001000,"reason":"user"}"#);
    handle.join().unwrap().expect("record failed");
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

/// A Rukoo that connects to Voice mid-meeting: Voice sends its pid over the control port, and
/// the capture leaves its windows out from then on. A bad list is refused by the port.
#[test]
fn exclusions_arrive_over_the_control_port() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("exclude-command");
    let platform = FakePlatform::new(true);
    let excluded = platform.excluded.clone();
    let opts = meeting.opts();
    let handle = std::thread::spawn(move || record(platform, opts));
    meeting.wait_for("capture", |lines| {
        kinds(lines).iter().any(|k| k == "capture")
    });
    let file = voice_screen_files::screenfile::read(&meeting.dir).expect("screen.json");
    let ask = |body: serde_json::Value| {
        voice_screen_files::control::request(file.port, &file.token, body, Duration::from_secs(5))
            .unwrap()
    };
    let reply = ask(serde_json::json!({ "cmd": "exclude", "pids": [4821, 77] }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(*excluded.lock().unwrap(), Some(vec![77, 4821]));
    let reply = ask(serde_json::json!({ "cmd": "exclude", "pids": [0] }));
    assert_eq!(reply["error"], "bad_request", "{reply}");
    assert_eq!(*excluded.lock().unwrap(), Some(vec![77, 4821]));
    meeting.line(r#"{"kind":"stop","t":60000,"wall":1790000060000,"reason":"user"}"#);
    handle.join().unwrap().expect("record failed");
}

/// Without the Screen Recording permission the session refuses before it journal anything, with
/// the platform's reason: the meeting's audio goes on without a screen.
#[test]
fn without_the_capture_permission_the_session_refuses() {
    let _turn = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let meeting = Meeting::new("no-permission");
    let error = record(FakePlatform::new(true).without_permission(), meeting.opts()).unwrap_err();
    // Its own exit code, so Voice shows the reason and doesn't retry.
    assert_eq!(error.code, voice_screen::exit::NO_PERMISSION, "{error}");
    assert!(error.message.contains("Screen Recording"), "{error}");
    assert!(
        !meeting.dir.join("screen").join("journal.jsonl").exists(),
        "the refused session left a journal"
    );
}
