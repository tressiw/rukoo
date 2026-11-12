//! The meeting controller: runs `rukoo-voice-recorder` for the meeting that records, and keeps it
//! running.
//!
//! The recorder is its own detached process, so the app can crash, quit or update while a
//! meeting records. The controller starts it, talks to it over its control port, restarts it
//! with `--resume` when it dies (up to five times, with backoff), reattaches to it after the app
//! restarts and finishes meetings that were cut off. It never stops a recorder because the app
//! is quitting.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use voice_bridge::ControlError;
use voice_meetings::{CreateMeeting, Library, MeetingFile};
use voice_protocol::{ScreenMode, StartArgs, StartedBy, Track};
use voice_recorder::control::Client;
use voice_recorder::meeting::{self, RecorderFile};
use voice_recorder::timeline::{self, History, Line};

/// The recorder's exit codes (voice/RECORDING.md).
const EXIT_CONFLICT: i32 = 3;
const EXIT_BUSY: i32 = 4;
/// How long Stop keeps asking a recorder that holds the meeting but doesn't answer.
const STOP_PATIENCE: Duration = Duration::from_secs(20);

/// How the controller waits and retries. The defaults are for real use; tests shorten them.
#[derive(Clone, Debug)]
pub struct ControllerTiming {
    /// How often the watchdog checks the recorder.
    pub poll: Duration,
    /// How long a starting recorder may take to answer on its control port.
    pub start_timeout: Duration,
    /// Waits before each restart; their number is the most restarts in a row.
    pub backoff: Vec<Duration>,
    /// A recorder that has run this long counts as healthy again: the restart count resets.
    pub stable_after: Duration,
    /// Status checks in a row that may fail before a recorder without a process handle counts
    /// as dead.
    pub ping_failures: u32,
    /// Extra recorder arguments, for tests (`--segment-ms`).
    pub recorder_args: Vec<String>,
}

impl Default for ControllerTiming {
    fn default() -> ControllerTiming {
        ControllerTiming {
            poll: Duration::from_millis(500),
            start_timeout: Duration::from_secs(20),
            backoff: [500, 1000, 2000, 4000, 8000]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
            stable_after: Duration::from_secs(60),
            ping_failures: 3,
            recorder_args: Vec::new(),
        }
    }
}

/// What the controller tells the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Started {
        id: String,
        at_ms: u64,
    },
    /// Found a recorder still recording after the app restarted.
    Reattached {
        id: String,
        paused: bool,
        at_ms: u64,
    },
    Paused {
        id: String,
        at_ms: u64,
    },
    Resumed {
        id: String,
        at_ms: u64,
    },
    /// The recorder's clock and microphone, about once a second.
    Clock {
        id: String,
        at_ms: u64,
        mic_ok: bool,
    },
    /// The recorder died and a new one took over with `--resume`.
    Restarted {
        id: String,
        attempt: u32,
    },
    Ended {
        id: String,
        reason: String,
        at_ms: u64,
    },
    /// A meeting cut off while the app wasn't running, finished at start.
    Finalized {
        id: String,
    },
    /// Something the user should read in a notification.
    Notify {
        title_key: &'static str,
        body: String,
    },
}

/// The meeting recording now, for the tray and the pill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recording {
    pub id: String,
    pub paused: bool,
    pub at_ms: u64,
}

/// `RUKOO_VOICE_TEST_SOURCES`: `mic=file:a.wav,call=synthetic:tone:440`. Options of a source
/// (`synthetic:tone:440,fail-at=2000`) stay with it: a new track starts only at `mic=` or `call=`.
pub fn parse_sources(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut tracks: Vec<(String, String)> = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            Some((track @ ("mic" | "call"), source)) => {
                if tracks.iter().any(|(t, _)| t == track) {
                    return Err(format!("track {track} is given twice"));
                }
                tracks.push((track.to_string(), source.to_string()));
            }
            _ => match tracks.last_mut() {
                Some((_, source)) => {
                    source.push(',');
                    source.push_str(part);
                }
                None => {
                    return Err(format!(
                        "expected mic=<source> or call=<source>, got {part:?}"
                    ))
                }
            },
        }
    }
    if tracks.is_empty() {
        return Err("no tracks".into());
    }
    Ok(tracks)
}

pub struct ControllerOptions {
    pub data_dir: PathBuf,
    pub recorder_exe: PathBuf,
    /// Track name to source; the defaults are `default-input` and `default-output`.
    pub sources: Vec<(String, String)>,
    pub timing: ControllerTiming,
}

pub fn default_sources() -> Vec<(String, String)> {
    vec![
        ("mic".into(), "default-input".into()),
        ("call".into(), "default-output".into()),
    ]
}

struct Live {
    id: String,
    dir: PathBuf,
    tracks: Vec<(String, String)>,
    child: Option<Child>,
    file: RecorderFile,
    paused: bool,
    at_ms: u64,
    at_when: Instant,
    mic_ok: bool,
    restarting: bool,
    stopping: bool,
    restarts: u32,
    alive_since: Instant,
    failed_pings: u32,
}

impl Live {
    fn clock(&self) -> u64 {
        self.at_ms + self.at_when.elapsed().as_millis() as u64
    }
}

pub struct Controller {
    lib: Library,
    exe: PathBuf,
    logs: PathBuf,
    sources: Vec<(String, String)>,
    timing: ControllerTiming,
    inner: Mutex<Option<Live>>,
    changed: Condvar,
    on_change: Box<dyn Fn(Change) + Send + Sync>,
    stopped: AtomicBool,
    watchdog: Mutex<Option<JoinHandle<()>>>,
    /// Settings' screen mode for a meeting that doesn't say, and whether this computer can
    /// record the screen at all.
    screen: Mutex<(ScreenMode, bool)>,
}

#[derive(Debug)]
struct LaunchError {
    code: Option<i32>,
    message: String,
}

impl Controller {
    pub fn new(
        options: ControllerOptions,
        on_change: impl Fn(Change) + Send + Sync + 'static,
    ) -> io::Result<Arc<Controller>> {
        let lib = Library::open(&options.data_dir)?;
        let controller = Arc::new(Controller {
            lib,
            exe: options.recorder_exe,
            logs: voice_core::paths::logs_dir(&options.data_dir),
            sources: options.sources,
            timing: options.timing,
            inner: Mutex::new(None),
            changed: Condvar::new(),
            on_change: Box::new(on_change),
            stopped: AtomicBool::new(false),
            watchdog: Mutex::new(None),
            screen: Mutex::new((ScreenMode::Off, false)),
        });
        let weak = Arc::downgrade(&controller);
        let thread = thread::Builder::new()
            .name("recorder-watchdog".into())
            .spawn(move || watch(weak))?;
        *lock(&controller.watchdog) = Some(thread);
        Ok(controller)
    }

    pub fn library(&self) -> &Library {
        &self.lib
    }

    /// Settings > Meetings > Screen, and whether the screen process can record here. A meeting
    /// started after this records `default` unless its start asks for another mode, and `off`
    /// when the screen can't be recorded.
    pub fn set_screen(&self, default: ScreenMode, available: bool) {
        *lock(&self.screen) = (default, available);
    }

    /// The `screen` a meeting gets when its start leaves it out (`status.screenDefault`):
    /// Settings' mode, or `off` when this computer can't record the screen, so Rukoo never
    /// labels a default that Voice wouldn't record.
    pub fn screen_default(&self) -> ScreenMode {
        match *lock(&self.screen) {
            (default, true) => default,
            (_, false) => ScreenMode::Off,
        }
    }

    fn screen_available(&self) -> bool {
        lock(&self.screen).1
    }

    fn lock(&self) -> MutexGuard<'_, Option<Live>> {
        lock(&self.inner)
    }

    fn emit(&self, change: Change) {
        (self.on_change)(change);
    }

    /// At start: reattaches to a recorder that still records and finishes meetings whose
    /// recorder died while the app wasn't running.
    pub fn recover(&self) {
        for dir in self.lib.dirs() {
            let id = dir.id().to_string();
            let path = meeting::timeline_path(dir.path());
            let Ok(lines) = timeline::read(&path) else {
                continue;
            };
            if lines.is_empty() {
                continue;
            }
            if let Some(file) = meeting::live_recorder(dir.path()) {
                let mut inner = self.lock();
                if inner.is_some() {
                    continue;
                }
                let status = request(&file, json!({"cmd": "status"}), Duration::from_secs(2)).ok();
                let paused = status.as_ref().is_some_and(|s| s["state"] == "paused");
                let at_ms = status.as_ref().and_then(|s| s["t"].as_u64()).unwrap_or(0);
                // A dictation the app before this one started can't end any more: that app is
                // gone. The recorder ends it when the app's tap closes; this covers a dictation
                // without a tap. Nothing here dictates yet: the lock keeps `dictation` out.
                if History::from_lines(&lines).dictating.is_some() {
                    let _ = request(
                        &file,
                        json!({"cmd": "dictation", "state": "end"}),
                        Duration::from_secs(2),
                    );
                }
                *inner = Some(Live {
                    id: id.clone(),
                    dir: dir.path().to_path_buf(),
                    tracks: tracks_of(dir.meeting().ok().as_ref(), &lines, &self.sources),
                    child: None,
                    file,
                    paused,
                    at_ms,
                    at_when: Instant::now(),
                    mic_ok: true,
                    restarting: false,
                    stopping: false,
                    restarts: 0,
                    alive_since: Instant::now(),
                    failed_pings: 0,
                });
                drop(inner);
                self.emit(Change::Reattached { id, paused, at_ms });
                continue;
            }
            if History::from_lines(&lines).stopped {
                continue;
            }
            match meeting::finalize(dir.path(), "interrupted") {
                Ok(_) => self.emit(Change::Finalized { id }),
                Err(e) => self.emit(Change::Notify {
                    title_key: NOTIFY_MEETING,
                    body: format!("Voice couldn't finish a meeting that was cut off: {e}"),
                }),
            }
        }
    }

    /// Starts recording a new meeting.
    pub fn start(&self, args: StartArgs, by: StartedBy) -> Result<String, ControlError> {
        let mut inner = self.lock();
        if let Some(live) = inner.as_ref() {
            return Err(ControlError::busy(
                live.id.clone(),
                "A meeting is already recording.",
            ));
        }
        let wanted = args.tracks.unwrap_or_else(|| vec![Track::Mic, Track::Call]);
        let tracks: Vec<(String, String)> = self
            .sources
            .iter()
            .filter(|(name, _)| wanted.iter().any(|t| t.as_str() == name))
            .cloned()
            .collect();
        if tracks.is_empty() {
            return Err(ControlError::failed("No source is set for those tracks."));
        }
        let title = args
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "Meeting".to_string());
        let dir = self
            .lib
            .create(&CreateMeeting {
                title,
                tracks: tracks
                    .iter()
                    .filter_map(|(name, _)| Track::parse(name))
                    .collect(),
                sources: tracks.iter().cloned().collect::<BTreeMap<_, _>>(),
                // A Voice that can't record the screen here starts the meeting with `off`.
                screen: if self.screen_available() {
                    args.screen.unwrap_or(self.screen_default())
                } else {
                    ScreenMode::Off
                },
                started_by: by,
                context: args.context,
            })
            .map_err(|e| ControlError::failed(format!("Couldn't make the meeting folder: {e}")))?;
        let id = dir.id().to_string();
        match self.launch(&id, dir.path(), &tracks, false) {
            Ok((child, file)) => {
                let now = Instant::now();
                *inner = Some(Live {
                    id: id.clone(),
                    dir: dir.path().to_path_buf(),
                    tracks,
                    child: Some(child),
                    file,
                    paused: false,
                    at_ms: 0,
                    at_when: now,
                    mic_ok: true,
                    restarting: false,
                    stopping: false,
                    restarts: 0,
                    alive_since: now,
                    failed_pings: 0,
                });
                drop(inner);
                self.emit(Change::Started {
                    id: id.clone(),
                    at_ms: 0,
                });
                Ok(id)
            }
            Err(e) => {
                // A folder without a timeline holds nothing worth keeping.
                if !meeting::timeline_path(dir.path()).exists() {
                    let _ = fs::remove_dir_all(dir.path());
                }
                Err(ControlError::failed(format!(
                    "The recorder didn't start: {}",
                    e.message
                )))
            }
        }
    }

    /// The live meeting with `id` (or any, for `None`), waiting out a restart.
    fn live_for<'a>(
        &'a self,
        mut inner: MutexGuard<'a, Option<Live>>,
        id: Option<&str>,
    ) -> Result<MutexGuard<'a, Option<Live>>, ControlError> {
        let deadline = Instant::now() + self.timing.start_timeout + Duration::from_secs(10);
        while inner.as_ref().is_some_and(|l| l.restarting) && Instant::now() < deadline {
            inner = self
                .changed
                .wait_timeout(inner, Duration::from_millis(200))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        match inner.as_ref() {
            Some(live) if id.is_none_or(|id| id == live.id) && !live.stopping => {
                if live.restarting {
                    return Err(ControlError::failed("The recorder is restarting."));
                }
                Ok(inner)
            }
            _ => Err(self.not_live(id)),
        }
    }

    fn not_live(&self, id: Option<&str>) -> ControlError {
        match id {
            Some(id) if self.lib.dir(id).is_some() => {
                ControlError::not_recording("The meeting isn't recording.")
            }
            Some(_) => ControlError::not_found("There is no meeting with that id."),
            None => ControlError::not_recording("No meeting is recording."),
        }
    }

    /// Stops the meeting. `reason` is `user` or `rukoo`.
    pub fn stop(&self, id: Option<&str>, reason: &str) -> Result<(), ControlError> {
        let (file, dir) = {
            let mut inner = self.live_for(self.lock(), id)?;
            let live = inner.as_mut().expect("live_for checked it");
            // Keeps the watchdog and every other command off the meeting while it stops, so
            // the lock needn't be held through the requests.
            live.stopping = true;
            (live.file.clone(), live.dir.clone())
        };
        let stopped = stop_recorder(&file, &dir, reason);
        let mut inner = self.lock();
        // Nothing else ends a meeting that is stopping; this is only defensive.
        let Some(live) = inner.as_mut() else {
            return stopped.map(|_| ());
        };
        let at_ms = match stopped {
            Ok(at_ms) => at_ms.unwrap_or(live.clock()),
            Err(e) => {
                live.stopping = false;
                return Err(e);
            }
        };
        if let Some(child) = live.child.as_mut() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
        }
        let id = live.id.clone();
        *inner = None;
        drop(inner);
        self.changed.notify_all();
        self.emit(Change::Ended {
            id,
            reason: reason.to_string(),
            at_ms,
        });
        Ok(())
    }

    pub fn pause(&self, id: Option<&str>) -> Result<(), ControlError> {
        self.pause_or_resume(id, true)
    }

    pub fn resume(&self, id: Option<&str>) -> Result<(), ControlError> {
        self.pause_or_resume(id, false)
    }

    fn pause_or_resume(&self, id: Option<&str>, pause: bool) -> Result<(), ControlError> {
        let mut inner = self.live_for(self.lock(), id)?;
        let live = inner.as_mut().expect("live_for checked it");
        let cmd = if pause { "pause" } else { "resume" };
        let reply = command(&live.file, json!({ "cmd": cmd }))?;
        live.paused = pause;
        live.at_ms = reply["t"].as_u64().unwrap_or(live.clock());
        live.at_when = Instant::now();
        let (id, at_ms) = (live.id.clone(), live.at_ms);
        drop(inner);
        self.emit(if pause {
            Change::Paused { id, at_ms }
        } else {
            Change::Resumed { id, at_ms }
        });
        Ok(())
    }

    /// A user mark. Returns its id and meeting time.
    pub fn mark(
        &self,
        id: Option<&str>,
        label: Option<&str>,
    ) -> Result<(String, u64), ControlError> {
        let inner = self.live_for(self.lock(), id)?;
        let live = inner.as_ref().expect("live_for checked it");
        let mut request = json!({"cmd": "mark"});
        if let Some(label) = label {
            // The mark event Rukoo gets allows 200 characters (PROTOCOL.md, Checking messages).
            let label: String = label
                .chars()
                .take(voice_protocol::MAX_LABEL_CHARS)
                .collect();
            request["label"] = label.into();
        }
        let reply = command(&live.file, request)?;
        let mark = reply["id"].as_str().unwrap_or_default().to_string();
        Ok((mark, reply["t"].as_u64().unwrap_or(0)))
    }

    /// Tells the recorder the user dictates on the microphone (decision 9). Returns the meeting
    /// it told, or `None` without a meeting.
    pub fn dictation(&self, state: &str) -> Option<String> {
        let inner = self.lock();
        let live = inner.as_ref().filter(|l| !l.restarting && !l.stopping)?;
        let _ = request(
            &live.file,
            json!({"cmd": "dictation", "state": state}),
            Duration::from_secs(2),
        );
        Some(live.id.clone())
    }

    /// How to reach the recorder's microphone tap, while a meeting records.
    pub fn tap(&self) -> Option<RecorderFile> {
        let inner = self.lock();
        inner
            .as_ref()
            .filter(|l| !l.restarting && !l.stopping && l.tracks.iter().any(|(t, _)| t == "mic"))
            .map(|l| l.file.clone())
    }

    pub fn current(&self) -> Option<Recording> {
        self.lock().as_ref().map(|live| Recording {
            id: live.id.clone(),
            paused: live.paused,
            at_ms: live.clock(),
        })
    }

    /// Stops watching. The recorder keeps recording; the next start reattaches to it.
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.changed.notify_all();
        let thread = lock(&self.watchdog).take();
        if let Some(thread) = thread {
            if thread.thread().id() != thread::current().id() {
                let _ = thread.join();
            }
        }
    }

    fn launch(
        &self,
        id: &str,
        dir: &Path,
        tracks: &[(String, String)],
        resume: bool,
    ) -> Result<(Child, RecorderFile), LaunchError> {
        let mut command = Command::new(&self.exe);
        command.arg("record").arg("--meeting").arg(dir);
        if resume {
            command.arg("--resume");
        }
        for (track, source) in tracks {
            command.arg("--track").arg(format!("{track}={source}"));
        }
        command.args(&self.timing.recorder_args);
        let _ = fs::create_dir_all(&self.logs);
        let log_path = self.logs.join(format!("recorder-{id}.log"));
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null());
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);
        let mut child = spawn_detached(&mut command).map_err(|e| LaunchError {
            code: None,
            message: format!("{} couldn't run: {e}", self.exe.display()),
        })?;
        let deadline = Instant::now() + self.timing.start_timeout;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(LaunchError {
                    code: status.code(),
                    message: last_line(&log_path)
                        .unwrap_or_else(|| format!("it exited with {status}")),
                });
            }
            if let Some(file) = meeting::read_recorder_file(dir) {
                if file.pid == child.id()
                    && request(&file, json!({"cmd": "status"}), Duration::from_secs(1))
                        .is_ok_and(|r| r["ok"] == true)
                {
                    return Ok((child, file));
                }
            }
            if Instant::now() >= deadline {
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

    /// One watchdog round.
    fn check(&self) {
        let mut inner = self.lock();
        let Some(live) = inner.as_mut() else {
            return;
        };
        if live.restarting || live.stopping {
            return;
        }
        let exited = live
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))));
        let status = if exited {
            None
        } else {
            request(&live.file, json!({"cmd": "status"}), Duration::from_secs(1))
                .ok()
                .filter(|r| r["ok"] == true)
        };
        if let Some(status) = status {
            live.failed_pings = 0;
            if let Some(t) = status["t"].as_u64() {
                live.at_ms = t;
                live.at_when = Instant::now();
            }
            live.paused = status["state"] == "paused";
            live.mic_ok = status["tracks"]["mic"]["ok"] != false;
            if live.restarts > 0 && live.alive_since.elapsed() >= self.timing.stable_after {
                live.restarts = 0;
            }
            let change = Change::Clock {
                id: live.id.clone(),
                at_ms: live.at_ms,
                mic_ok: live.mic_ok,
            };
            drop(inner);
            self.emit(change);
            return;
        }
        if !exited {
            live.failed_pings += 1;
            // Without a process handle (reattached), a few missed checks mean it's gone.
            if live.child.is_some() || live.failed_pings < self.timing.ping_failures {
                return;
            }
        }
        self.recorder_died(inner);
    }

    fn recorder_died(&self, mut inner: MutexGuard<'_, Option<Live>>) {
        let live = inner.as_mut().expect("called with a live meeting");
        let lines = timeline::read(&meeting::timeline_path(&live.dir)).unwrap_or_default();
        if let Some((reason, t)) = stop_line(&lines) {
            // It stopped on its own: the length limit.
            let id = live.id.clone();
            *inner = None;
            drop(inner);
            self.changed.notify_all();
            self.emit(Change::Ended {
                id,
                reason,
                at_ms: t,
            });
            return;
        }
        let max = self.timing.backoff.len() as u32;
        if live.restarts >= max {
            let finished = meeting::finalize(&live.dir, "interrupted");
            if finished.as_ref().is_err_and(|e| e.code == EXIT_BUSY) {
                // A recorder still holds the meeting: it lives and didn't answer. The meeting
                // goes on, and the watchdog keeps asking it.
                live.failed_pings = 0;
                return;
            }
            let id = live.id.clone();
            *inner = None;
            drop(inner);
            self.changed.notify_all();
            let at_ms = finished.ok().and_then(|done| done.stopped_at).unwrap_or(0);
            self.emit(Change::Notify {
                title_key: NOTIFY_MEETING,
                body: format!(
                    "The recorder stopped {max} times in a row, so Voice ended the meeting. Everything recorded until then is saved."
                ),
            });
            self.emit(Change::Ended {
                id,
                reason: "interrupted".into(),
                at_ms,
            });
            return;
        }
        live.restarts += 1;
        live.restarting = true;
        let attempt = live.restarts;
        let delay = self.timing.backoff[(attempt - 1) as usize];
        let (id, dir, tracks) = (live.id.clone(), live.dir.clone(), live.tracks.clone());
        drop(inner);

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
            self.launch(&id, &dir, &tracks, true)
        };

        let mut inner = self.lock();
        let Some(live) = inner.as_mut() else {
            return;
        };
        live.restarting = false;
        match result {
            Ok((child, file)) => {
                live.child = Some(child);
                live.file = file;
                live.alive_since = Instant::now();
                live.failed_pings = 0;
                drop(inner);
                self.changed.notify_all();
                self.emit(Change::Restarted { id, attempt });
            }
            Err(e) if e.code == Some(EXIT_BUSY) => {
                // A recorder records into it after all: watch that one. When it holds the
                // meeting without answering it is alive all the same, so nothing died and this
                // attempt doesn't count towards giving up.
                live.restarts = live.restarts.saturating_sub(1);
                live.child = None;
                live.failed_pings = 0;
                if let Some(file) = meeting::live_recorder(&dir) {
                    live.file = file;
                }
                drop(inner);
                self.changed.notify_all();
            }
            Err(e) if e.code == Some(EXIT_CONFLICT) => {
                // The timeline has `stop`: the next check ends the meeting.
                drop(inner);
                self.changed.notify_all();
            }
            Err(_) => {
                // The next check sees the dead process again and tries once more.
                drop(inner);
                self.changed.notify_all();
            }
        }
    }
}

pub const NOTIFY_MEETING: &str = "app.notify.meeting";

fn watch(controller: Weak<Controller>) {
    loop {
        let Some(this) = controller.upgrade() else {
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

fn stop_line(lines: &[Line]) -> Option<(String, u64)> {
    lines.iter().rev().find_map(|line| match line {
        Line::Stop { t, reason, .. } => Some((reason.clone(), *t)),
        _ => None,
    })
}

/// The tracks a meeting records and the source of each. `meeting.json` holds what the meeting was
/// started with and wins: the timeline's `track` lines only name devices that opened, so a
/// microphone that was missing from the start would drop out of the meeting after a watchdog
/// restart, and a call-only meeting would pick up the default microphone. The timeline and then
/// the configured sources only fill in a source `meeting.json` doesn't name.
fn tracks_of(
    meeting: Option<&MeetingFile>,
    lines: &[Line],
    fallback: &[(String, String)],
) -> Vec<(String, String)> {
    let mut opened: Vec<(String, String)> = Vec::new();
    for line in lines {
        if let Line::Track { track, source, .. } = line {
            if !opened.iter().any(|(t, _)| t == track) {
                opened.push((track.clone(), source.clone()));
            }
        }
    }
    if let Some(meeting) = meeting {
        let source_of = |name: &str| {
            meeting
                .sources
                .get(name)
                .cloned()
                .or_else(|| {
                    opened
                        .iter()
                        .find(|(t, _)| t == name)
                        .map(|(_, s)| s.clone())
                })
                .or_else(|| {
                    fallback
                        .iter()
                        .find(|(t, _)| t == name)
                        .map(|(_, s)| s.clone())
                })
        };
        let chosen: Vec<(String, String)> = meeting
            .tracks
            .iter()
            .filter_map(|track| {
                source_of(track.as_str()).map(|source| (track.as_str().to_string(), source))
            })
            .collect();
        if !chosen.is_empty() {
            return chosen;
        }
    }
    if opened.is_empty() {
        fallback.to_vec()
    } else {
        opened
    }
}

/// Asks the recorder to stop, or finishes the folder when no recorder records into it any more,
/// so the meeting ends where its audio does. Returns the meeting time of the `stop` line, when
/// one was written now.
///
/// `finalize` answers busy while a recorder still holds the meeting: that recorder is alive and
/// didn't answer, for instance while its control port is full. The meeting goes on, so Stop
/// asks again for a while and then says so, instead of reporting a meeting that ended.
fn stop_recorder(
    file: &RecorderFile,
    dir: &Path,
    reason: &str,
) -> Result<Option<u64>, ControlError> {
    let deadline = Instant::now() + STOP_PATIENCE;
    loop {
        let reply = request(
            file,
            json!({"cmd": "stop", "reason": reason}),
            Duration::from_secs(15),
        );
        if let Ok(reply) = reply {
            if reply["ok"] == true {
                return Ok(reply["t"].as_u64());
            }
        }
        match meeting::finalize(dir, reason) {
            Ok(done) => return Ok(done.stopped_at),
            Err(e) if e.code == EXIT_BUSY && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(250));
            }
            Err(e) if e.code == EXIT_BUSY => {
                return Err(ControlError::failed(
                    "The recorder still records but doesn't answer, so the meeting goes on.",
                ))
            }
            Err(e) => {
                return Err(ControlError::failed(format!(
                    "The meeting couldn't be stopped: {e}"
                )))
            }
        }
    }
}

/// One control request with the recorder's token.
pub fn request(file: &RecorderFile, mut body: Value, timeout: Duration) -> io::Result<Value> {
    let mut client = Client::connect(file.port, timeout.min(Duration::from_secs(2)))?;
    client.set_read_timeout(Some(timeout))?;
    body["token"] = file.token.clone().into();
    client.request(&body)
}

/// A request whose failure is a command error for the bridge.
fn command(file: &RecorderFile, body: Value) -> Result<Value, ControlError> {
    let reply = request(file, body, Duration::from_secs(5))
        .map_err(|e| ControlError::failed(format!("The recorder didn't answer: {e}")))?;
    if reply["ok"] == true {
        return Ok(reply);
    }
    match reply["error"].as_str().unwrap_or_default() {
        "not_recording" => Err(ControlError::not_recording(
            "The meeting is already in that state.",
        )),
        other => Err(ControlError::failed(format!(
            "The recorder refused: {other}"
        ))),
    }
}

fn last_line(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
}

/// Starts the recorder (or the screen process) outside the app's console, process group and
/// kill-on-close job, so it outlives the app (voice/RECORDING.md).
///
/// That is the difference from Voice's own workers, which end with the app: the local speech
/// worker and the speaker label worker get `local::parent`'s variable from their spawn code and
/// watch Voice's pid, while the recorder is never told a parent to watch. A meeting can't be
/// repeated, so the recorder keeps recording after Voice crashes.
pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const ERROR_ACCESS_DENIED: i32 = 5;
        command.creation_flags(
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
        );
        match command.spawn() {
            // A job that doesn't allow breakaway refuses the flag; the process still runs
            // detached, and outlives the app unless that job is closed with it.
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
                command.spawn()
            }
            other => other,
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        command.spawn()
    }
    #[cfg(not(any(windows, unix)))]
    {
        command.spawn()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting_file(tracks: &[voice_protocol::Track], sources: &[(&str, &str)]) -> MeetingFile {
        MeetingFile {
            id: voice_protocol::MeetingId::parse("m-20261030-171203-4f2a").unwrap(),
            title: String::new(),
            created_at: 1_793_000_000_000,
            tracks: tracks.to_vec(),
            sources: sources
                .iter()
                .map(|(t, s)| (t.to_string(), s.to_string()))
                .collect(),
            screen: Default::default(),
            started_by: voice_protocol::StartedBy::Voice,
            context: None,
            version: 1,
        }
    }

    fn opened(track: &str, source: &str) -> Line {
        Line::Track {
            t: 0,
            track: track.into(),
            source: source.into(),
            device: "Device".into(),
            device_id: String::new(),
            native_rate: 48_000,
            native_channels: 2,
        }
    }

    fn pair(track: &str, source: &str) -> (String, String) {
        (track.to_string(), source.to_string())
    }

    #[test]
    fn a_microphone_that_never_opened_stays_in_the_meeting() {
        use voice_protocol::Track;
        let meeting = meeting_file(
            &[Track::Mic, Track::Call],
            &[("mic", "input:headset"), ("call", "default-output")],
        );
        let lines = vec![opened("call", "default-output")];
        let configured = vec![pair("mic", "default-input"), pair("call", "default-output")];
        assert_eq!(
            tracks_of(Some(&meeting), &lines, &configured),
            vec![pair("mic", "input:headset"), pair("call", "default-output")]
        );
    }

    #[test]
    fn a_call_only_meeting_never_gains_a_microphone() {
        use voice_protocol::Track;
        let meeting = meeting_file(&[Track::Call], &[("call", "default-output")]);
        let configured = vec![pair("mic", "default-input"), pair("call", "default-output")];
        assert_eq!(
            tracks_of(Some(&meeting), &[], &configured),
            vec![pair("call", "default-output")]
        );
    }

    #[test]
    fn a_meeting_file_without_sources_borrows_them_from_the_timeline_then_the_settings() {
        use voice_protocol::Track;
        let meeting = meeting_file(&[Track::Mic, Track::Call], &[]);
        let lines = vec![opened("call", "output:speakers")];
        let configured = vec![pair("mic", "default-input"), pair("call", "default-output")];
        assert_eq!(
            tracks_of(Some(&meeting), &lines, &configured),
            vec![
                pair("mic", "default-input"),
                pair("call", "output:speakers")
            ]
        );
        assert_eq!(
            tracks_of(None, &lines, &configured),
            vec![pair("call", "output:speakers")]
        );
    }

    #[test]
    fn test_sources_keep_their_options() {
        assert_eq!(
            parse_sources("mic=file:a.wav,call=synthetic:tone:440").unwrap(),
            vec![
                ("mic".to_string(), "file:a.wav".to_string()),
                ("call".to_string(), "synthetic:tone:440".to_string())
            ]
        );
        assert_eq!(
            parse_sources("mic=synthetic:tone:440,fail-at=2000,back-at=4000, call=file:x.wav,loop")
                .unwrap(),
            vec![
                (
                    "mic".to_string(),
                    "synthetic:tone:440,fail-at=2000,back-at=4000".to_string()
                ),
                ("call".to_string(), "file:x.wav,loop".to_string())
            ]
        );
        assert!(parse_sources("").is_err());
        assert!(parse_sources("fail-at=2").is_err());
        assert!(parse_sources("mic=a,mic=b").is_err());
    }

    #[test]
    fn the_default_timing_restarts_five_times() {
        let timing = ControllerTiming::default();
        assert_eq!(timing.backoff.len(), 5);
        assert!(timing.backoff.windows(2).all(|w| w[0] < w[1]));
    }
}
