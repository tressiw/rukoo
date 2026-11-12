//! The screen supervisor, the start handshake, `frame.get`, `clip.get`, `screen.get` and
//! `screen.select` against the real recorder and the real screen process, with synthetic sources.
//!
//! This test binary has its own main (`harness = false`) so it can also be both child processes:
//! run with `VOICE_ENGINE_TEST_ROLE=child` it is the recorder when its command line names a
//! `--track` (only the recorder takes one) and the screen process otherwise, calling the same
//! `voice_recorder::run` and `voice_screen::run` as the two programs' mains.
//!
//! The screen process only ever records its synthetic source here (`--test synthetic:...`), and
//! video uses Microsoft's software encoder (`no-hardware,software-encoder`): nothing of the real
//! desktop is captured and no hardware encoder runs.
//!
//! The screen process records on Windows only until its macOS port, so the tests that start it run
//! there. This binary still lists every test everywhere: the two that never start a screen process
//! run on the other platforms too.

mod support;

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde_json::{json, Value};
use support::{lock, wait_for, Harness, TempDir};
use voice_bridge::{EventLog, MeetingControl};
use voice_engine::media::{MediaService, RecordedScreen};
use voice_engine::meetings::bridge::Control;
use voice_engine::meetings::controller::{Change, Controller, ControllerOptions, ControllerTiming};
use voice_engine::meetings::screen::{ScreenChange, ScreenOptions, ScreenSupervisor, ScreenTiming};
use voice_engine::PillPhase;
use voice_protocol::{ErrorCode, ScreenMode, StartArgs, StartedBy, Track};
use voice_screen_files::journal::{self, Line};
use voice_screen_files::{control, screenfile};

const ROLE: &str = "VOICE_ENGINE_TEST_ROLE";
/// What the screen process records here: a synthetic screen whose text changes every second.
const SCREEN: &str = "synthetic:1280x720,change-every=1000,text=Slide";
/// Video without a hardware encoder: the software MFT on the system-memory path.
const VIDEO: &str = "no-hardware,software-encoder";
/// How soon after the start a screen process that waits for nobody is launched (RUK-108 4.3:
/// under 100 ms). It takes a thread, a read of meeting.json and a file open.
const LAUNCH_SLACK: u64 = 100;

fn main() {
    if std::env::var(ROLE).as_deref() == Ok("child") {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // Only the recorder takes `--track`; the screen process never does.
        if args.iter().any(|a| a == "--track") {
            if let Err(e) = voice_recorder::run(&args) {
                eprintln!("rukoo-voice-recorder: {e}");
                std::process::exit(e.code);
            }
        } else if let Err(e) = voice_screen::run(&args) {
            eprintln!("rukoo-voice-screen: {e}");
            std::process::exit(e.code);
        }
        return;
    }
    // Single-threaded here, so setting the variables is safe. Every child inherits them, and the
    // screen process takes `--test` only in a development copy.
    std::env::set_var(ROLE, "child");
    let scratch = std::env::temp_dir().join(format!("voice-engine-screen-{}", std::process::id()));
    std::env::set_var(voice_core::paths::DATA_DIR_ENV, &scratch);
    let tests: Vec<(&str, fn())> = vec![
        (
            "voice_started_meeting_waits_for_rukoo_then_captures",
            voice_started_meeting_waits_for_rukoo_then_captures,
        ),
        (
            "voice_started_meeting_waits_for_a_reconnecting_rukoo",
            voice_started_meeting_waits_for_a_reconnecting_rukoo,
        ),
        (
            "a_meeting_stopped_during_the_handshake_ends_its_journal",
            a_meeting_stopped_during_the_handshake_ends_its_journal,
        ),
        (
            "rukoo_started_meeting_skips_the_handshake",
            rukoo_started_meeting_skips_the_handshake,
        ),
        (
            "screen_off_sends_no_meeting_starting",
            screen_off_sends_no_meeting_starting,
        ),
        (
            "the_watchdog_restarts_a_killed_screen_process",
            the_watchdog_restarts_a_killed_screen_process,
        ),
        (
            "the_watchdog_gives_up_and_finalizes",
            the_watchdog_gives_up_and_finalizes,
        ),
        ("a_new_supervisor_reattaches", a_new_supervisor_reattaches),
        (
            "recover_finalizes_a_cut_off_screen_journal",
            recover_finalizes_a_cut_off_screen_journal,
        ),
        (
            "frame_get_answers_at_or_before",
            frame_get_answers_at_or_before,
        ),
        ("clip_get_cuts_and_reuses", clip_get_cuts_and_reuses),
        (
            "clip_get_refuses_near_the_video_floor",
            clip_get_refuses_near_the_video_floor,
        ),
        (
            "clip_get_answers_by_its_deadline_while_its_check_runs",
            clip_get_answers_by_its_deadline_while_its_check_runs,
        ),
        ("screen_get_and_select", screen_get_and_select),
        (
            "screen_commands_answer_by_their_deadline_while_starting",
            screen_commands_answer_by_their_deadline_while_starting,
        ),
        (
            "screen_select_answers_by_its_deadline_when_the_screen_is_slow",
            screen_select_answers_by_its_deadline_when_the_screen_is_slow,
        ),
        ("drops_reach_the_pill", drops_reach_the_pill),
        ("encoder_drops_reach_the_pill", encoder_drops_reach_the_pill),
        (
            "pill_screen_menu_changes_modes_and_monitors",
            pill_screen_click_moves_to_the_next_monitor,
        ),
        (
            "screen_mode_changes_require_each_clients_current_ack",
            screen_mode_changes_require_each_clients_current_ack,
        ),
    ];
    // The screen process records on Windows only until its macOS port, so the tests that start one
    // run there. These two never start it and run everywhere: turning the screen off starts nothing,
    // and the slow-screen case answers through a stand-in process of its own.
    #[cfg(not(windows))]
    let tests: Vec<(&str, fn())> = tests
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                *name,
                "screen_off_sends_no_meeting_starting"
                    | "screen_select_answers_by_its_deadline_when_the_screen_is_slow"
            )
        })
        .collect();
    // A renamed test would slip out of that filter without a word.
    #[cfg(not(windows))]
    assert_eq!(
        tests.len(),
        2,
        "the filter in main() keeps the two tests that start no screen process; a rename dropped one out, so \
         update its name there"
    );
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let mut failed = Vec::new();
    let mut ran = 0;
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        ran += 1;
        let started = Instant::now();
        match panic::catch_unwind(AssertUnwindSafe(test)) {
            Ok(()) => println!("test {name} ... ok ({} ms)", started.elapsed().as_millis()),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(name);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    println!(
        "test result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        ran - failed.len(),
        failed.len()
    );
    if !failed.is_empty() {
        std::process::exit(101);
    }
}

// Helpers ------------------------------------------------------------------------------------

fn exe() -> PathBuf {
    std::env::current_exe().unwrap()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// The deadline of a command the bridge runs as soon as it reads it: 4 s from now.
fn in_time() -> Instant {
    Instant::now() + voice_protocol::CLIP_WAIT
}

fn controller_timing() -> ControllerTiming {
    ControllerTiming {
        poll: Duration::from_millis(100),
        start_timeout: Duration::from_secs(20),
        backoff: vec![Duration::from_millis(100), Duration::from_millis(200)],
        stable_after: Duration::from_secs(60),
        ping_failures: 3,
        recorder_args: vec!["--segment-ms".into(), "1000".into()],
    }
}

fn screen_timing() -> ScreenTiming {
    ScreenTiming {
        poll: Duration::from_millis(100),
        start_timeout: Duration::from_secs(20),
        backoff: vec![Duration::from_millis(100), Duration::from_millis(200)],
        stable_after: Duration::from_secs(60),
        ping_failures: 3,
        handshake: Duration::from_millis(500),
        end_wait: Duration::from_secs(10),
        reconnect: Duration::from_secs(5),
    }
}

/// A controller and a screen supervisor wired as the engine wires them, on a temp folder.
struct Rig {
    _dir: TempDir,
    data: PathBuf,
    controller: Arc<Controller>,
    screen: Mutex<Arc<ScreenSupervisor>>,
    /// Where the controller's changes go: the supervisor of the moment.
    slot: Slot,
    log: EventLog,
    changes: Arc<Mutex<Vec<ScreenChange>>>,
    rukoo: Arc<(AtomicUsize, AtomicBool)>,
    started_at: Arc<AtomicU64>,
    /// What the supervisor's free-space stand-in says, when the rig has one.
    free: Arc<AtomicU64>,
    /// How long, in ms, the free-space stand-in takes to answer, as a slow disk can.
    free_wait: Arc<AtomicU64>,
}

type Slot = Arc<OnceLock<Mutex<Weak<ScreenSupervisor>>>>;

impl Rig {
    fn new(name: &str, test: &str) -> Rig {
        Rig::with(name, test, None)
    }

    fn with(name: &str, test: &str, free_bytes: Option<u64>) -> Rig {
        let dir = TempDir::new(name);
        let data = dir.join("data");
        let log = EventLog::open(&data).unwrap();
        let slot: Slot = Arc::default();
        let started_at = Arc::new(AtomicU64::new(0));
        let controller = Controller::new(
            ControllerOptions {
                data_dir: data.clone(),
                recorder_exe: exe(),
                sources: vec![("mic".into(), "synthetic:tone:440".into())],
                timing: controller_timing(),
            },
            {
                let slot = Arc::clone(&slot);
                let started_at = Arc::clone(&started_at);
                move |change| {
                    let Some(screen) = slot.get().and_then(|s| lock(s).upgrade()) else {
                        return;
                    };
                    match change {
                        Change::Started { id, .. } => {
                            started_at.store(now_ms(), Ordering::SeqCst);
                            screen.begin(&id);
                        }
                        Change::Reattached { id, .. } => screen.reattach(&id),
                        Change::Paused { id, .. } => screen.pause(&id, true),
                        Change::Resumed { id, .. } => screen.pause(&id, false),
                        Change::Ended { id, .. } => {
                            thread::spawn(move || screen.end(&id));
                        }
                        _ => {}
                    }
                }
            },
        )
        .unwrap();
        let changes: Arc<Mutex<Vec<ScreenChange>>> = Arc::default();
        let rukoo: Arc<(AtomicUsize, AtomicBool)> = Arc::default();
        let free = Arc::new(AtomicU64::new(free_bytes.unwrap_or(0)));
        let free_wait = Arc::new(AtomicU64::new(0));
        let screen = supervisor(
            &data,
            &log,
            test,
            &changes,
            &rukoo,
            free_bytes.map(|_| (Arc::clone(&free), Arc::clone(&free_wait))),
        );
        let _ = slot.set(Mutex::new(Arc::downgrade(&screen)));
        controller.set_screen(ScreenMode::Frames, screen.available());
        Rig {
            _dir: dir,
            data,
            controller,
            screen: Mutex::new(screen),
            slot,
            log,
            changes,
            rukoo,
            started_at,
            free,
            free_wait,
        }
    }

    fn screen(&self) -> Arc<ScreenSupervisor> {
        Arc::clone(&lock(&self.screen))
    }

    /// A new supervisor in place of the old one, as after the app restarted.
    fn restart_supervisor(&self, test: &str) -> Arc<ScreenSupervisor> {
        let old = self.screen();
        old.shutdown();
        let new = supervisor(
            &self.data,
            &self.log,
            test,
            &self.changes,
            &self.rukoo,
            None,
        );
        if let Some(slot) = self.slot.get() {
            *lock(slot) = Arc::downgrade(&new);
        }
        *lock(&self.screen) = Arc::clone(&new);
        new
    }

    fn control(&self) -> Control {
        Control::new(
            Arc::clone(&self.controller),
            self.screen(),
            self.log.clone(),
            Arc::default(),
            // Media links read the same screen folder; no bucket sync here.
            MediaService::new(
                &self.data,
                Arc::clone(&self.controller),
                None,
                Arc::new(RecordedScreen::default()),
                Arc::new(|_| None),
            ),
        )
    }

    fn rukoo(&self, connected: usize, seen: bool) {
        self.rukoo.0.store(connected, Ordering::SeqCst);
        self.rukoo.1.store(seen, Ordering::SeqCst);
    }

    fn start(&self, by: StartedBy, screen: Option<ScreenMode>) -> String {
        let args = StartArgs {
            tracks: Some(vec![Track::Mic]),
            screen,
            ..StartArgs::default()
        };
        match by {
            StartedBy::Voice => self.controller.start(args, StartedBy::Voice).unwrap(),
            StartedBy::Rukoo => MeetingControl::start(&self.control(), args).unwrap(),
        }
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.data.join("meetings").join(id)
    }

    fn journal(&self, id: &str) -> Vec<Line> {
        journal::read_all(&self.dir(id)).unwrap_or_default()
    }

    /// Waits for a journal line `what` picks, and returns it.
    fn wait_line(&self, id: &str, what: &str, pick: impl Fn(&Line) -> bool) -> Line {
        let mut found = None;
        wait_for(what, || {
            found = self.journal(id).into_iter().find(|l| pick(l));
            found.is_some()
        });
        found.unwrap()
    }

    fn session(&self, id: &str) -> (u64, u64) {
        match self.wait_line(id, "the screen session", |l| {
            matches!(l, Line::Session { .. })
        }) {
            Line::Session { t, wall, .. } => (t, wall),
            _ => unreachable!(),
        }
    }

    fn starting_events(&self, id: &str) -> Vec<voice_bridge::Stored> {
        self.log
            .meeting_events(id)
            .unwrap()
            .1
            .into_iter()
            .filter(|e| e.event == voice_protocol::events::MEETING_STARTING)
            .collect()
    }

    fn screen_pid(&self, id: &str) -> u32 {
        let mut pid = 0;
        wait_for("screen.json", || {
            pid = screenfile::live(&self.dir(id)).map_or(0, |f| f.pid);
            pid != 0
        });
        pid
    }

    fn stop(&self, id: &str) {
        self.controller.stop(Some(id), "user").unwrap();
        self.wait_line(id, "the screen journal's stop", |l| {
            matches!(l, Line::Stop { .. })
        });
    }

    /// When the supervisor launched the meeting's first screen process: it makes the process's
    /// log right before.
    fn launched_at(&self, id: &str) -> u64 {
        let log = self.data.join("logs").join(format!("screen-{id}.log"));
        std::fs::metadata(log)
            .and_then(|m| m.created())
            .map(|at| at.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64)
            .expect("the screen process's log")
    }

    fn changes(&self) -> Vec<ScreenChange> {
        lock(&self.changes).clone()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.screen().shutdown();
        self.controller.shutdown();
        // A failed test leaves a screen process: stop it by its pid, never by name.
        if let Ok(entries) = std::fs::read_dir(self.data.join("meetings")) {
            for entry in entries.flatten() {
                if let Some(file) = screenfile::read(&entry.path()) {
                    if file.pid != std::process::id() {
                        kill(file.pid);
                    }
                }
            }
        }
    }
}

fn supervisor(
    data: &Path,
    log: &EventLog,
    test: &str,
    changes: &Arc<Mutex<Vec<ScreenChange>>>,
    rukoo: &Arc<(AtomicUsize, AtomicBool)>,
    // What the free-space stand-in says, and how many ms it takes.
    free_bytes: Option<(Arc<AtomicU64>, Arc<AtomicU64>)>,
) -> Arc<ScreenSupervisor> {
    let screen = ScreenSupervisor::new(
        ScreenOptions {
            data_dir: data.to_path_buf(),
            exe: exe(),
            test: Some(test.to_string()),
            test_monitor: None,
            timing: screen_timing(),
            free_bytes: free_bytes.map(|(free, wait)| {
                Arc::new(move |_: &Path| {
                    thread::sleep(Duration::from_millis(wait.load(Ordering::SeqCst)));
                    Some(free.load(Ordering::SeqCst))
                }) as voice_engine::meetings::screen::FreeBytes
            }),
        },
        Some(log.clone()),
        {
            let changes = Arc::clone(changes);
            move |change| lock(&changes).push(change)
        },
    )
    .unwrap();
    let rukoo = Arc::clone(rukoo);
    screen.set_rukoo_clients(Box::new(move || {
        (
            rukoo.0.load(Ordering::SeqCst),
            rukoo.1.load(Ordering::SeqCst),
        )
    }));
    screen
}

/// Ends a process this test started, by its process id.
fn kill(pid: u32) {
    #[cfg(windows)]
    let mut command = std::process::Command::new("taskkill");
    #[cfg(windows)]
    command.args(["/PID", &pid.to_string(), "/F"]);
    #[cfg(not(windows))]
    let mut command = std::process::Command::new("kill");
    #[cfg(not(windows))]
    command.args(["-9", &pid.to_string()]);
    let _ = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Acks `meeting.starting` number `nth` for `id` `after` it was logged; returns the ack time.
fn ack_later(rig: &Rig, id: &str, nth: usize, after: Duration) -> thread::JoinHandle<u64> {
    let log = rig.log.clone();
    let screen = rig.screen();
    let id = id.to_string();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let starting = log
                .meeting_events(&id)
                .unwrap()
                .1
                .into_iter()
                .filter(|e| e.event == voice_protocol::events::MEETING_STARTING)
                .count();
            if starting >= nth {
                break;
            }
            assert!(Instant::now() < deadline, "no meeting.starting {nth}");
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(after);
        let at = now_ms();
        screen.ack(&id);
        at
    })
}

// The start handshake -------------------------------------------------------------------------

fn voice_started_meeting_waits_for_rukoo_then_captures() {
    let rig = Rig::new("handshake", SCREEN);
    rig.rukoo(1, true);
    // One Rukoo, which hides its windows and confirms 200 ms later.
    let id = rig.start(StartedBy::Voice, None);
    let acked = ack_later(&rig, &id, 1, Duration::from_millis(200))
        .join()
        .unwrap();
    let (_, wall) = rig.session(&id);
    assert!(
        wall >= acked,
        "capture began at {wall}, before the ack at {acked}"
    );
    let starting = rig.starting_events(&id);
    assert_eq!(starting.len(), 1);
    assert_eq!(starting[0].data, json!({"screen": "frames"}));
    rig.stop(&id);

    // A Rukoo that never confirms: capture waits the 500 ms, then begins.
    let id = rig.start(StartedBy::Voice, None);
    let (_, wall) = rig.session(&id);
    let sent = rig.starting_events(&id)[0].created_at;
    assert!(
        wall >= sent + 450,
        "capture began {} ms after meeting.starting",
        wall - sent
    );
    rig.stop(&id);

    // No Rukoo now and none in this run: no event, no wait.
    rig.rukoo(0, false);
    let id = rig.start(StartedBy::Voice, None);
    rig.session(&id);
    let started = rig.started_at.load(Ordering::SeqCst);
    assert!(rig.starting_events(&id).is_empty());
    let launched = rig.launched_at(&id);
    assert!(
        launched < started + LAUNCH_SLACK,
        "the screen process launched {} ms after the start",
        launched.saturating_sub(started)
    );
    rig.stop(&id);
}

fn voice_started_meeting_waits_for_a_reconnecting_rukoo() {
    let rig = Rig::new("reconnect", SCREEN);
    // A Rukoo was connected earlier and is away now; it comes back 1.5 s after the start.
    rig.rukoo(0, true);
    let id = rig.start(StartedBy::Voice, None);
    let rukoo = Arc::clone(&rig.rukoo);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(1500));
        rukoo.0.store(1, Ordering::SeqCst);
    });
    let acked = ack_later(&rig, &id, 2, Duration::from_millis(200))
        .join()
        .unwrap();
    let (_, wall) = rig.session(&id);
    assert!(
        wall >= acked,
        "capture began at {wall}, before the ack at {acked}"
    );
    assert_eq!(
        rig.starting_events(&id).len(),
        2,
        "a fresh meeting.starting for the Rukoo that came back"
    );
    rig.stop(&id);

    // Nobody comes back: capture begins after the 5 s reconnect wait.
    rig.rukoo(0, true);
    let id = rig.start(StartedBy::Voice, None);
    let started = rig.started_at.load(Ordering::SeqCst);
    let (_, wall) = rig.session(&id);
    assert!(
        wall >= started + 4_900,
        "capture began {} ms after the start",
        wall.saturating_sub(started)
    );
    assert_eq!(rig.starting_events(&id).len(), 1);
    rig.stop(&id);
}

fn a_meeting_stopped_during_the_handshake_ends_its_journal() {
    let rig = Rig::new("stop-early", SCREEN);
    // A Rukoo seen earlier and away now: the handshake waits up to 5 s for it. The meeting
    // stops in that wait, so no screen process ever runs; the journal still gets its `stop`
    // at once, which the pipeline waits for before the meeting is done.
    rig.rukoo(0, true);
    let id = rig.start(StartedBy::Voice, None);
    rig.controller.stop(Some(&id), "user").unwrap();
    let stopped = Instant::now();
    let stop = rig.wait_line(&id, "the journal's stop", |l| {
        matches!(l, Line::Stop { .. })
    });
    assert!(
        matches!(&stop, Line::Stop { reason, .. } if reason == "interrupted"),
        "{stop:?}"
    );
    assert!(
        stopped.elapsed() < Duration::from_secs(3),
        "the stop came {} ms after the meeting's",
        stopped.elapsed().as_millis()
    );
    // Nothing launches once the handshake's wait would have ended.
    thread::sleep(Duration::from_secs(6));
    let lines = rig.journal(&id);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(screenfile::live(&rig.dir(&id)).is_none());
}

fn rukoo_started_meeting_skips_the_handshake() {
    let rig = Rig::new("rukoo-start", SCREEN);
    rig.rukoo(1, true);
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    rig.session(&id);
    let started = rig.started_at.load(Ordering::SeqCst);
    assert!(rig.starting_events(&id).is_empty());
    let launched = rig.launched_at(&id);
    assert!(
        launched < started + LAUNCH_SLACK,
        "the screen process launched {} ms after the start",
        launched.saturating_sub(started)
    );
    rig.stop(&id);
}

fn screen_off_sends_no_meeting_starting() {
    let rig = Rig::new("screen-off", SCREEN);
    rig.rukoo(1, true);
    rig.controller.set_screen(ScreenMode::Off, true);
    let id = rig.start(StartedBy::Voice, None);
    thread::sleep(Duration::from_millis(1500));
    assert!(rig.starting_events(&id).is_empty());
    assert!(rig.journal(&id).is_empty(), "no screen process ran");
    assert_eq!(rig.screen().mode_of(&id), ScreenMode::Off);
    let control = rig.control();
    let info = control.screen(&id, in_time()).unwrap();
    assert_eq!(info["screen"], "off");
    assert_eq!(info["state"], "off");
    assert_eq!(info["frames"], false);
    assert_eq!(
        control.frame(&id, 0).unwrap_err().code,
        ErrorCode::NotAvailable
    );
    rig.controller.stop(Some(&id), "user").unwrap();
}

// The watchdog ---------------------------------------------------------------------------------

fn the_watchdog_restarts_a_killed_screen_process() {
    let rig = Rig::new("restart", SCREEN);
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    let first = rig.screen_pid(&id);
    rig.wait_line(&id, "a frame", |l| matches!(l, Line::Frame { .. }));
    kill(first);
    let mut second = first;
    wait_for("a new screen process", || {
        second = screenfile::live(&rig.dir(&id)).map_or(first, |f| f.pid);
        second != first
    });
    let crash = rig.wait_line(&id, "the resumed session's crash line", |l| {
        matches!(l, Line::Crash { .. })
    });
    let sessions = rig
        .journal(&id)
        .iter()
        .filter(|l| matches!(l, Line::Session { resumed: true, .. }))
        .count();
    assert_eq!(sessions, 1, "one resumed session");
    assert!(matches!(crash, Line::Crash { t, last_seen } if t >= last_seen));
    // The recorder never noticed.
    let timeline = voice_recorder::timeline::read(&rig.dir(&id).join("timeline.jsonl")).unwrap();
    assert!(!timeline
        .iter()
        .any(|l| matches!(l, voice_recorder::timeline::Line::Crash { .. })));
    assert!(rig.controller.current().is_some());
    assert_eq!(rig.screen().mode_of(&id), ScreenMode::Frames);
    rig.stop(&id);
    assert!(!rig
        .changes()
        .iter()
        .any(|c| matches!(c, ScreenChange::GaveUp { .. })));
}

fn the_watchdog_gives_up_and_finalizes() {
    let rig = Rig::new("give-up", SCREEN);
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    // Two restarts are allowed here; the third death is one too many.
    let mut last = 0;
    for _ in 0..3 {
        let mut pid = last;
        wait_for("a screen process", || {
            pid = screenfile::live(&rig.dir(&id)).map_or(last, |f| f.pid);
            pid != last
        });
        kill(pid);
        last = pid;
    }
    wait_for("the supervisor to give up", || {
        rig.changes()
            .iter()
            .any(|c| matches!(c, ScreenChange::GaveUp { id: gave } if *gave == id))
    });
    let stop = rig.wait_line(&id, "the journal's stop", |l| {
        matches!(l, Line::Stop { .. })
    });
    assert!(matches!(stop, Line::Stop { reason, .. } if reason == "failed"));
    assert_eq!(rig.screen().mode_of(&id), ScreenMode::Off);
    assert_eq!(
        rig.screen()
            .select(&id, "anything", in_time())
            .unwrap_err()
            .code,
        ErrorCode::NotAvailable
    );
    // The audio goes on.
    let at = rig.controller.current().unwrap().at_ms;
    wait_for("the recorder to go on", || {
        rig.controller.current().is_some_and(|r| r.at_ms > at + 500)
    });
    rig.controller.stop(Some(&id), "user").unwrap();
}

fn a_new_supervisor_reattaches() {
    let rig = Rig::new("reattach", SCREEN);
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    let pid = rig.screen_pid(&id);
    // The app restarts: the old supervisor stops watching, the process goes on.
    let screen = rig.restart_supervisor(SCREEN);
    screen.recover(Some(&id));
    screen.reattach(&id);
    // The process says `starting` until its first frame arrives.
    let mut described = Value::Null;
    wait_for("the reattached process to record", || {
        described = screen.describe(&id, in_time()).unwrap();
        described["state"] == "recording"
    });
    assert_eq!(described["screen"], "frames");
    assert_eq!(
        screenfile::live(&rig.dir(&id)).unwrap().pid,
        pid,
        "the same process"
    );
    // Pause and resume reach it; the stop ends it.
    rig.controller.pause(Some(&id)).unwrap();
    wait_for("the screen to pause", || {
        screen.describe(&id, in_time()).unwrap()["state"] == "paused"
    });
    rig.controller.resume(Some(&id)).unwrap();
    rig.stop(&id);
    wait_for("the screen process to end", || {
        screenfile::live(&rig.dir(&id)).is_none()
    });
    assert_eq!(
        screen.describe(&id, in_time()).unwrap_err().code,
        ErrorCode::NotRecording
    );
}

fn recover_finalizes_a_cut_off_screen_journal() {
    let rig = Rig::new("recover", SCREEN);
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    let pid = rig.screen_pid(&id);
    rig.wait_line(&id, "a frame", |l| matches!(l, Line::Frame { .. }));
    // The app and both processes die: nothing finishes the journal.
    rig.screen().shutdown();
    rig.controller.shutdown();
    let recorder = voice_recorder::meeting::read_recorder_file(&rig.dir(&id)).unwrap();
    kill(pid);
    kill(recorder.pid);
    wait_for("the screen process to be gone", || {
        screenfile::live(&rig.dir(&id)).is_none()
    });
    voice_recorder::meeting::finalize(&rig.dir(&id), "interrupted").unwrap();
    assert!(!rig
        .journal(&id)
        .iter()
        .any(|l| matches!(l, Line::Stop { .. })));
    // The next start: the meeting doesn't record any more, so its journal gets its stop.
    let screen = rig.restart_supervisor(SCREEN);
    screen.recover(None);
    let lines = rig.journal(&id);
    assert!(
        matches!(lines.last(), Some(Line::Stop { reason, .. }) if reason == "interrupted"),
        "{:?}",
        lines.last()
    );
    // Running it again changes nothing.
    screen.recover(None);
    assert_eq!(rig.journal(&id).len(), lines.len());
}

// What Rukoo asks -------------------------------------------------------------------------------

fn frame_get_answers_at_or_before() {
    let rig = Rig::new("frame-get", SCREEN);
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    let control = rig.control();
    let mut frames: Vec<(u64, String)> = Vec::new();
    wait_for("two frames", || {
        frames = rig
            .journal(&id)
            .into_iter()
            .filter_map(|l| match l {
                Line::Frame { t, id, .. } => Some((t, id)),
                _ => None,
            })
            .collect();
        frames.len() >= 2
    });
    let (first_at, first_id) = frames[0].clone();
    let (second_at, second_id) = frames[1].clone();
    let exact = control.frame(&id, second_at).unwrap();
    assert_eq!(exact["frame"]["id"], second_id.as_str());
    assert_eq!(exact["frame"]["at"], second_at);
    assert_eq!(exact["image"]["type"], "image/jpeg");
    let jpeg = base64::engine::general_purpose::STANDARD
        .decode(exact["image"]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "a JPEG");
    assert!(exact["frame"]["width"].as_u64().unwrap() > 0);
    // Between two frames: the earlier one.
    let between = control.frame(&id, second_at - 1).unwrap();
    assert_eq!(between["frame"]["id"], first_id.as_str());
    assert_eq!(
        control
            .frame(&id, first_at.saturating_sub(1))
            .unwrap_err()
            .code,
        if first_at == 0 {
            ErrorCode::NotAvailable
        } else {
            ErrorCode::NotFound
        }
    );
    assert_eq!(
        control.frame("m-20000101-000000-0000", 1).unwrap_err().code,
        ErrorCode::NotFound
    );
    // A frame whose JPEG retention removed keeps its place and its text, without an image.
    rig.stop(&id);
    std::fs::remove_dir_all(rig.dir(&id).join("screen").join("frames")).unwrap();
    let removed = control.frame(&id, second_at).unwrap();
    assert_eq!(removed["image"], serde_json::Value::Null, "{removed}");
    assert_eq!(removed["frame"]["at"], second_at);
    assert!(removed["frame"]["text"].is_string(), "{removed}");
}

fn video_spec() -> String {
    format!("{SCREEN},{VIDEO}")
}

/// A video meeting of about `seconds`, stopped.
fn recorded_video(rig: &Rig, seconds: u64) -> String {
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Video));
    rig.wait_line(&id, "a video file", |l| matches!(l, Line::Video { .. }));
    wait_for("the recording", || {
        rig.controller
            .current()
            .is_some_and(|r| r.at_ms >= seconds * 1000)
    });
    rig.stop(&id);
    id
}

fn clip_get_cuts_and_reuses() {
    let rig = Rig::new("clip-get", &video_spec());
    let id = recorded_video(&rig, 6);
    let control = rig.control();
    let ask = || control.clip(&id, 1_000, 3_000, Instant::now() + Duration::from_secs(4));
    let mut reply = ask().unwrap();
    // A cut that takes longer goes on; the same request later returns its file.
    let deadline = Instant::now() + Duration::from_secs(60);
    while reply["clips"].as_array().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the clip never came");
        thread::sleep(Duration::from_millis(200));
        reply = ask().unwrap();
    }
    let clip = reply["clips"][0].clone();
    let path = PathBuf::from(clip["path"].as_str().unwrap());
    assert!(path.is_file());
    assert!(path.starts_with(rig.dir(&id).join("screen").join("clips")));
    assert!(clip["start"].as_u64().unwrap() <= 1_000, "{clip}");
    assert!(clip["end"].as_u64().unwrap() >= 3_000, "{clip}");
    assert_eq!(clip["bytes"], std::fs::metadata(&path).unwrap().len());
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    thread::sleep(Duration::from_millis(1100));
    // The same request reuses the file and marks it used.
    let again = ask().unwrap();
    assert_eq!(again["clips"][0]["path"], clip["path"]);
    let after = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(after > before, "a reused clip's time is its last use");
    // No video there, no frames meeting's video.
    assert_eq!(
        control
            .clip(
                &id,
                3_600_000,
                3_600_000,
                Instant::now() + Duration::from_secs(4)
            )
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    let frames = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    assert_eq!(
        control
            .clip(&frames, 0, 1_000, Instant::now() + Duration::from_secs(4))
            .unwrap_err()
            .code,
        ErrorCode::NotAvailable
    );
    rig.stop(&frames);
}

fn clip_get_refuses_near_the_video_floor() {
    const GIB: u64 = 1 << 30;
    // 5 GiB free while a meeting records: the 4 GiB video floor plus the 2 GiB margin don't fit.
    let rig = Rig::with("clip-floor", &video_spec(), Some(5 * GIB));
    let id = recorded_video(&rig, 4);
    let live = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    let refused = rig
        .control()
        .clip(&id, 500, 2_500, Instant::now() + Duration::from_secs(4))
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Failed);
    assert_eq!(refused.message, "Not enough free disk space to cut a clip.");
    rig.stop(&live);

    // The clip's own input counts: one byte short of the floor and margin after it, refused.
    let roomy = Rig::with("clip-roomy", &video_spec(), Some(8 * GIB));
    let id = recorded_video(&roomy, 4);
    let live = roomy.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    let control = roomy.control();
    let input = voice_screen_files::media::clip_input_bytes(&roomy.dir(&id), 500, 2_500).unwrap();
    assert!(input > 0, "the clip reads video fragments");
    roomy.free.store(6 * GIB + input - 1, Ordering::SeqCst);
    let refused = control
        .clip(&id, 500, 2_500, Instant::now() + Duration::from_secs(4))
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Failed);
    // Exactly enough, and with 8 GiB free, it cuts, also while a meeting records.
    roomy.free.store(6 * GIB + input, Ordering::SeqCst);
    let enough = control.clip(&id, 500, 2_500, Instant::now() + Duration::from_secs(4));
    assert!(enough.is_ok(), "{enough:?}");
    roomy.free.store(8 * GIB, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let reply = control
            .clip(&id, 500, 2_500, Instant::now() + Duration::from_secs(4))
            .unwrap();
        if !reply["clips"].as_array().unwrap().is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "the clip never came");
        thread::sleep(Duration::from_millis(200));
    }
    roomy.stop(&live);
}

/// The free-space check needs the clip's input, which a long meeting on a slow disk takes a
/// while to read: `clip.get` still answers by its deadline, and the cut goes on after the check.
fn clip_get_answers_by_its_deadline_while_its_check_runs() {
    const GIB: u64 = 1 << 30;
    let rig = Rig::with("clip-slow-check", &video_spec(), Some(8 * GIB));
    let id = recorded_video(&rig, 4);
    let control = rig.control();
    rig.free_wait.store(6_000, Ordering::SeqCst);
    let asked = Instant::now();
    let reply = control
        .clip(&id, 500, 2_500, asked + Duration::from_secs(1))
        .unwrap();
    let took = asked.elapsed();
    assert!(
        took < Duration::from_secs(2),
        "clip.get answered {took:?} after it came"
    );
    assert_eq!(reply, json!({ "clips": [] }));
    rig.free_wait.store(0, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let asked = Instant::now();
        let reply = control
            .clip(&id, 500, 2_500, asked + Duration::from_secs(1))
            .unwrap();
        assert!(asked.elapsed() < Duration::from_secs(2));
        if !reply["clips"].as_array().unwrap().is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "the clip never came");
        thread::sleep(Duration::from_millis(200));
    }
}

fn screen_get_and_select() {
    let rig = Rig::new("screen-select", &format!("{SCREEN},monitors=2"));
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Frames));
    let control = rig.control();
    rig.wait_line(&id, "the capture", |l| matches!(l, Line::Capture { .. }));
    let mut described = Value::Null;
    wait_for("two monitors, recording", || {
        described = control.screen(&id, in_time()).unwrap();
        described["state"] == "recording"
            && described["monitors"]
                .as_array()
                .is_some_and(|m| m.len() == 2)
    });
    assert_eq!(described["screen"], "frames");
    assert_eq!(described["video"], false);
    assert_eq!(described["frames"], true);
    let monitors = described["monitors"].as_array().unwrap().clone();
    let current = described["monitor"].as_str().unwrap().to_string();
    let other = monitors
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .find(|m| *m != current)
        .unwrap();
    for m in &monitors {
        for field in ["id", "name", "width", "height", "primary"] {
            assert!(m.get(field).is_some(), "{field} in {m}");
        }
    }
    assert_eq!(
        control.select_monitor(&id, &other, in_time()).unwrap(),
        other
    );
    rig.wait_line(
        &id,
        "the monitor line",
        |l| matches!(l, Line::Monitor { id: m, reason, .. } if *m == other && reason == "user"),
    );
    wait_for("screen.get to say so", || {
        control.screen(&id, in_time()).unwrap()["monitor"] == other.as_str()
    });
    wait_for("the move to be reported", || {
        rig.changes()
            .iter()
            .any(|c| matches!(c, ScreenChange::Moved { .. }))
    });
    assert_eq!(
        control
            .select_monitor(&id, "\\\\.\\DISPLAY99", in_time())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        control
            .screen("m-20000101-000000-0000", in_time())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    rig.stop(&id);
    assert_eq!(
        control.screen(&id, in_time()).unwrap_err().code,
        ErrorCode::NotRecording
    );
    assert_eq!(
        control
            .select_monitor(&id, &other, in_time())
            .unwrap_err()
            .code,
        ErrorCode::NotRecording
    );
}

/// `screen.get` and `screen.select` answer by their deadline while the screen process starts: a
/// meeting Voice started waits up to 5 s for a Rukoo that was connected to come back.
fn screen_commands_answer_by_their_deadline_while_starting() {
    let rig = Rig::new("screen-deadline-starting", &format!("{SCREEN},monitors=2"));
    rig.rukoo(0, true);
    let id = rig.start(StartedBy::Voice, Some(ScreenMode::Frames));
    wait_for("meeting.starting", || !rig.starting_events(&id).is_empty());
    let control = rig.control();
    let asked = Instant::now();
    let described = control
        .screen(&id, asked + Duration::from_millis(500))
        .unwrap();
    let took = asked.elapsed();
    assert!(
        took < Duration::from_millis(1_500),
        "screen.get answered after {took:?}"
    );
    assert_eq!(described["state"], "starting");
    let asked = Instant::now();
    let refused = control
        .select_monitor(&id, "\\\\.\\SYNTHETIC2", asked + Duration::from_millis(500))
        .unwrap_err();
    let took = asked.elapsed();
    assert!(
        took < Duration::from_millis(1_500),
        "screen.select answered after {took:?}"
    );
    assert_eq!(refused.code, ErrorCode::Failed);
    rig.stop(&id);
}

/// A screen process on a real control port that answers `status` at once, or once slowly when
/// asked to, and takes 6 s to switch monitors.
struct SlowScreen {
    port: u16,
    token: String,
    slow_status: Arc<AtomicBool>,
    status_started: std::sync::mpsc::Receiver<()>,
}

impl SlowScreen {
    fn start() -> SlowScreen {
        let listener = control::bind().unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = control::new_token().unwrap();
        let (to_main, requests) = std::sync::mpsc::sync_channel::<control::Request>(8);
        control::serve(listener, token.clone(), to_main).unwrap();
        let slow_status = Arc::new(AtomicBool::new(false));
        let (started, status_started) = std::sync::mpsc::channel();
        let slow = Arc::clone(&slow_status);
        thread::spawn(move || {
            for request in requests {
                let (slow, started) = (Arc::clone(&slow), started.clone());
                thread::spawn(move || {
                    let answer = if request.body["cmd"] == "monitor" {
                        thread::sleep(Duration::from_secs(6));
                        json!({ "ok": true, "monitor": request.body["id"] })
                    } else {
                        if slow.swap(false, Ordering::SeqCst) {
                            let _ = started.send(());
                            thread::sleep(Duration::from_millis(900));
                        }
                        json!({ "ok": true, "state": "recording", "video": false, "frames": true, "monitor": "\\\\.\\SYNTHETIC1" })
                    };
                    let _ = request.reply.send(answer);
                });
            }
        });
        SlowScreen {
            port,
            token,
            slow_status,
            status_started,
        }
    }

    /// Its `screen.json`. The pid is this test's, so the rig never stops it.
    fn file(&self) -> screenfile::ScreenFile {
        screenfile::ScreenFile {
            pid: std::process::id(),
            port: self.port,
            token: self.token.clone(),
            session: 1,
            started_at: now_ms(),
        }
    }
}

/// A screen process that is slow to answer: `screen.select` still answers by its deadline, also
/// while the watchdog waits for that process's status.
fn screen_select_answers_by_its_deadline_when_the_screen_is_slow() {
    let rig = Rig::new("screen-deadline-slow", SCREEN);
    let slow = SlowScreen::start();
    // A meeting whose screen process is the stand-in, reattached as after the app restarted.
    let dir = voice_meetings::Library::open(&rig.data)
        .unwrap()
        .create(&voice_meetings::CreateMeeting {
            title: "Slow screen".into(),
            tracks: vec![Track::Mic],
            sources: Default::default(),
            screen: ScreenMode::Frames,
            started_by: StartedBy::Rukoo,
            context: None,
        })
        .unwrap();
    let id = dir.id().to_string();
    screenfile::write(dir.path(), &slow.file()).unwrap();
    rig.screen().reattach(&id);
    let control = rig.control();
    wait_for("the stand-in to record", || {
        control
            .screen(&id, Instant::now() + Duration::from_secs(4))
            .is_ok_and(|s| s["state"] == "recording")
    });

    // It takes 6 s to switch.
    let asked = Instant::now();
    let refused = control
        .select_monitor(&id, "\\\\.\\SYNTHETIC2", asked + Duration::from_secs(1))
        .unwrap_err();
    let took = asked.elapsed();
    assert!(
        took < Duration::from_millis(1_800),
        "screen.select answered after {took:?}"
    );
    assert_eq!(refused.code, ErrorCode::Failed);

    // The watchdog's next status takes 900 ms: screen.select doesn't wait for it.
    slow.slow_status.store(true, Ordering::SeqCst);
    slow.status_started
        .recv_timeout(Duration::from_secs(10))
        .expect("the watchdog asks for the status");
    let asked = Instant::now();
    let refused = control
        .select_monitor(&id, "\\\\.\\SYNTHETIC2", asked + Duration::from_millis(250))
        .unwrap_err();
    let took = asked.elapsed();
    assert!(
        took < Duration::from_millis(650),
        "screen.select waited {took:?} for the watchdog"
    );
    assert_eq!(refused.code, ErrorCode::Failed);
}

// Through the whole engine ----------------------------------------------------------------------

/// An engine whose recorder and screen process are this test binary.
fn engine_harness(name: &str, test: String) -> Harness {
    let mut h = Harness::new(name);
    h.recorder_exe = exe();
    h.screen_exe = exe();
    h.timing = controller_timing();
    h.screen_timing = screen_timing();
    h.test_sources = Some("mic=synthetic:tone:440,call=synthetic:idle".into());
    h.test_screen = Some(test);
    h
}

fn drops_reach_the_pill() {
    const GIB: u64 = 1 << 30;
    let free = TempDir::new("free-bytes");
    let file = free.join("free.txt");
    std::fs::write(&file, (10 * GIB).to_string()).unwrap();
    let h = engine_harness(
        "drops",
        format!("{SCREEN},{VIDEO},free-bytes-file={}", file.display()),
    );
    h.config(json!({"meetings_screen": "video"}));
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    let dir = h.data().join("meetings").join(&id);
    wait_for("the video", || {
        journal_has(&dir, |l| matches!(l, Line::Video { .. }))
    });
    wait_for("the pill's screen", || {
        h.sink
            .last_pill()
            .and_then(|p| p.meeting)
            .is_some_and(|m| m.screen == "video")
    });
    // Under 4 GiB the video stops; the frames go on.
    std::fs::write(&file, (3 * GIB).to_string()).unwrap();
    let notice = wait_notice(&h, "pill.videoDropped.disk");
    assert_eq!(notice.phase, PillPhase::Meeting);
    assert_eq!(notice.meeting.unwrap().screen, "frames");
    let note = h.sink.wait_note("app.notify.meeting");
    assert!(note.body.contains("Frames go on"), "{}", note.body);
    // Under 2 GiB the frames stop too; the audio never does.
    std::fs::write(&file, GIB.to_string()).unwrap();
    let notice = wait_notice(&h, "pill.framesDropped.disk");
    let meeting = notice.meeting.unwrap();
    assert_eq!(
        (meeting.screen.as_str(), meeting.screen_switch),
        ("off", false)
    );
    wait_for("the screen's stop", || {
        journal_has(
            &dir,
            |l| matches!(l, Line::Stop { reason, .. } if reason == "dropped"),
        )
    });
    // Video first, then frames.
    let drops: Vec<String> = journal::read_all(&dir)
        .unwrap()
        .into_iter()
        .filter_map(|l| match l {
            Line::Dropped { what, reason, .. } => Some(format!("{what} {reason}")),
            _ => None,
        })
        .collect();
    assert_eq!(drops, ["video disk", "frames disk"]);
    assert!(h.sink.notes().iter().any(|n| n
        .body
        .contains("stopped recording the screen to keep recording audio")));
    // The audio goes on.
    let timeline = dir.join("timeline.jsonl");
    let before = std::fs::metadata(&timeline).unwrap().len();
    wait_for("the recorder to go on", || {
        std::fs::metadata(&timeline).unwrap().len() > before
    });
    assert_eq!(engine.tray_state().meeting.as_deref(), Some("recording"));
    engine.stop_meeting(voice_engine::StopReason::User).unwrap();
    engine.shutdown();
}

/// A failed encoder drops the video and says so on the pill and in the tray, and the frames go
/// on: once on a PC without a hardware encoder, once when the encoder fails twice.
fn encoder_drops_reach_the_pill() {
    for (name, test) in [
        ("no-encoder", format!("{SCREEN},no-hardware")),
        (
            "encoder-fails",
            format!("{SCREEN},{VIDEO},encoder-fail-at=2000"),
        ),
    ] {
        println!("    {name}");
        let h = engine_harness(name, test);
        h.config(json!({"meetings_screen": "video"}));
        let engine = h.start();
        let id = engine.start_meeting().unwrap();
        let dir = h.data().join("meetings").join(&id);
        let notice = wait_notice(&h, "pill.videoDropped.encoder");
        assert_eq!(notice.phase, PillPhase::Meeting);
        assert_eq!(notice.meeting.unwrap().screen, "frames");
        let note = h.sink.wait_note("app.notify.meeting");
        assert!(note.body.contains("frames only"), "{}", note.body);
        let drops: Vec<String> = journal::read_all(&dir)
            .unwrap()
            .into_iter()
            .filter_map(|l| match l {
                Line::Dropped { what, reason, .. } => Some(format!("{what} {reason}")),
                _ => None,
            })
            .collect();
        assert_eq!(drops, ["video encoder"]);
        let frames_before = journal::read_all(&dir)
            .unwrap()
            .iter()
            .filter(|l| matches!(l, Line::Frame { .. }))
            .count();
        wait_for("frames after the drop", || {
            journal::read_all(&dir)
                .unwrap_or_default()
                .iter()
                .filter(|l| matches!(l, Line::Frame { .. }))
                .count()
                > frames_before
        });
        engine.stop_meeting(voice_engine::StopReason::User).unwrap();
        engine.shutdown();
    }
}

fn journal_has(dir: &Path, pick: impl Fn(&Line) -> bool) -> bool {
    journal::read_all(dir).unwrap_or_default().iter().any(pick)
}

/// Waits for a pill snapshot with this notice.
fn wait_notice(h: &Harness, key: &str) -> voice_engine::PillSnapshot {
    let mut found = None;
    wait_for(&format!("the pill's {key}"), || {
        found = lock(&h.sink.pills)
            .iter()
            .rev()
            .find(|p| p.notice.as_ref().is_some_and(|n| n.key == key))
            .cloned();
        found.is_some()
    });
    found.unwrap()
}

fn pill_screen_click_moves_to_the_next_monitor() {
    let h = engine_harness("pill-switch", format!("{SCREEN},monitors=2"));
    h.config(json!({"meetings_screen": "frames"}));
    let engine = h.start();
    // The probe tells Settings what this computer can record.
    wait_for("the probe's answer in Settings", || {
        !engine.settings_state()["screen"].is_null()
    });
    let screen = engine.settings_state()["screen"].clone();
    assert_eq!(screen["available"], true, "{screen}");
    assert!(screen["monitors"].as_u64().unwrap() >= 1, "{screen}");
    assert!(screen["ocr"].is_array(), "{screen}");
    assert!(screen.get("hardwareEncoder").is_some(), "{screen}");
    let id = engine.start_meeting().unwrap();
    let dir = h.data().join("meetings").join(&id);
    wait_for("the screen button", || {
        h.sink
            .last_pill()
            .and_then(|p| p.meeting)
            .is_some_and(|m| m.screen == "frames" && m.screen_switch)
    });
    let mut first = None;
    wait_for("the first monitor line", || {
        first = journal::read_all(&dir)
            .unwrap_or_default()
            .into_iter()
            .find_map(|l| match l {
                Line::Monitor { id, .. } => Some(id),
                _ => None,
            });
        first.is_some()
    });
    let first = first.unwrap();
    engine.pill_screen_clicked();
    wait_for("the screen menu", || !lock(&h.sink.screen_menus).is_empty());
    let info = lock(&h.sink.screen_menus).last().cloned().unwrap();
    assert_eq!(info["meeting"], id);
    let other = info["monitors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] != first)
        .unwrap()["id"]
        .as_str()
        .unwrap();
    engine.pill_screen_set(&id, "frames", Some(other)).unwrap();
    wait_for("a monitor line from the user", || {
        journal::read_all(&dir).unwrap_or_default().iter().any(
            |l| matches!(l, Line::Monitor { id, reason, .. } if *id != first && reason == "user"),
        )
    });
    let moved = wait_notice(&h, "pill.screenMoved");
    assert!(moved.notice.unwrap().args["name"].is_string());
    engine.pill_screen_set(&id, "off", None).unwrap();
    wait_for("Off in the pill", || {
        h.sink
            .last_pill()
            .and_then(|p| p.meeting)
            .is_some_and(|m| m.screen == "off" && m.screen_switch)
    });
    engine.pill_screen_set(&id, "frames", Some(&first)).unwrap();
    wait_for("back to the first monitor", || {
        journal::read_all(&dir)
            .unwrap_or_default()
            .iter()
            .filter(|l| matches!(l, Line::Monitor { reason, .. } if reason == "user"))
            .count()
            == 2
    });
    let modes: Vec<_> = journal::read_all(&dir)
        .unwrap()
        .into_iter()
        .filter_map(|line| match line {
            Line::Mode { mode, .. } => Some(mode),
            _ => None,
        })
        .collect();
    assert_eq!(
        modes,
        ["off", "frames"],
        "monitor-only changes do not add screen-mode marks"
    );
    engine.stop_meeting(voice_engine::StopReason::User).unwrap();
    engine.shutdown();
}

fn screen_mode_changes_require_each_clients_current_ack() {
    let rig = Rig::new("mode-protection", &format!("{SCREEN},{VIDEO}"));
    let id = rig.start(StartedBy::Rukoo, Some(ScreenMode::Off));
    let screen = rig.screen();
    // These clients connect only after the inert process has started. Protection must include
    // them rather than using a client list from before the potentially slow process startup.
    let screen_file = rig.dir(&id).join("screen").join("screen.json");
    screen.set_protection_clients(Box::new(move || {
        if screen_file.is_file() {
            vec![11, 22]
        } else {
            Vec::new()
        }
    }));
    let change = {
        let screen = Arc::clone(&screen);
        let id = id.clone();
        thread::spawn(move || {
            screen.set_mode(
                &id,
                ScreenMode::Frames,
                None,
                Instant::now() + Duration::from_secs(15),
            )
        })
    };
    let mut request = None;
    wait_for("a mode protection request", || {
        request = rig
            .starting_events(&id)
            .last()
            .and_then(|e| e.data["request"].as_str())
            .map(str::to_owned);
        request.is_some()
    });
    let request = request.unwrap();
    screen.ack_mode(&id, "old-round", 22);
    screen.ack_mode(&id, &request, 11);
    screen.ack_mode(&id, &request, 11);
    thread::sleep(Duration::from_millis(100));
    assert!(
        !change.is_finished(),
        "duplicate or stale acknowledgements cannot protect the second client"
    );
    assert!(
        !rig.journal(&id).iter().any(|line| matches!(
            line,
            Line::Capture { .. } | Line::Frame { .. } | Line::Video { .. }
        )),
        "no capture before every client protects its windows"
    );
    screen.ack_mode(&id, &request, 22);
    assert_eq!(change.join().unwrap().unwrap()["screen"], "frames");
    rig.wait_line(&id, "capture after protection", |l| {
        matches!(l, Line::Frame { .. })
    });
    screen
        .set_mode(
            &id,
            ScreenMode::Off,
            None,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(screen.mode_of(&id), ScreenMode::Off);
    let before = rig
        .journal(&id)
        .iter()
        .filter(|l| matches!(l, Line::Frame { .. }))
        .count();
    // Nobody acknowledges this new round: it must leave capture Off.
    let error = screen
        .set_mode(
            &id,
            ScreenMode::Video,
            None,
            Instant::now() + Duration::from_secs(3),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Failed);
    assert_eq!(screen.mode_of(&id), ScreenMode::Off);
    assert_eq!(
        rig.journal(&id)
            .iter()
            .filter(|l| matches!(l, Line::Frame { .. }))
            .count(),
        before
    );
    rig.stop(&id);
}
