//! The screen supervisor: runs `rukoo-voice-screen` for the meeting that records the screen, and
//! keeps it running (voice/RECORDING.md, "The screen folder").
//!
//! The screen process is detached like the recorder, so a graphics driver, encoder or OCR crash
//! never costs audio, and the app can crash or quit while it records. The supervisor starts it
//! once the recorder runs, after the start handshake with Rukoo for a meeting Voice started
//! itself; watches it over its control port; restarts it with `--resume` when it dies (up to five
//! times, with backoff) and finishes its journal when it gives up; reattaches to it after the app
//! restarts; forwards pause, resume and monitor switches; and stops it when the meeting ends. It
//! never stops a screen process because the app is quitting, and it never touches the recorder.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use voice_bridge::{ControlError, EventLog};
use voice_meetings::Library;
use voice_protocol::{events, ScreenMode, StartedBy};
use voice_screen_files::caps::Capabilities;
use voice_screen_files::journal::{self, JournalReader, Line};
use voice_screen_files::monitors::{self, MonitorInfo};
use voice_screen_files::policy::{Part, Reason};
use voice_screen_files::screenfile::{self, ScreenFile};
use voice_screen_files::{control, paths};

use super::controller::spawn_detached;

#[cfg(windows)]
pub const SCREEN_EXE: &str = "rukoo-voice-screen.exe";
#[cfg(not(windows))]
pub const SCREEN_EXE: &str = "rukoo-voice-screen";

/// The screen process's exit codes, as the recorder's (voice/RECORDING.md).
const EXIT_CONFLICT: i32 = 3;
const EXIT_BUSY: i32 = 4;
/// How long a control request may take.
const REQUEST_WAIT: Duration = Duration::from_secs(1);
/// How long `stop` over the control port may take: it finalizes the video and drains OCR.
const STOP_WAIT: Duration = Duration::from_secs(5);
/// How long `probe` and `finalize` may run.
const TOOL_WAIT: Duration = Duration::from_secs(30);
/// How long a monitor switch waits for a screen process that is starting, at most: a command
/// from Rukoo waits only until its deadline, 4 s after Voice read it.
const STARTING_PATIENCE: Duration = Duration::from_secs(3);
/// How long the pill's monitor switch waits for the screen process to answer.
const SWITCH_WAIT: Duration = Duration::from_secs(5);

/// How the supervisor waits and retries. The defaults are for real use; tests shorten them.
#[derive(Clone, Debug)]
pub struct ScreenTiming {
    /// How often the watchdog asks the screen process for its status.
    pub poll: Duration,
    /// How long a starting screen process may take to answer on its control port.
    pub start_timeout: Duration,
    /// Waits before each restart; their number is the most restarts in a row.
    pub backoff: Vec<Duration>,
    /// A screen process that has run this long counts as healthy again.
    pub stable_after: Duration,
    /// Status checks in a row that may fail before one without a process handle counts as dead.
    pub ping_failures: u32,
    /// How long a meeting Voice started waits for every connected Rukoo to hide its windows.
    pub handshake: Duration,
    /// How long the end of a meeting waits for the screen process to stop on its own.
    pub end_wait: Duration,
    /// How long a meeting Voice started waits for a Rukoo that was connected earlier to return.
    pub reconnect: Duration,
}

impl Default for ScreenTiming {
    fn default() -> ScreenTiming {
        ScreenTiming {
            poll: Duration::from_millis(500),
            start_timeout: Duration::from_secs(10),
            backoff: [500, 1000, 2000, 4000, 8000]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
            stable_after: Duration::from_secs(60),
            ping_failures: 3,
            handshake: voice_protocol::STARTING_WAIT,
            end_wait: Duration::from_secs(10),
            reconnect: Duration::from_secs(5),
        }
    }
}

/// Free bytes on the volume of a path, for the clip cut's reserve check.
pub type FreeBytes = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

/// How many Rukoo clients are connected now, and whether one completed `hello` earlier in this
/// app run.
pub type RukooClients = Box<dyn Fn() -> (usize, bool) + Send + Sync>;
pub type ProtectionClients = Box<dyn Fn() -> Vec<u64> + Send + Sync>;
/// The pids whose windows the screen process leaves out, as `--exclude-pid`.
pub type ExcludePids = Box<dyn Fn() -> Vec<u32> + Send + Sync>;

pub struct ScreenOptions {
    pub data_dir: PathBuf,
    pub exe: PathBuf,
    /// Development only: `RUKOO_VOICE_TEST_SCREEN`, passed as `--test`.
    pub test: Option<String>,
    /// Development only: `RUKOO_VOICE_TEST_MONITOR`, passed as `--monitor`.
    pub test_monitor: Option<String>,
    pub timing: ScreenTiming,
    /// The free space a clip cut may use; `None` doesn't check.
    pub free_bytes: Option<FreeBytes>,
}

/// What the supervisor tells the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum ScreenChange {
    Mode {
        id: String,
    },
    /// A part of the screen recording stopped for the rest of the meeting.
    Dropped {
        id: String,
        what: Part,
        reason: Reason,
    },
    /// The screen recording moved to another monitor.
    Moved {
        id: String,
        name: String,
    },
    /// The screen process died five times in a row; the screen isn't recorded any more.
    GaveUp {
        id: String,
    },
    /// The number of monitors is known or changed.
    Monitors {
        count: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The start handshake runs; nothing launched yet.
    Starting,
    Running,
    Restarting,
    /// The meeting ended; the supervisor waits for the process to stop.
    Ending,
    /// The screen recording is over for this meeting: stopped, dropped or given up.
    Stopped,
}

struct ScreenLive {
    id: String,
    dir: PathBuf,
    mode: ScreenMode,
    phase: Phase,
    child: Option<Child>,
    file: Option<ScreenFile>,
    restarts: u32,
    alive_since: Instant,
    failed_pings: u32,
    /// The newest status the process answered.
    status: Option<Value>,
    /// The journal, read as it grows, for drops, monitor moves and `stop`.
    journal: JournalReader,
    dropped_video: Option<Reason>,
    dropped_frames: Option<Reason>,
    /// The `stop` line's reason, once the journal has one.
    stopped: Option<String>,
    gave_up: bool,
    monitors: Option<usize>,
}

impl ScreenLive {
    fn new(id: &str, dir: &Path, mode: ScreenMode, phase: Phase) -> ScreenLive {
        ScreenLive {
            id: id.to_string(),
            dir: dir.to_path_buf(),
            mode,
            phase,
            child: None,
            file: None,
            restarts: 0,
            alive_since: Instant::now(),
            failed_pings: 0,
            status: None,
            journal: JournalReader::new(dir),
            dropped_video: None,
            dropped_frames: None,
            stopped: None,
            gave_up: false,
            monitors: None,
        }
    }

    /// Reads the journal's new lines. Drops and moves are reported when `report`, else only noted
    /// (for lines from before the app restarted).
    fn read_journal(&mut self, report: bool) -> Vec<ScreenChange> {
        let mut changes = Vec::new();
        let lines = self.journal.poll().unwrap_or_default();
        for line in lines {
            match line {
                Line::Mode { mode, .. } => {
                    if let Some(mode) = ScreenMode::parse(&mode) {
                        self.mode = mode;
                        if report {
                            changes.push(ScreenChange::Mode {
                                id: self.id.clone(),
                            });
                        }
                    }
                }
                Line::Dropped { what, reason, .. } => {
                    let (Some(part), Some(reason)) = (Part::parse(&what), Reason::parse(&reason))
                    else {
                        continue;
                    };
                    let slot = match part {
                        Part::Video => &mut self.dropped_video,
                        Part::Frames => &mut self.dropped_frames,
                    };
                    if slot.is_none() {
                        *slot = Some(reason);
                        if report {
                            changes.push(ScreenChange::Dropped {
                                id: self.id.clone(),
                                what: part,
                                reason,
                            });
                        }
                    }
                }
                Line::Monitor { name, reason, .. } if reason == "user" || reason == "lost" => {
                    if report {
                        changes.push(ScreenChange::Moved {
                            id: self.id.clone(),
                            name,
                        });
                    }
                }
                Line::Stop { reason, .. } => self.stopped = Some(reason),
                _ => {}
            }
        }
        // Video and frames dropped together, as a full disk or a device that never came back
        // does: the frames' notice says the whole screen recording stopped, and a video notice
        // saying the frames go on would contradict it.
        let frames_dropped = changes.iter().any(|c| {
            matches!(
                c,
                ScreenChange::Dropped {
                    what: Part::Frames,
                    ..
                }
            )
        });
        if frames_dropped {
            changes.retain(|c| {
                !matches!(
                    c,
                    ScreenChange::Dropped {
                        what: Part::Video,
                        ..
                    }
                )
            });
        }
        changes
    }

    /// Whether the screen is still recorded: not stopped, dropped or given up.
    fn recording(&self) -> bool {
        self.mode != ScreenMode::Off
            && self.phase != Phase::Stopped
            && !self.gave_up
            && self.dropped_frames.is_none()
            && self.stopped.is_none()
    }

    /// What the pill shows: the mode it records now.
    fn shown_mode(&self) -> ScreenMode {
        if !self.recording() {
            ScreenMode::Off
        } else if self.mode == ScreenMode::Video && self.dropped_video.is_some() {
            ScreenMode::Frames
        } else {
            self.mode
        }
    }
}

/// The live meeting's screen recording, as `screen.get` and `screen.select` read it.
struct LiveView {
    mode: ScreenMode,
    phase: Phase,
    file: Option<ScreenFile>,
    /// The newest status the process answered.
    status: Option<Value>,
    /// Whether each part still records.
    video: bool,
    frames: bool,
}

#[derive(Debug)]
struct LaunchError {
    code: Option<i32>,
    message: String,
}

pub struct ScreenSupervisor {
    lib: Library,
    logs: PathBuf,
    exe: PathBuf,
    test: Option<String>,
    test_monitor: Option<String>,
    timing: ScreenTiming,
    free_bytes: Option<FreeBytes>,
    log: Option<EventLog>,
    slot: Mutex<Option<ScreenLive>>,
    changed: Condvar,
    acks: Mutex<HashMap<String, u32>>,
    acked: Condvar,
    clients: Mutex<Option<RukooClients>>,
    protection_clients: Mutex<Option<ProtectionClients>>,
    exclude_pids: Mutex<Option<ExcludePids>>,
    mode_change: Mutex<()>,
    mode_acks: Mutex<Option<ModeProtection>>,
    mode_acked: Condvar,
    caps: OnceLock<Capabilities>,
    on_change: Box<dyn Fn(ScreenChange) + Send + Sync>,
    stopped: AtomicBool,
    watchdog: Mutex<Option<JoinHandle<()>>>,
}

struct ModeProtection {
    meeting: String,
    request: String,
    pending: HashSet<u64>,
    acked: HashSet<u64>,
}

impl ScreenSupervisor {
    pub fn new(
        opts: ScreenOptions,
        log: Option<EventLog>,
        on_change: impl Fn(ScreenChange) + Send + Sync + 'static,
    ) -> io::Result<Arc<ScreenSupervisor>> {
        let lib = Library::open(&opts.data_dir)?;
        let supervisor = Arc::new(ScreenSupervisor {
            lib,
            logs: voice_core::paths::logs_dir(&opts.data_dir),
            exe: opts.exe,
            test: opts.test,
            test_monitor: opts.test_monitor,
            timing: opts.timing,
            free_bytes: opts.free_bytes,
            log,
            slot: Mutex::new(None),
            changed: Condvar::new(),
            acks: Mutex::new(HashMap::new()),
            acked: Condvar::new(),
            clients: Mutex::new(None),
            protection_clients: Mutex::new(None),
            exclude_pids: Mutex::new(None),
            mode_change: Mutex::new(()),
            mode_acks: Mutex::new(None),
            mode_acked: Condvar::new(),
            caps: OnceLock::new(),
            on_change: Box::new(on_change),
            stopped: AtomicBool::new(false),
            watchdog: Mutex::new(None),
        });
        let weak = Arc::downgrade(&supervisor);
        let thread = thread::Builder::new()
            .name("screen-watchdog".into())
            .spawn(move || watch(weak))?;
        *lock(&supervisor.watchdog) = Some(thread);
        Ok(supervisor)
    }

    /// How many Rukoo clients are connected now, and whether one completed `hello` earlier in
    /// this app run. Without it, the handshake never waits.
    pub fn set_rukoo_clients(&self, clients: RukooClients) {
        *lock(&self.clients) = Some(clients);
    }

    pub fn set_protection_clients(&self, clients: ProtectionClients) {
        *lock(&self.protection_clients) = Some(clients);
    }

    /// The pids the screen process leaves out of its capture. A launch reads them; a screen
    /// process that already records learns nothing new (a Rukoo that reconnects mid-meeting with
    /// another pid isn't followed yet).
    pub fn set_exclude_pids(&self, pids: ExcludePids) {
        *lock(&self.exclude_pids) = Some(pids);
    }

    fn protection_clients(&self) -> HashSet<u64> {
        lock(&self.protection_clients)
            .as_ref()
            .map_or_else(HashSet::new, |clients| clients().into_iter().collect())
    }

    fn exclude_pids(&self) -> Vec<u32> {
        lock(&self.exclude_pids)
            .as_ref()
            .map_or_else(Vec::new, |pids| pids())
    }

    pub fn ack_mode(&self, id: &str, request: &str, connection: u64) {
        let mut round = lock(&self.mode_acks);
        if let Some(round) = round
            .as_mut()
            .filter(|round| round.meeting == id && round.request == request)
        {
            if round.pending.remove(&connection) {
                round.acked.insert(connection);
            }
        }
        self.mode_acked.notify_all();
    }

    fn protect_mode(
        &self,
        id: &str,
        mode: ScreenMode,
        deadline: Instant,
    ) -> Result<(), ControlError> {
        let clients = self.protection_clients();
        if clients.is_empty() {
            return Ok(());
        }
        let log = self
            .log
            .as_ref()
            .ok_or_else(|| ControlError::failed("Voice cannot request window protection."))?;
        let request =
            voice_protocol::new_instance().map_err(|e| ControlError::failed(e.to_string()))?;
        *lock(&self.mode_acks) = Some(ModeProtection {
            meeting: id.into(),
            request: request.clone(),
            pending: clients,
            acked: HashSet::new(),
        });
        let until =
            deadline.min(Instant::now() + self.timing.handshake.max(Duration::from_secs(2)));
        let result = (|| {
            log.append(
                events::MEETING_STARTING,
                Some(id),
                json!({ "screen": mode, "request": request }),
            )
            .map_err(|e| ControlError::failed(e.to_string()))?;
            loop {
                let connected = self.protection_clients();
                let mut locked = lock(&self.mode_acks);
                let round = locked.as_mut().expect("mode changes are serialized");
                round
                    .pending
                    .retain(|connection| connected.contains(connection));
                let joined: Vec<u64> = connected
                    .difference(&round.acked)
                    .filter(|connection| !round.pending.contains(connection))
                    .copied()
                    .collect();
                if !joined.is_empty() {
                    round.pending.extend(joined);
                    drop(locked);
                    // A client's welcome may include the previous announcement as history.
                    log.append(
                        events::MEETING_STARTING,
                        Some(id),
                        json!({ "screen": mode, "request": request }),
                    )
                    .map_err(|e| ControlError::failed(e.to_string()))?;
                    continue;
                }
                if round.pending.is_empty() {
                    return Ok(());
                }
                if Instant::now() >= until {
                    return Err(ControlError::failed("Rukoo could not protect all its windows. The screen recording was not changed."));
                }
                let _wait = self
                    .mode_acked
                    .wait_timeout(locked, Duration::from_millis(25))
                    .unwrap_or_else(PoisonError::into_inner);
            }
        })();
        *lock(&self.mode_acks) = None;
        result
    }

    pub fn set_mode(
        self: &Arc<Self>,
        id: &str,
        mode: ScreenMode,
        monitor: Option<&str>,
        deadline: Instant,
    ) -> Result<Value, ControlError> {
        let _change = loop {
            if let Ok(change) = self.mode_change.try_lock() {
                break change;
            }
            if Instant::now() >= deadline {
                return Err(ControlError::failed(
                    "Another screen change is still in progress.",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        if Instant::now() >= deadline {
            return Err(ControlError::failed("The screen change deadline elapsed."));
        }
        let dir = self
            .lib
            .dir(id)
            .ok_or_else(|| ControlError::not_found("There is no meeting with that id."))?;
        if !self.recording_now(dir.path()) {
            return Err(ControlError::not_recording(
                "The meeting does not record now.",
            ));
        }
        if !self.available() {
            return Err(ControlError::not_available(
                "This computer cannot record the screen.",
            ));
        }
        let needs_launch = self.lock().as_ref().is_none_or(|live| live.id != id);
        if needs_launch {
            if mode == ScreenMode::Off && monitor.is_none() {
                return Ok(json!({ "screen": mode }));
            }
            *self.lock() = Some(ScreenLive::new(
                id,
                dir.path(),
                ScreenMode::Off,
                Phase::Starting,
            ));
            // Start inert, so a requested monitor is selected before the first capture.
            self.first_launch(
                id,
                dir.path(),
                ScreenMode::Off,
                journal_has_content(dir.path()),
            );
        }
        let view = self.live_of(id, deadline)?;
        let file = view
            .file
            .filter(|_| view.phase == Phase::Running)
            .ok_or_else(|| ControlError::not_available("The screen recorder is not available."))?;
        if !self.recording_now(dir.path()) {
            return Err(ControlError::not_recording(
                "The meeting stopped while the screen recorder was starting.",
            ));
        }
        // Starting the inert process can take seconds. Include clients that connected during
        // that startup, then acknowledge protection immediately before enabling capture.
        if mode != ScreenMode::Off {
            self.protect_mode(id, mode, deadline)?;
        }
        let reply = request_until(
            &file,
            json!({ "cmd": "mode", "mode": mode, "monitor": monitor }),
            deadline,
        )
        .map_err(|e| ControlError::failed(e.to_string()))?;
        if reply["ok"] != true {
            return Err(ControlError::not_available(
                "The screen recording could not be changed.",
            ));
        }
        let (changes, actual) = {
            let mut slot = self.lock();
            let live = slot
                .as_mut()
                .filter(|live| live.id == id)
                .ok_or_else(|| ControlError::not_recording("The meeting stopped."))?;
            let changes = live.read_journal(true);
            live.mode = mode;
            live.status = None;
            (changes, live.shown_mode())
        };
        self.emit_all(changes);
        Ok(json!({ "screen": actual }))
    }

    fn rukoo_clients(&self) -> (usize, bool) {
        lock(&self.clients).as_ref().map_or((0, false), |f| f())
    }

    fn emit(&self, change: ScreenChange) {
        (self.on_change)(change);
    }

    fn emit_all(&self, changes: Vec<ScreenChange>) {
        for change in changes {
            self.emit(change);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<ScreenLive>> {
        lock(&self.slot)
    }

    // Capabilities ---------------------------------------------------------------------------

    /// Runs `rukoo-voice-screen probe` once on a thread of its own and keeps what it says. Ends
    /// with [`ScreenChange::Monitors`], so Settings and the pill can show it.
    pub fn probe_async(self: &Arc<Self>) {
        if !self.exe.is_file() || self.caps.get().is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        let _ = thread::Builder::new()
            .name("screen-probe".into())
            .spawn(move || {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                match this.run_probe() {
                    Ok(caps) => {
                        let count = caps.monitors.len();
                        let _ = this.caps.set(caps);
                        this.emit(ScreenChange::Monitors { count });
                    }
                    Err(e) => log::warn!("screen: probe failed: {e}"),
                }
            });
    }

    fn run_probe(&self) -> Result<Capabilities, String> {
        let mut command = Command::new(&self.exe);
        command
            .arg("probe")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(self.log_file("screen-probe.log"));
        no_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("{} couldn't run: {e}", self.exe.display()))?;
        let mut stdout = child.stdout.take().ok_or("no stdout")?;
        let reader = thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            out
        });
        let status = wait_child(&mut child, TOOL_WAIT).ok_or("it didn't answer in time")?;
        let out = reader.join().unwrap_or_default();
        if !status.success() {
            return Err(format!("it exited with {status}"));
        }
        serde_json::from_slice(&out).map_err(|e| format!("its answer didn't parse: {e}"))
    }

    pub fn capabilities(&self) -> Option<Capabilities> {
        self.caps.get().cloned()
    }

    /// Whether this computer can record the screen: the screen program is there, and its probe
    /// didn't say capture is unsupported.
    pub fn available(&self) -> bool {
        self.exe.is_file() && self.caps.get().is_none_or(|caps| caps.capture)
    }

    // Lifecycle ------------------------------------------------------------------------------

    /// At start, after the controller's: finishes the screen journal of every meeting that
    /// doesn't record any more and stops a screen process still recording into one. The live
    /// meeting is reattached by [`ScreenSupervisor::reattach`].
    pub fn recover(&self, live: Option<&str>) {
        for dir in self.lib.dirs() {
            let id = dir.id().as_str();
            if Some(id) == live {
                continue;
            }
            if !paths::journal_path(dir.path()).is_file() {
                continue;
            }
            let Some(history) = journal_history(dir.path()) else {
                continue;
            };
            if history.stopped.is_some() {
                continue;
            }
            match screenfile::live(dir.path()) {
                // Its meeting stopped; the process stops at the timeline's stop on its own, and
                // this makes sure.
                Some(file) => {
                    let _ = request(&file, json!({"cmd": "stop"}), STOP_WAIT);
                }
                None => {
                    if let Err(e) = self.finalize(id, dir.path(), "interrupted") {
                        log::warn!("screen: {id}: couldn't finish the screen journal: {e}");
                    }
                }
            }
        }
    }

    /// A meeting started: launches the screen process on its own thread, after the start
    /// handshake when Voice started the meeting (voice/PROTOCOL.md, `meeting.starting`). Audio
    /// never waits for any of this.
    pub fn begin(self: &Arc<Self>, id: &str) {
        let Some(dir) = self.lib.dir(id) else {
            return;
        };
        let Ok(meeting) = dir.meeting() else {
            return;
        };
        if meeting.screen == ScreenMode::Off || !self.exe.is_file() {
            return;
        }
        {
            let mut slot = self.lock();
            if slot.as_ref().is_some_and(|l| l.id == id) {
                return;
            }
            *slot = Some(ScreenLive::new(
                id,
                dir.path(),
                meeting.screen,
                Phase::Starting,
            ));
        }
        let this = Arc::clone(self);
        let id = id.to_string();
        let path = dir.path().to_path_buf();
        let by = meeting.started_by;
        let mode = meeting.screen;
        let _ = thread::Builder::new()
            .name("screen-begin".into())
            .spawn(move || {
                if by == StartedBy::Voice {
                    this.handshake(&id, mode);
                }
                this.first_launch(&id, &path, mode, false);
            });
    }

    /// The meeting's recorder was reattached after the app restarted: reattaches its screen
    /// process, or relaunches it with `--resume` when it died meanwhile.
    pub fn reattach(self: &Arc<Self>, id: &str) {
        let Some(dir) = self.lib.dir(id) else {
            return;
        };
        let Ok(meeting) = dir.meeting() else {
            return;
        };
        if meeting.screen == ScreenMode::Off && !paths::journal_path(dir.path()).is_file() {
            return;
        }
        let mut live = ScreenLive::new(id, dir.path(), meeting.screen, Phase::Running);
        // What happened before the app restarted is known, not news.
        live.read_journal(false);
        let running = screenfile::live(dir.path());
        {
            let mut slot = self.lock();
            if slot.as_ref().is_some_and(|l| l.id == id) {
                return;
            }
            if live.stopped.is_some() {
                live.phase = Phase::Stopped;
                *slot = Some(live);
                return;
            }
            if let Some(file) = running {
                live.file = Some(file);
                *slot = Some(live);
                drop(slot);
                self.changed.notify_all();
                return;
            }
            if !self.exe.is_file() {
                return;
            }
            live.phase = Phase::Starting;
            *slot = Some(live);
        }
        let this = Arc::clone(self);
        let id = id.to_string();
        let path = dir.path().to_path_buf();
        let resume = journal_has_content(&path);
        let _ = thread::Builder::new()
            .name("screen-begin".into())
            .spawn(move || {
                let mode = journal_history(&path)
                    .and_then(|h| h.mode)
                    .and_then(|m| ScreenMode::parse(&m))
                    .unwrap_or(meeting.screen);
                if mode != ScreenMode::Off {
                    this.handshake(&id, mode);
                }
                this.first_launch(&id, &path, mode, resume);
            });
    }

    /// Launches the first screen process of a meeting, unless the meeting ended meanwhile. A
    /// launch that fails is handled by the watchdog, like a process that died.
    fn first_launch(&self, id: &str, dir: &Path, mode: ScreenMode, resume: bool) {
        {
            let mut slot = self.lock();
            match slot.as_mut() {
                // The end of the meeting waits for a launch under way, as for a restart.
                Some(live) if live.id == id && live.phase == Phase::Starting => {
                    live.phase = Phase::Restarting;
                }
                _ => return,
            }
        }
        let result = self.launch(id, dir, mode, resume);
        let mut slot = self.lock();
        let Some(live) = slot
            .as_mut()
            .filter(|l| l.id == id && l.phase == Phase::Restarting)
        else {
            return;
        };
        live.phase = Phase::Running;
        live.failed_pings = 0;
        match result {
            Ok((child, file)) => {
                live.child = Some(child);
                live.file = Some(file);
                live.alive_since = Instant::now();
            }
            Err(e) if e.code == Some(EXIT_BUSY) => {
                live.file = screenfile::live(dir);
            }
            // The watchdog's next round counts it as a process that died.
            Err(e) => log::warn!(
                "screen: {id}: the screen process didn't start: {}",
                e.message
            ),
        }
        drop(slot);
        self.changed.notify_all();
    }

    /// The start handshake: `meeting.starting`, then a wait for every connected Rukoo's
    /// `meeting.protected`, at most [`ScreenTiming::handshake`]. A Rukoo seen earlier in this
    /// app run but not connected now gets up to [`ScreenTiming::reconnect`] to come back, then a
    /// fresh `meeting.starting` and its own wait.
    fn handshake(&self, id: &str, mode: ScreenMode) {
        let Some(log) = &self.log else {
            return;
        };
        let (connected, seen) = self.rukoo_clients();
        if connected == 0 && !seen {
            return;
        }
        if !self.announce(log, id, mode) {
            return;
        }
        if connected > 0 {
            self.wait_acks(id, connected);
            return;
        }
        let deadline = Instant::now() + self.timing.reconnect;
        let back = loop {
            if self.ending(id) {
                return;
            }
            let (connected, _) = self.rukoo_clients();
            if connected > 0 {
                break connected;
            }
            if Instant::now() >= deadline {
                lock(&self.acks).remove(id);
                return;
            }
            thread::sleep(Duration::from_millis(25));
        };
        // The replay up to its `welcome` holds the first `meeting.starting`, which Rukoo treats
        // as history; this one reaches it live.
        if self.announce(log, id, mode) {
            self.wait_acks(id, back);
        }
    }

    /// Clears the meeting's acks and appends `meeting.starting`.
    fn announce(&self, log: &EventLog, id: &str, mode: ScreenMode) -> bool {
        lock(&self.acks).insert(id.to_string(), 0);
        match log.append(
            events::MEETING_STARTING,
            Some(id),
            json!({ "screen": mode }),
        ) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("screen: {id}: meeting.starting wasn't logged: {e}");
                lock(&self.acks).remove(id);
                false
            }
        }
    }

    fn wait_acks(&self, id: &str, wanted: usize) {
        let deadline = Instant::now() + self.timing.handshake;
        let mut acks = lock(&self.acks);
        loop {
            let got = acks.get(id).copied().unwrap_or(0) as usize;
            let now = Instant::now();
            if got >= wanted || now >= deadline {
                break;
            }
            acks = self
                .acked
                .wait_timeout(acks, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        acks.remove(id);
    }

    /// `meeting.protected` from a Rukoo.
    pub fn ack(&self, id: &str) {
        let mut acks = lock(&self.acks);
        if let Some(count) = acks.get_mut(id) {
            *count += 1;
        }
        drop(acks);
        self.acked.notify_all();
    }

    fn ending(&self, id: &str) -> bool {
        !self
            .lock()
            .as_ref()
            .is_some_and(|l| l.id == id && l.phase == Phase::Starting)
    }

    /// Forwards a pause or resume to the screen process at once. The timeline says the same a
    /// moment later, and a restarted process follows the timeline.
    pub fn pause(&self, id: &str, paused: bool) {
        let file = {
            let slot = self.lock();
            match slot.as_ref() {
                Some(live) if live.id == id && live.phase == Phase::Running => live.file.clone(),
                _ => None,
            }
        };
        if let Some(file) = file {
            let cmd = if paused { "pause" } else { "resume" };
            if let Err(e) = request(&file, json!({ "cmd": cmd }), REQUEST_WAIT) {
                log::warn!("screen: {id}: {cmd} didn't reach the screen process: {e}");
            }
        }
    }

    /// The meeting ended. Waits for the screen process to stop at the timeline's `stop`, asks
    /// it to stop after [`ScreenTiming::end_wait`], and finishes its journal when it is gone
    /// without a `stop` line. Blocks up to `end_wait` plus 10 s: call it off the event thread.
    pub fn end(&self, id: &str) {
        let deadline = Instant::now() + self.timing.end_wait;
        {
            let mut slot = self.lock();
            // A restart in progress finishes first, so its process isn't lost.
            while slot
                .as_ref()
                .is_some_and(|l| l.id == id && l.phase == Phase::Restarting)
                && Instant::now() < deadline
            {
                slot = self
                    .changed
                    .wait_timeout(slot, Duration::from_millis(100))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            match slot.as_mut() {
                Some(live) if live.id == id => match live.phase {
                    // Nothing launched yet: the begin thread sees this and doesn't launch. No
                    // screen process will write the journal's `stop`, so this does, or the
                    // meeting would wait `SCREEN_STOP_WAIT` before it is done.
                    Phase::Starting => {
                        let taken = slot.take();
                        drop(slot);
                        self.changed.notify_all();
                        self.acked.notify_all();
                        if let Some(mut live) = taken {
                            match self.finalize(id, &live.dir, "interrupted") {
                                Ok(()) => {
                                    let changes = live.read_journal(true);
                                    self.emit_all(changes);
                                }
                                Err(e) => log::warn!(
                                    "screen: {id}: couldn't finish the screen journal: {e}"
                                ),
                            }
                        }
                        return;
                    }
                    Phase::Stopped => {
                        *slot = None;
                        return;
                    }
                    _ => live.phase = Phase::Ending,
                },
                _ => return,
            }
        }
        if !self.wait_gone(id, deadline) {
            let file = self.lock().as_ref().and_then(|l| l.file.clone());
            if let Some(file) = file {
                let _ = request(&file, json!({"cmd": "stop"}), STOP_WAIT);
            }
            self.wait_gone(id, Instant::now() + STOP_WAIT);
        }
        let taken = {
            let mut slot = self.lock();
            match slot.as_ref() {
                Some(live) if live.id == id => slot.take(),
                _ => None,
            }
        };
        self.changed.notify_all();
        let Some(mut live) = taken else {
            return;
        };
        let changes = live.read_journal(true);
        self.emit_all(changes);
        if live.stopped.is_none() && screenfile::live(&live.dir).is_none() {
            match self.finalize(id, &live.dir, "interrupted") {
                Ok(()) => {
                    let changes = live.read_journal(true);
                    self.emit_all(changes);
                }
                Err(e) => log::warn!("screen: {id}: couldn't finish the screen journal: {e}"),
            }
        }
    }

    /// Waits until the journal has `stop` or the process is gone, up to `deadline`.
    fn wait_gone(&self, id: &str, deadline: Instant) -> bool {
        let mut last_live_check = None::<Instant>;
        loop {
            {
                let mut slot = self.lock();
                let Some(live) = slot.as_mut().filter(|l| l.id == id) else {
                    return true;
                };
                let changes = live.read_journal(true);
                let exited = live
                    .child
                    .as_mut()
                    .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
                let done = live.stopped.is_some() || exited;
                let check_file = live.child.is_none()
                    && last_live_check.is_none_or(|at| at.elapsed() >= Duration::from_millis(500));
                let dir = live.dir.clone();
                drop(slot);
                self.emit_all(changes);
                if done {
                    return true;
                }
                if check_file {
                    last_live_check = Some(Instant::now());
                    if screenfile::live(&dir).is_none() {
                        return true;
                    }
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    // What Rukoo and the pill ask -------------------------------------------------------------

    /// The meeting's screen recording as `screen.get` answers it, by `deadline`: without time
    /// left to ask the screen process, from the status the watchdog had last.
    pub fn describe(&self, id: &str, deadline: Instant) -> Result<Value, ControlError> {
        if self.lock().as_ref().is_none_or(|live| live.id != id) {
            if let Some(dir) = self
                .lib
                .dir(id)
                .filter(|dir| self.recording_now(dir.path()))
            {
                if self.available() {
                    let _ = dir;
                    return Ok(
                        json!({ "screen": "off", "state": "off", "video": false, "frames": false, "monitor": null, "monitors": self.monitor_list(&Value::Null) }),
                    );
                }
            }
        }
        let view = self.live_of(id, deadline)?;
        let fresh = match (&view.file, view.phase) {
            (Some(file), Phase::Running) => request_until(
                file,
                json!({"cmd": "status"}),
                deadline.min(Instant::now() + REQUEST_WAIT),
            )
            .ok()
            .filter(|s| s["ok"] == true),
            _ => None,
        };
        let status = fresh.or(view.status).unwrap_or(Value::Null);
        let state = match view.phase {
            Phase::Running if view.mode == ScreenMode::Off => "off",
            Phase::Starting | Phase::Restarting => "starting",
            Phase::Running if view.frames => match status["state"].as_str() {
                Some(state @ ("starting" | "recording" | "paused" | "locked")) => state,
                Some("stopped" | "stopping") => "stopped",
                _ => "recording",
            },
            Phase::Running | Phase::Stopped | Phase::Ending => "stopped",
        };
        let live = state != "stopped";
        // The journal knows of every drop; the status says so too, a moment sooner.
        let on = |part: &str, known: bool| live && known && status[part].as_bool() != Some(false);
        let monitors = self.monitor_list(&status);
        Ok(json!({
            "screen": view.mode,
            "state": state,
            "video": on("video", view.video),
            "frames": on("frames", view.frames),
            "monitor": status["monitor"].as_str().filter(|_| live),
            "monitors": monitors
                .iter()
                .map(|m| json!({ "id": m.id, "name": m.name, "width": m.width, "height": m.height, "primary": m.primary }))
                .collect::<Vec<_>>(),
        }))
    }

    /// The monitors of a status answer, or else of the probe, left to right.
    fn monitor_list(&self, status: &Value) -> Vec<MonitorInfo> {
        let listed: Option<Vec<MonitorInfo>> = status
            .get("monitors")
            .and_then(|m| serde_json::from_value(m.clone()).ok());
        let list = listed.unwrap_or_else(|| {
            self.caps
                .get()
                .map(|caps| caps.monitors.clone())
                .unwrap_or_default()
        });
        monitors::ordered(&list)
    }

    /// What `screen.get` and `screen.select` need of the meeting that records now, or their
    /// errors: `not_found` without the meeting, `not_recording` when it doesn't record now,
    /// `not_available` when it records no screen. A screen process that is starting gets until
    /// `deadline`, at most [`STARTING_PATIENCE`].
    fn live_of(&self, id: &str, deadline: Instant) -> Result<LiveView, ControlError> {
        let Some(dir) = self.lib.dir(id) else {
            return Err(ControlError::not_found("There is no meeting with that id."));
        };
        let until = deadline.min(Instant::now() + STARTING_PATIENCE);
        let slot = self.wait_started(self.lock(), Some(id), until);
        match slot.as_ref() {
            Some(live) if live.id == id && live.phase == Phase::Ending => Err(
                ControlError::not_recording("The meeting doesn't record now."),
            ),
            Some(live) if live.id == id => Ok(LiveView {
                mode: live.mode,
                phase: live.phase,
                file: live.file.clone(),
                status: live.status.clone(),
                video: live.recording()
                    && live.mode == ScreenMode::Video
                    && live.dropped_video.is_none(),
                frames: live.recording(),
            }),
            _ => {
                drop(slot);
                if self.recording_now(dir.path()) {
                    Err(ControlError::not_available(
                        "The meeting doesn't record the screen.",
                    ))
                } else {
                    Err(ControlError::not_recording(
                        "The meeting doesn't record now.",
                    ))
                }
            }
        }
    }

    /// Whether the meeting's recorder still records: its timeline has no `stop`.
    fn recording_now(&self, dir: &Path) -> bool {
        voice_meetings::MeetingDir::at(dir).is_some_and(|d| !d.stopped())
    }

    /// `screen.select`: records the monitor with that id. Returns the id. Answers by
    /// `deadline`, `failed` when the screen process is still starting or didn't switch by then.
    pub fn select(
        &self,
        id: &str,
        monitor: &str,
        deadline: Instant,
    ) -> Result<String, ControlError> {
        let view = self.live_of(id, deadline)?;
        if !view.frames || view.phase == Phase::Stopped {
            return Err(ControlError::not_available("The screen recording stopped."));
        }
        let Some(file) = view.file.filter(|_| view.phase == Phase::Running) else {
            return Err(ControlError::failed("The screen recorder is starting."));
        };
        self.switch(&file, monitor, deadline)
    }

    /// Asks the screen process to record `monitor`, and waits for its answer until `until`.
    fn switch(
        &self,
        file: &ScreenFile,
        monitor: &str,
        until: Instant,
    ) -> Result<String, ControlError> {
        let reply = request_until(file, json!({"cmd": "monitor", "id": monitor}), until)
            .map_err(|e| ControlError::failed(format!("The screen recorder didn't answer: {e}")))?;
        if reply["ok"] == true {
            return Ok(reply["monitor"].as_str().unwrap_or(monitor).to_string());
        }
        match reply["error"].as_str().unwrap_or_default() {
            "unknown_monitor" => Err(ControlError::not_found("There is no monitor with that id.")),
            "stopped" => Err(ControlError::not_available("The screen recording stopped.")),
            other => Err(ControlError::failed(format!(
                "The screen recorder refused: {other}"
            ))),
        }
    }

    /// Waits until `until` at most for a screen process that is starting or restarting, so a
    /// switch asked right then isn't lost.
    fn wait_started<'a>(
        &'a self,
        mut slot: MutexGuard<'a, Option<ScreenLive>>,
        id: Option<&str>,
        until: Instant,
    ) -> MutexGuard<'a, Option<ScreenLive>> {
        while slot.as_ref().is_some_and(|l| {
            id.is_none_or(|id| l.id == id)
                && (l.phase == Phase::Starting || l.phase == Phase::Restarting)
        }) {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            slot = self
                .changed
                .wait_timeout(slot, left.min(Duration::from_millis(100)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        slot
    }

    /// The pill's screen button: the recording moves to the next monitor, left to right.
    pub fn next_monitor(&self) -> Result<MonitorInfo, String> {
        let (id, file) = {
            let slot = self.wait_started(self.lock(), None, Instant::now() + STARTING_PATIENCE);
            match slot.as_ref() {
                Some(live) if live.phase == Phase::Running && live.recording() => (
                    live.id.clone(),
                    live.file
                        .clone()
                        .ok_or("The screen recorder is starting.")?,
                ),
                _ => return Err("No meeting records the screen.".into()),
            }
        };
        let status = request(&file, json!({"cmd": "status"}), REQUEST_WAIT)
            .map_err(|e| format!("The screen recorder didn't answer: {e}"))?;
        let list = self.monitor_list(&status);
        let current = status["monitor"].as_str().unwrap_or_default();
        let next = monitors::next(&list, current).ok_or("There is no other monitor.")?;
        if next == current {
            return Err("There is no other monitor.".into());
        }
        self.switch(&file, &next, Instant::now() + SWITCH_WAIT)
            .map_err(|e| format!("{id}: {}", e.message))?;
        list.into_iter()
            .find(|m| m.id == next)
            .ok_or_else(|| "The monitor went away.".into())
    }

    /// The screen mode a meeting records with: its `meeting.json`, or `off` once the live
    /// meeting's screen recording stopped for good.
    pub fn mode_of(&self, id: &str) -> ScreenMode {
        {
            let slot = self.lock();
            if let Some(live) = slot.as_ref().filter(|l| l.id == id) {
                return if live.recording() {
                    live.mode
                } else {
                    ScreenMode::Off
                };
            }
        }
        self.lib
            .dir(id)
            .map(|dir| {
                let history = journal_history(dir.path());
                let mut mode = history
                    .as_ref()
                    .and_then(|h| h.mode.as_deref())
                    .and_then(ScreenMode::parse)
                    .or_else(|| dir.meeting().ok().map(|m| m.screen))
                    .unwrap_or_default();
                if let Some(history) = history {
                    if history.dropped_frames.is_some() {
                        mode = ScreenMode::Off;
                    } else if mode == ScreenMode::Video && history.dropped_video.is_some() {
                        mode = ScreenMode::Frames;
                    }
                }
                mode
            })
            .unwrap_or_default()
    }

    /// What the pill shows for the meeting: the mode it records now, and whether the screen
    /// button can move it to another monitor.
    pub fn pill_state(&self, id: &str) -> (ScreenMode, bool) {
        let known = {
            let slot = self.lock();
            slot.as_ref().filter(|l| l.id == id).map(|live| {
                (
                    live.shown_mode(),
                    !live.gave_up
                        && live.dropped_frames.is_none()
                        && live.stopped.is_none()
                        && live.phase != Phase::Stopped,
                )
            })
        };
        let (mode, usable) = known.unwrap_or_else(|| (self.mode_of(id), true));
        (mode, usable && self.available())
    }

    /// The free space on the meetings volume, when known.
    pub fn free_bytes(&self) -> Option<u64> {
        self.free_bytes.as_ref().and_then(|f| f(self.lib.root()))
    }

    /// Stops watching. A screen process that records goes on; the next start reattaches to it.
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.changed.notify_all();
        self.acked.notify_all();
        let thread = lock(&self.watchdog).take();
        if let Some(thread) = thread {
            if thread.thread().id() != thread::current().id() {
                let _ = thread.join();
            }
        }
    }

    // The process ----------------------------------------------------------------------------

    fn log_file(&self, name: &str) -> Stdio {
        let _ = fs::create_dir_all(&self.logs);
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.logs.join(name))
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null())
    }

    fn launch(
        &self,
        id: &str,
        dir: &Path,
        mode: ScreenMode,
        resume: bool,
    ) -> Result<(Child, ScreenFile), LaunchError> {
        let mut command = Command::new(&self.exe);
        command.args(record_args(
            dir,
            mode,
            resume,
            self.test_monitor.as_deref(),
            self.test.as_deref(),
            &self.exclude_pids(),
        ));
        let log_name = format!("screen-{id}.log");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(self.log_file(&log_name));
        let mut child = spawn_detached(&mut command).map_err(|e| LaunchError {
            code: None,
            message: format!("{} couldn't run: {e}", self.exe.display()),
        })?;
        let deadline = Instant::now() + self.timing.start_timeout;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(LaunchError {
                    code: status.code(),
                    message: last_line(&self.logs.join(&log_name))
                        .unwrap_or_else(|| format!("it exited with {status}")),
                });
            }
            if let Some(file) = screenfile::read(dir) {
                if file.pid == child.id()
                    && request(&file, json!({"cmd": "status"}), REQUEST_WAIT)
                        .is_ok_and(|r| r["ok"] == true)
                {
                    return Ok((child, file));
                }
            }
            if Instant::now() >= deadline || self.stopped.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LaunchError {
                    code: None,
                    message: "it didn't answer in time".into(),
                });
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// `rukoo-voice-screen finalize`: repairs the last video file and ends the journal with
    /// `stop`, for a screen process that can't any more.
    fn finalize(&self, id: &str, dir: &Path, reason: &str) -> Result<(), String> {
        if !self.exe.is_file() {
            return Err(format!("{} is missing", self.exe.display()));
        }
        let mut command = Command::new(&self.exe);
        command
            .arg("finalize")
            .arg("--meeting")
            .arg(dir)
            .arg("--reason")
            .arg(reason)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(self.log_file(&format!("screen-{id}.log")));
        no_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("{} couldn't run: {e}", self.exe.display()))?;
        match wait_child(&mut child, TOOL_WAIT) {
            Some(status) if status.success() => Ok(()),
            Some(status) if status.code() == Some(EXIT_CONFLICT) => Ok(()),
            Some(status) => Err(format!("finalize exited with {status}")),
            None => Err("finalize didn't finish in time".into()),
        }
    }

    // The watchdog ---------------------------------------------------------------------------

    /// One watchdog round. The status request runs without the slot's lock, so a screen process
    /// that is slow to answer doesn't hold up `screen.get`, `screen.select` or the pill.
    fn check(&self) {
        let (id, file, exited, mut changes) = {
            let mut slot = self.lock();
            let Some(live) = slot.as_mut() else {
                return;
            };
            if live.phase != Phase::Running {
                return;
            }
            let changes = live.read_journal(true);
            let exited = live
                .child
                .as_mut()
                .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
            (live.id.clone(), live.file.clone(), exited, changes)
        };
        let status = match (&file, exited) {
            (Some(file), false) => request(file, json!({"cmd": "status"}), REQUEST_WAIT)
                .ok()
                .filter(|r| r["ok"] == true),
            _ => None,
        };
        let mut slot = self.lock();
        let Some(live) = slot
            .as_mut()
            .filter(|l| l.id == id && l.phase == Phase::Running && l.file == file)
        else {
            // The meeting ended or its process changed meanwhile; the next round looks again.
            drop(slot);
            self.emit_all(changes);
            return;
        };
        if let Some(status) = status {
            live.failed_pings = 0;
            if live.restarts > 0 && live.alive_since.elapsed() >= self.timing.stable_after {
                live.restarts = 0;
            }
            let count = status["monitors"].as_array().map(Vec::len);
            if count.is_some() && count != live.monitors {
                live.monitors = count;
                changes.push(ScreenChange::Monitors {
                    count: count.unwrap_or(0),
                });
            }
            live.status = Some(status);
            drop(slot);
            self.emit_all(changes);
            return;
        }
        if !exited && live.file.is_some() && live.stopped.is_none() {
            live.failed_pings += 1;
            // Without a process handle (reattached), a few missed checks mean it's gone.
            if live.child.is_some() || live.failed_pings < self.timing.ping_failures {
                drop(slot);
                self.emit_all(changes);
                return;
            }
        }
        self.died(slot, changes);
    }

    /// The screen process is gone. Ends it for this meeting when its journal has `stop` or the
    /// meeting stopped; otherwise restarts it with `--resume` after the backoff, and gives up
    /// after the last one: `finalize --reason failed`, a notification and the pill's notice.
    fn died(&self, mut slot: MutexGuard<'_, Option<ScreenLive>>, mut changes: Vec<ScreenChange>) {
        let live = slot.as_mut().expect("called with a live screen");
        changes.extend(live.read_journal(true));
        live.child = None;
        live.file = None;
        if live.stopped.is_some() {
            // It stopped on its own: everything dropped, or the meeting's stop.
            live.phase = Phase::Stopped;
            drop(slot);
            self.changed.notify_all();
            self.emit_all(changes);
            return;
        }
        let (id, dir, mode) = (live.id.clone(), live.dir.clone(), live.mode);
        if !self.recording_now(&dir) {
            // The meeting stopped while the screen process was down: nothing to resume.
            live.phase = Phase::Stopped;
            drop(slot);
            self.changed.notify_all();
            self.emit_all(changes);
            if let Err(e) = self.finalize(&id, &dir, "interrupted") {
                log::warn!("screen: {id}: couldn't finish the screen journal: {e}");
            }
            return;
        }
        let max = self.timing.backoff.len() as u32;
        if live.restarts >= max {
            live.phase = Phase::Stopped;
            live.gave_up = true;
            drop(slot);
            self.changed.notify_all();
            self.emit_all(changes);
            if let Err(e) = self.finalize(&id, &dir, "failed") {
                log::warn!("screen: {id}: couldn't finish the screen journal: {e}");
            }
            if let Some(live) = self.lock().as_mut().filter(|l| l.id == id) {
                live.read_journal(false);
            }
            log::warn!("screen: {id}: the screen process stopped {max} times in a row; gave up");
            self.emit(ScreenChange::GaveUp { id });
            return;
        }
        live.restarts += 1;
        live.phase = Phase::Restarting;
        let attempt = live.restarts;
        let delay = self.timing.backoff[(attempt - 1) as usize];
        drop(slot);
        self.emit_all(changes);

        let deadline = Instant::now() + delay;
        while Instant::now() < deadline && !self.stopped.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(20).min(delay));
        }
        let result = if self.stopped.load(Ordering::SeqCst) {
            Err(LaunchError {
                code: None,
                message: "Voice is quitting".into(),
            })
        } else {
            log::warn!("screen: {id}: restarting the screen process (attempt {attempt})");
            self.launch(&id, &dir, mode, journal_has_content(&dir))
        };

        let mut slot = self.lock();
        let Some(live) = slot.as_mut().filter(|l| l.id == id) else {
            return;
        };
        if live.phase != Phase::Restarting {
            return;
        }
        live.phase = Phase::Running;
        live.failed_pings = 0;
        match result {
            Ok((child, file)) => {
                live.child = Some(child);
                live.file = Some(file);
                live.alive_since = Instant::now();
            }
            Err(e) if e.code == Some(EXIT_BUSY) => {
                // Another screen process records into it after all: watch that one. It is
                // alive, so this attempt doesn't count.
                live.restarts = live.restarts.saturating_sub(1);
                live.file = screenfile::live(&dir);
            }
            // The journal has `stop`, or the launch failed: the next round sees it.
            Err(e) => log::warn!("screen: {id}: restart failed: {}", e.message),
        }
        drop(slot);
        self.changed.notify_all();
    }
}

/// The arguments of `rukoo-voice-screen record`, without the program name.
fn record_args(
    dir: &Path,
    mode: ScreenMode,
    resume: bool,
    test_monitor: Option<&str>,
    test: Option<&str>,
    exclude_pids: &[u32],
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "record".into(),
        "--meeting".into(),
        dir.as_os_str().to_owned(),
        "--mode".into(),
        mode.as_str().into(),
    ];
    if resume {
        args.push("--resume".into());
    }
    if let Some(monitor) = test_monitor {
        args.push("--monitor".into());
        args.push(monitor.into());
    }
    if let Some(test) = test {
        args.push("--test".into());
        args.push(test.into());
    }
    // Every connected Rukoo's pid, so its windows stay out of the recording.
    for pid in exclude_pids {
        args.push("--exclude-pid".into());
        args.push(pid.to_string().into());
    }
    args
}

fn watch(supervisor: Weak<ScreenSupervisor>) {
    loop {
        let Some(this) = supervisor.upgrade() else {
            return;
        };
        if this.stopped.load(Ordering::SeqCst) {
            return;
        }
        this.check();
        let poll = this.timing.poll;
        drop(this);
        thread::sleep(poll);
    }
}

/// One control request with the screen process's token.
fn request(file: &ScreenFile, body: Value, timeout: Duration) -> io::Result<Value> {
    control::request(file.port, &file.token, body, timeout)
}

/// One control request that ends, answered or not, by `deadline`.
fn request_until(file: &ScreenFile, body: Value, deadline: Instant) -> io::Result<Value> {
    control::request_until(file.port, &file.token, body, deadline)
}

/// What the journal says, or `None` when there is none.
fn journal_history(dir: &Path) -> Option<journal::History> {
    if !paths::journal_path(dir).is_file() {
        return None;
    }
    let lines = journal::read_all(dir).ok()?;
    Some(journal::History::from_lines(&lines))
}

fn journal_has_content(dir: &Path) -> bool {
    fs::metadata(paths::journal_path(dir)).is_ok_and(|m| m.len() > 0)
}

/// Waits for a child up to `timeout`; kills it after that.
fn wait_child(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// A console program run from a windowed app: no console window flashes up.
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

fn last_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_timing_matches_the_spec() {
        let timing = ScreenTiming::default();
        assert_eq!(timing.poll, Duration::from_millis(500));
        assert_eq!(timing.backoff.len(), 5);
        assert!(timing.backoff.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(timing.handshake, Duration::from_millis(500));
        assert_eq!(timing.reconnect, Duration::from_secs(5));
        assert_eq!(timing.end_wait, Duration::from_secs(10));
    }

    #[test]
    fn the_screen_process_learns_the_pids_to_leave_out() {
        let args = record_args(
            Path::new("/m"),
            ScreenMode::Frames,
            false,
            None,
            None,
            &[4821, 77],
        );
        assert_eq!(
            args,
            [
                "record",
                "--meeting",
                "/m",
                "--mode",
                "frames",
                "--exclude-pid",
                "4821",
                "--exclude-pid",
                "77",
            ]
            .map(OsString::from)
        );
        // Without Rukoo's pid the command line is what it was before the field existed.
        let none = record_args(
            Path::new("/m"),
            ScreenMode::Video,
            true,
            Some("display-2"),
            Some("synthetic:640x480"),
            &[],
        );
        assert_eq!(
            none,
            [
                "record",
                "--meeting",
                "/m",
                "--mode",
                "video",
                "--resume",
                "--monitor",
                "display-2",
                "--test",
                "synthetic:640x480",
            ]
            .map(OsString::from)
        );
    }

    fn live(mode: ScreenMode) -> ScreenLive {
        ScreenLive::new(
            "m-20261030-171203-4f2a",
            Path::new("/nowhere"),
            mode,
            Phase::Running,
        )
    }

    #[test]
    fn the_pill_shows_what_still_records() {
        let mut video = live(ScreenMode::Video);
        assert_eq!(video.shown_mode(), ScreenMode::Video);
        video.dropped_video = Some(Reason::Encoder);
        assert_eq!(video.shown_mode(), ScreenMode::Frames);
        video.dropped_frames = Some(Reason::Disk);
        assert_eq!(video.shown_mode(), ScreenMode::Off);
        assert!(!video.recording());

        let mut frames = live(ScreenMode::Frames);
        frames.gave_up = true;
        assert_eq!(frames.shown_mode(), ScreenMode::Off);
        let mut stopped = live(ScreenMode::Frames);
        stopped.stopped = Some("meeting".into());
        assert!(!stopped.recording());
    }

    /// Video and frames dropped in one read get the frames' notice only; a video drop alone
    /// gets its own.
    #[test]
    fn a_drop_of_both_parts_reports_the_frames() {
        let dir = std::env::temp_dir().join(format!(
            "voice-engine-drops-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut journal = voice_screen_files::journal::JournalWriter::open(&dir).unwrap();
        let mut live = ScreenLive::new("m-1", &dir, ScreenMode::Video, Phase::Running);
        let dropped = |what: &str| Line::Dropped {
            t: 5_000,
            what: what.into(),
            reason: "device".into(),
        };
        journal.write(&dropped("video")).unwrap();
        journal.write(&dropped("frames")).unwrap();
        let changes = live.read_journal(true);
        assert!(
            matches!(
                changes.as_slice(),
                [ScreenChange::Dropped {
                    what: Part::Frames,
                    reason: Reason::Device,
                    ..
                }]
            ),
            "{changes:?}"
        );
        assert_eq!(live.dropped_video, Some(Reason::Device));
        assert_eq!(live.shown_mode(), ScreenMode::Off);

        let other = dir.join("video-only");
        std::fs::create_dir_all(&other).unwrap();
        let mut journal = voice_screen_files::journal::JournalWriter::open(&other).unwrap();
        let mut live = ScreenLive::new("m-2", &other, ScreenMode::Video, Phase::Running);
        journal.write(&dropped("video")).unwrap();
        let changes = live.read_journal(true);
        assert!(
            matches!(
                changes.as_slice(),
                [ScreenChange::Dropped {
                    what: Part::Video,
                    reason: Reason::Device,
                    ..
                }]
            ),
            "{changes:?}"
        );
        assert_eq!(live.shown_mode(), ScreenMode::Frames);
        drop(journal);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
