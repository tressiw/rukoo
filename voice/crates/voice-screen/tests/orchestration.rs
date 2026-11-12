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
}

impl Displays for FakeDisplays {
    fn list(&self, _test: Option<&voice_screen::args::TestSpec>) -> Vec<MonitorInfo> {
        vec![MonitorInfo {
            id: MONITOR.into(),
            name: "Fake".into(),
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
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
    worker: Option<(std::thread::JoinHandle<()>, Arc<AtomicBool>)>,
}

impl Source for FakeSource {
    fn start(&mut self, _monitor: &MonitorInfo, generation: u64) -> Result<(), PlatformError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (clock, to_main, flag) = (self.clock.clone(), self.to_main.clone(), stop.clone());
        let (allowed, stop_after, revoke, says) = (
            self.allowed.clone(),
            self.stop_after,
            self.revoke_on_stop,
            self.says_permission,
        );
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
                        let _ = to_main.try_send(Ev::CaptureStopped(
                            generation,
                            CaptureStop {
                                why: "the capture stopped".into(),
                                permission_lost: says,
                            },
                        ));
                        return;
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
            },
            allowed,
            stop_after: None,
            revoke_on_stop: false,
            says_permission: false,
        }
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

    fn video_maker(&self, _device: &FakeDevice) -> Option<Box<dyn VideoMaker<File = FakeWriter>>> {
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

/// A whole session with a fake backend: the journal lines a meeting's screen folder gets, the
/// frames, the video file and its `video_end`, and the stop the recorder asks for.
#[test]
fn a_session_runs_to_the_recorders_stop() {
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
