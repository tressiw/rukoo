//! A stand-in engine for development (`--fake-engine`, only with `RUKOO_VOICE_DATA_DIR` set). It plays
//! scripted pill states and keeps a Settings state in memory, so the windows, the stop window and the tray
//! can be exercised before the real engine lands. It records nothing, listens to no keys, touches no
//! network and writes no files.
//!
//! Scenarios: `idle` (nothing shows until the tray starts a meeting), `meeting` (a meeting records the
//! audio and screen frames from the start, with a screen button that cycles two fake monitors),
//! `dictation` (a dictation every 8 s), `cycle` (every pill state in turn) and `first-run` (Settings
//! opens at the Ownkey import). A meeting that records shows in Settings > Speaker labels,
//! labelled once the Nemotron model is "downloaded".

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use voice_engine::{
    Activity, ApiError, Edge, EngineApi, EngineOptions, Folder, Locale, MeetingPill, Notice,
    PillPhase, PillSnapshot, StopReason, Theme, TrayState, UiOptions, UiSink,
};

pub const SCENARIOS: &[&str] = &["idle", "meeting", "dictation", "cycle", "first-run"];

const KEYS: &[&str] = &[
    "right alt",
    "right ctrl",
    "right shift",
    "f13",
    "f14",
    "f15",
    "pause",
    "scroll lock",
];
const FILLER_LANGUAGES: &[&str] = &["en", "nl", "de", "fr", "es"];
/// Mistral's realtime models stream meetings; nothing else does.
const REALTIME_PREFIX: &str = "voxtral-mini-transcribe-realtime";
const TICK: Duration = Duration::from_millis(20);
/// The fake meeting's monitors, which the pill's screen button cycles.
const MONITORS: &[&str] = &["DELL U2720Q", "Display 2"];

/// The engine's hold times (pill-settings-design.md 3.4), shortened in tests.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub confirm: Duration,
    pub compact: Duration,
    pub saving: Duration,
    pub resend: Duration,
    /// How long a screen notice ("Screen: Display 2") stays.
    pub notice: Duration,
    pub dictation_every: Duration,
    pub cycle_step: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            confirm: Duration::from_millis(3000),
            compact: Duration::from_millis(4000),
            saving: Duration::from_millis(2200),
            resend: Duration::from_secs(60),
            notice: Duration::from_millis(2200),
            dictation_every: Duration::from_secs(8),
            cycle_step: Duration::from_millis(1500),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Recording,
    Paused,
    Saving { until: Instant },
}

struct Meeting {
    id: String,
    run: Run,
    started: Instant,
    confirm_until: Option<Instant>,
    compact_at: Option<Instant>,
    compact: bool,
    resend_at: Instant,
    /// The fake monitor the screen records, an index into [`MONITORS`].
    monitor: usize,
    screen: String,
    /// The pill's notice and when it ends.
    notice: Option<(Notice, Instant)>,
}

/// A step of a scripted timeline: at `at` after the start, show this.
#[derive(Clone)]
struct Step {
    at: Duration,
    phase: PillPhase,
    activity: Activity,
    notice: Option<&'static str>,
    words: Option<u32>,
    visible: bool,
    sound: bool,
}

const fn step(at_ms: u64, phase: PillPhase) -> Step {
    Step {
        at: Duration::from_millis(at_ms),
        phase,
        activity: Activity::Dictate,
        notice: None,
        words: None,
        visible: true,
        sound: false,
    }
}

fn dictation_script() -> Vec<Step> {
    vec![
        step(0, PillPhase::Arming),
        step(60, PillPhase::Listening),
        Step {
            sound: true,
            ..step(700, PillPhase::Listening)
        },
        step(3200, PillPhase::Transcribing),
        Step {
            words: Some(12),
            ..step(4300, PillPhase::Inserted)
        },
        Step {
            visible: false,
            ..step(5000, PillPhase::Hidden)
        },
    ]
}

fn cycle_script(every: Duration) -> Vec<Step> {
    let states = [
        step(0, PillPhase::Arming),
        step(0, PillPhase::Listening),
        Step {
            sound: true,
            ..step(0, PillPhase::Listening)
        },
        Step {
            notice: Some("pill.noAudio"),
            ..step(0, PillPhase::Listening)
        },
        Step {
            activity: Activity::Rewrite,
            sound: true,
            ..step(0, PillPhase::Listening)
        },
        Step {
            notice: Some("pill.selectTextBox"),
            ..step(0, PillPhase::Warning)
        },
        step(0, PillPhase::Transcribing),
        Step {
            notice: Some("pill.loadingModel"),
            ..step(0, PillPhase::Transcribing)
        },
        Step {
            activity: Activity::Rewrite,
            ..step(0, PillPhase::Rewriting)
        },
        Step {
            words: Some(12),
            ..step(0, PillPhase::Inserted)
        },
        Step {
            activity: Activity::Rewrite,
            ..step(0, PillPhase::Rewrote)
        },
        step(0, PillPhase::NothingHeard),
        Step {
            notice: Some("pill.error.offline"),
            ..step(0, PillPhase::Error)
        },
        Step {
            notice: Some("pill.error.noKey"),
            ..step(0, PillPhase::Error)
        },
        Step {
            visible: false,
            ..step(0, PillPhase::Hidden)
        },
    ];
    states
        .into_iter()
        .enumerate()
        .map(|(i, s)| Step {
            at: every * i as u32,
            ..s
        })
        .collect()
}

struct Script {
    steps: Vec<Step>,
    started: Instant,
    next: usize,
    repeat: Duration,
}

struct State {
    pill: PillSnapshot,
    acked: u64,
    settings_seq: u64,
    settings: Value,
    meeting: Option<Meeting>,
    meetings_started: u32,
    script: Option<Script>,
    sound: bool,
    /// The model a simulated download fetches, and since when.
    download: Option<(&'static str, Instant)>,
    start_page: Option<String>,
    stop_clicks: u32,
    screen_clicks: u32,
}

/// Output gathered under the lock and sent after it, so the sink never runs while the state is locked.
enum Out {
    Pill(PillSnapshot),
    Level(f32),
    Settings(Value),
    Tray(TrayState),
    Ui,
}

pub struct FakeEngine {
    sink: Arc<dyn UiSink>,
    data_dir: PathBuf,
    dev_mode: bool,
    timing: Timing,
    state: Mutex<State>,
    stopped: AtomicBool,
    born: Instant,
}

impl FakeEngine {
    pub fn start(
        options: EngineOptions,
        sink: Arc<dyn UiSink>,
        scenario: &str,
    ) -> Result<Arc<FakeEngine>, ApiError> {
        Self::start_with(options, sink, scenario, Timing::default())
    }

    pub fn start_with(
        options: EngineOptions,
        sink: Arc<dyn UiSink>,
        scenario: &str,
        timing: Timing,
    ) -> Result<Arc<FakeEngine>, ApiError> {
        if !SCENARIOS.contains(&scenario) {
            return Err(ApiError::new(
                "invalid_value",
                format!(
                    "Unknown fake engine scenario {scenario}; use one of {}.",
                    SCENARIOS.join(", ")
                ),
            ));
        }
        let settings = base_settings(&options.data_dir, options.dev_mode, &options.version);
        let engine = Arc::new(FakeEngine {
            sink,
            data_dir: options.data_dir.clone(),
            dev_mode: options.dev_mode,
            timing,
            state: Mutex::new(State {
                pill: PillSnapshot {
                    seq: 0,
                    visible: false,
                    phase: PillPhase::Hidden,
                    activity: Activity::Dictate,
                    notice: None,
                    words: None,
                    meeting: None,
                    edge: Edge::Bottom,
                    theme: Theme::Dark,
                    locale: Locale::En,
                    dev: options.dev_mode,
                },
                acked: 0,
                settings_seq: 0,
                settings,
                meeting: None,
                meetings_started: 0,
                script: None,
                sound: false,
                download: None,
                start_page: (scenario == "first-run").then(|| "import".to_string()),
                stop_clicks: 0,
                screen_clicks: 0,
            }),
            stopped: AtomicBool::new(false),
            born: Instant::now(),
        });
        engine.apply(|st, out| {
            st.restyle(out);
            st.settings_changed(out);
            out.push(Out::Tray(st.tray()));
        });
        match scenario {
            "meeting" => {
                // The meeting records the screen as frames, as Settings says.
                engine.apply(|st, out| {
                    st.settings["config"]["meetings"]["screen"] = json!("frames");
                    st.settings_changed(out);
                });
                // Nemotron shared from Ownkey's folder, so the meeting labels its speakers.
                let path = options
                    .data_dir
                    .join("ownkey-models")
                    .join("nemotron-3-diarization");
                engine.apply(|st, _| {
                    st.settings["models"]["nemotron"] = json!({
                        "id": "nemotron", "name": "Nemotron 3 Diarization", "sizeBytes": 107_000_000u64,
                        "stage": "installed", "percent": null, "location": "ownkey", "path": path,
                        "removable": false, "active": null, "error": null
                    });
                });
                engine.start_meeting()?;
            }
            "dictation" => engine.apply(|st, _| {
                st.script = Some(Script {
                    steps: dictation_script(),
                    started: Instant::now(),
                    next: 0,
                    repeat: engine.timing.dictation_every,
                })
            }),
            "cycle" => engine.apply(|st, _| {
                let steps = cycle_script(engine.timing.cycle_step);
                let repeat = engine.timing.cycle_step * (steps.len() as u32 + 1);
                st.script = Some(Script {
                    steps,
                    started: Instant::now(),
                    next: 0,
                    repeat,
                })
            }),
            _ => {}
        }
        let ticker = Arc::downgrade(&engine);
        thread::Builder::new()
            .name("fake-engine".into())
            .spawn(move || loop {
                thread::sleep(TICK);
                let Some(engine) = ticker.upgrade() else {
                    return;
                };
                if engine.stopped.load(Ordering::Relaxed) {
                    return;
                }
                engine.tick();
            })
            .map_err(|e| {
                ApiError::new("failed", format!("Could not start the fake engine: {e}"))
            })?;
        Ok(engine)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn apply<R>(&self, f: impl FnOnce(&mut State, &mut Vec<Out>) -> R) -> R {
        let mut out = Vec::new();
        let result = {
            let mut st = self.lock();
            f(&mut st, &mut out)
        };
        for o in out {
            match o {
                Out::Pill(s) => self.sink.pill(s),
                Out::Level(l) => self.sink.level(l),
                Out::Settings(v) => self.sink.settings(v),
                Out::Tray(t) => self.sink.tray(t),
                Out::Ui => self.sink.ui_changed(),
            }
        }
        result
    }

    /// How often the stop window's clicks reached the engine.
    #[cfg(test)]
    pub fn stop_clicks(&self) -> u32 {
        self.lock().stop_clicks
    }

    /// How often the screen window's clicks reached the engine.
    #[cfg(test)]
    pub fn screen_clicks(&self) -> u32 {
        self.lock().screen_clicks
    }

    fn tick(&self) {
        let now = Instant::now();
        let timing = self.timing;
        self.apply(|st, out| {
            if let Some(m) = st.meeting.as_mut() {
                let mut changed = false;
                if m.confirm_until.is_some_and(|until| now >= until) {
                    m.confirm_until = None;
                    changed = true;
                }
                if m.run == Run::Recording && m.compact_at.is_some_and(|at| now >= at) {
                    m.compact_at = None;
                    m.compact = true;
                    changed = true;
                }
                if now >= m.resend_at {
                    m.resend_at = now + timing.resend;
                    changed = true;
                }
                if m.notice.as_ref().is_some_and(|(_, until)| now >= *until) {
                    m.notice = None;
                    changed = true;
                }
                if let Run::Saving { until } = m.run {
                    if now >= until {
                        st.meeting = None;
                        st.pill_change(out, |p| {
                            p.visible = false;
                            p.phase = PillPhase::Hidden;
                            p.meeting = None;
                            p.notice = None;
                        });
                        out.push(Out::Tray(st.tray()));
                        st.settings_changed(out);
                        changed = false;
                    }
                }
                if changed {
                    st.meeting_pill(out);
                }
            }
            st.play_script(now, out);
            if st.pill.visible && st.pill.phase == PillPhase::Listening {
                let level = if st.sound {
                    let t = now.duration_since(self.born).as_secs_f32();
                    (0.45 + 0.35 * (t * 5.3).sin() * (t * 1.7).cos()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                out.push(Out::Level(level));
            }
            if let Some((id, started)) = st.download {
                let percent = (started.elapsed().as_millis() / 30).min(100) as u64;
                let model = &mut st.settings["models"][id];
                if percent < 100 {
                    if model["percent"] != json!(percent) {
                        model["stage"] = json!("downloading");
                        model["percent"] = json!(percent);
                        st.settings_changed(out);
                    }
                } else {
                    *model = installed_model(id, &self.data_dir);
                    st.download = None;
                    st.settings_changed(out);
                }
            }
        });
    }
}

impl State {
    /// A changed connection field or a new or removed secret: uploads pause until Test connection
    /// passes again, and the switch stays where the user put it.
    fn untest_sync(&mut self) {
        let sync = &mut self.settings["config"]["storage"]["sync"];
        sync["tested"] = json!(false);
        if sync["enabled"] == true {
            self.settings["sync"]["state"] = json!("paused");
        }
    }

    fn pill_change(&mut self, out: &mut Vec<Out>, f: impl FnOnce(&mut PillSnapshot)) {
        f(&mut self.pill);
        self.pill.seq += 1;
        out.push(Out::Pill(self.pill.clone()));
    }

    fn ui(&self) -> UiOptions {
        let ui = &self.settings["config"]["ui"];
        UiOptions {
            theme: resolve_theme(ui["theme"].as_str().unwrap_or("system")),
            theme_follows_system: ui["theme"].as_str().unwrap_or("system") == "system",
            locale: resolve_locale(ui["language"].as_str().unwrap_or("system")),
            pill_edge: if ui["pillEdge"] == "top" {
                Edge::Top
            } else {
                Edge::Bottom
            },
            hide_pill_from_capture: ui["hidePillFromCapture"].as_bool().unwrap_or(true),
            dev: self.pill.dev,
        }
    }

    /// Theme, language or edge changed: every window follows.
    fn restyle(&mut self, out: &mut Vec<Out>) {
        let ui = self.ui();
        self.settings["theme"] = json!(if ui.theme == Theme::Light {
            "light"
        } else {
            "dark"
        });
        self.settings["locale"] = json!(if ui.locale == Locale::Nl { "nl" } else { "en" });
        self.pill_change(out, |p| {
            p.theme = ui.theme;
            p.locale = ui.locale;
            p.edge = ui.pill_edge;
        });
        out.push(Out::Ui);
    }

    fn settings_changed(&mut self, out: &mut Vec<Out>) {
        self.settings_seq += 1;
        self.settings["seq"] = json!(self.settings_seq);
        self.settings["speakers"] = self.speakers();
        out.push(Out::Settings(self.settings.clone()));
    }

    /// Settings > Speaker labels as the real engine reports it, for a computer that can run
    /// Nemotron once its model is there.
    fn speakers(&self) -> Value {
        let meetings = &self.settings["config"]["meetings"];
        let on = meetings["liveSpeakers"].as_bool().unwrap_or(true);
        let mistral = meetings["provider"] == "mistral"
            && meetings["model"]
                .as_str()
                .is_some_and(|m| m.starts_with(REALTIME_PREFIX));
        let installed = self.settings["models"]["nemotron"]["stage"] == "installed";
        let reason = if !on {
            Some("off")
        } else if mistral {
            (meetings["consent"]["mode"] != "stream").then_some("consent")
        } else if !installed {
            Some("model")
        } else {
            None
        };
        let ready = reason.is_none();
        let live: Vec<Value> = self
            .meeting
            .iter()
            .map(|m| {
                let (mode, state) = match (mistral, ready) {
                    (true, true) => ("mistral", "listening"),
                    (true, false) => ("mistral", "starting"),
                    (false, true) => ("nemotron", "listening"),
                    (false, false) => ("off", "off"),
                };
                json!({
                    "meeting": m.id,
                    "mode": mode,
                    "state": state,
                    "speakers": if ready { 2 } else { 0 },
                    "reason": if mode == "off" { reason } else { None },
                    "workerPid": null,
                })
            })
            .collect();
        json!({
            "engine": if mistral { "mistral" } else { "nemotron" },
            "ready": ready,
            "reason": reason,
            "detail": null,
            "missing": [],
            "cpu": { "avx2": true, "fma": true, "f16c": true, "bmi2": true },
            "runtime": installed.then(|| json!({ "location": "voice", "commit": "97a15af" })),
            "live": live,
        })
    }

    /// Whether the meeting that records labels its speakers with Nemotron.
    fn labelling(&self) -> bool {
        self.speakers()["live"]
            .as_array()
            .is_some_and(|live| live.iter().any(|l| l["mode"] == "nemotron"))
    }

    fn tray(&self) -> TrayState {
        match self.meeting.as_ref() {
            None => TrayState::default(),
            Some(m) => TrayState {
                meeting: Some(
                    match m.run {
                        Run::Recording => "recording",
                        Run::Paused => "paused",
                        Run::Saving { .. } => "processing",
                    }
                    .to_string(),
                ),
                meeting_at_ms: Some(m.started.elapsed().as_millis() as u64),
                status: None,
            },
        }
    }

    fn meeting_pill(&mut self, out: &mut Vec<Out>) {
        let Some(m) = self.meeting.as_ref() else {
            return;
        };
        let dictating =
            self.script.is_some() && self.pill.phase != PillPhase::Meeting && self.pill.visible;
        let live = matches!(m.run, Run::Recording | Run::Paused);
        let screen = m.screen.clone();
        let meeting = MeetingPill {
            state: match m.run {
                Run::Recording => "recording",
                Run::Paused => "paused",
                Run::Saving { .. } => "processing",
            }
            .to_string(),
            at_ms: m.started.elapsed().as_millis() as u64,
            screen_switch: live,
            screen,
            compact: m.compact && m.run == Run::Recording && m.notice.is_none(),
            confirm_stop: m.confirm_until.is_some(),
            stop_enabled: !dictating && live,
            rukoo_connected: false,
        };
        let notice = m.notice.as_ref().map(|(notice, _)| notice.clone());
        self.pill_change(out, |p| {
            if !dictating {
                p.visible = true;
                p.phase = PillPhase::Meeting;
                p.notice = notice;
                p.words = None;
                p.activity = Activity::Dictate;
            }
            p.meeting = Some(meeting);
        });
    }

    fn play_script(&mut self, now: Instant, out: &mut Vec<Out>) {
        let Some(script) = self.script.as_mut() else {
            return;
        };
        let mut elapsed = now.duration_since(script.started);
        if elapsed >= script.repeat {
            script.started = now;
            script.next = 0;
            elapsed = Duration::ZERO;
        }
        let mut due = None;
        while script.next < script.steps.len() && script.steps[script.next].at <= elapsed {
            due = Some(script.steps[script.next].clone());
            script.next += 1;
        }
        let Some(s) = due else { return };
        self.sound = s.sound;
        let in_meeting = self.meeting.is_some();
        if !s.visible && in_meeting {
            // Back to the meeting after the dictation, as the engine does.
            self.meeting_pill(out);
            return;
        }
        let screen = self.settings["config"]["meetings"]["screen"]
            .as_str()
            .unwrap_or("off")
            .to_string();
        let meeting = self.meeting.as_ref().map(|m| MeetingPill {
            state: "recording".to_string(),
            at_ms: m.started.elapsed().as_millis() as u64,
            screen,
            screen_switch: false,
            compact: true,
            confirm_stop: false,
            stop_enabled: false,
            rukoo_connected: false,
        });
        self.pill_change(out, |p| {
            p.visible = s.visible;
            p.phase = s.phase;
            p.activity = s.activity;
            p.notice = s.notice.map(|key| Notice {
                key: key.to_string(),
                args: Map::new(),
            });
            p.words = s.words;
            p.meeting = meeting;
        });
    }
}

fn resolve_theme(setting: &str) -> Theme {
    match setting {
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        _ if crate::system::prefers_light_theme() => Theme::Light,
        _ => Theme::Dark,
    }
}

fn resolve_locale(setting: &str) -> Locale {
    match setting {
        "nl" => Locale::Nl,
        "en" => Locale::En,
        _ => crate::system::locale(),
    }
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new("invalid_value", message)
}

/// The host of an endpoint URL, for consent (decision 10).
pub fn host_of(endpoint: &str) -> String {
    let rest = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host.to_ascii_lowercase()
}

fn set_path(root: &mut Value, dotted: &str, value: Value) {
    let mut node = root;
    let mut parts = dotted.split('.').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            node[part] = value;
            return;
        }
        node = &mut node[part];
    }
}

fn installed_model(id: &str, data_dir: &Path) -> Value {
    if id == "nemotron" {
        return json!({
            "id": "nemotron", "name": "Nemotron 3 Diarization", "sizeBytes": 107_000_000u64,
            "stage": "installed", "percent": null,
            "location": "voice", "path": data_dir.join("models").join("nemotron-3-diarization"),
            "removable": true, "active": null, "error": null
        });
    }
    json!({
        "id": "orukeet", "name": "Orukeet", "sizeBytes": 487_000_000u64,
        "stage": "installed", "percent": null,
        "location": "voice", "path": data_dir.join("models").join("orukeet-onnx-int8"),
        "removable": true, "active": "unloaded", "error": null
    })
}

fn base_settings(data_dir: &Path, dev_mode: bool, version: &str) -> Value {
    let discovery = voice_protocol::paths::bridge_path_here(voice_protocol::Side::Voice)
        .unwrap_or_else(|| data_dir.join("bridge.json"));
    let (dictate, rewrite) = if dev_mode {
        ("f13", "f14")
    } else {
        ("right alt", "right ctrl")
    };
    json!({
        "seq": 0,
        "version": version,
        "devMode": dev_mode,
        "platform": if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" },
        // Every feature is in this Voice: screen recording, speaker labels and bucket sync.
        "features": { "screen": true, "speakers": true, "sync": true },
        "theme": "dark",
        "locale": "en",
        "config": {
            "dictation": { "hotkey": dictate, "rewriteHotkey": rewrite, "pasteMode": true, "readyChime": false, "startAtLogin": !dev_mode },
            "transcription": { "provider": "orukeet", "endpoint": "", "model": "orukeet-onnx-int8", "language": "auto", "keySaved": false },
            "local": { "modelFolder": "", "idleMinutes": 20 },
            "dictionary": { "vocabulary": ["Rukoo", "Orukeet"], "corrections": [{ "from": "rocco", "to": "Rukoo" }] },
            "fillers": { "enabled": true, "languages": ["en", "nl"], "custom": "" },
            "rewriting": { "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/chat/completions", "model": "mistral-small-latest", "keySaved": false, "auto": false, "tone": "auto", "formatting": true, "instructions": "" },
            "meetings": { "provider": "orukeet", "endpoint": "", "model": "orukeet-onnx-int8", "keySaved": false, "consent": null, "retention": "days7", "stopKey": "off", "screen": "off", "liveSpeakers": true },
            "ui": { "language": "system", "theme": "system", "pillEdge": "bottom", "hidePillFromCapture": true },
            "updates": { "check": false },
            "storage": { "sync": { "enabled": false, "service": "upcloud", "endpoint": "", "region": "", "bucket": "", "prefix": "", "accessKeyId": "", "retention": "keep", "secretSaved": false, "tested": false, "secret": "" } }
        },
        "providers": {
            "audio": [
                { "id": "openai", "endpoints": ["https://api.openai.com/v1/audio/transcriptions"], "streamingPrefix": null },
                { "id": "google", "endpoints": ["https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"], "streamingPrefix": null },
                { "id": "mistral", "endpoints": ["https://api.mistral.ai/v1/audio/transcriptions"], "streamingPrefix": REALTIME_PREFIX },
                { "id": "custom", "endpoints": ["http://localhost:1234/v1/audio/transcriptions"], "streamingPrefix": null },
                { "id": "orukeet", "endpoints": [], "streamingPrefix": null }
            ],
            "rewrite": [
                { "id": "openai", "endpoints": ["https://api.openai.com/v1/responses", "https://api.openai.com/v1/chat/completions"] },
                { "id": "anthropic", "endpoints": ["https://api.anthropic.com/v1/messages"] },
                { "id": "google", "endpoints": ["https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"] },
                { "id": "mistral", "endpoints": ["https://api.mistral.ai/v1/chat/completions"] },
                { "id": "ollama", "endpoints": ["http://localhost:11434/api/chat", "https://ollama.com/api/chat"] },
                { "id": "openrouter", "endpoints": ["https://openrouter.ai/api/v1/chat/completions"] },
                { "id": "custom", "endpoints": ["http://localhost:1234/v1/chat/completions"] }
            ]
        },
        "dictation": { "status": "ready" },
        "rukoo": { "state": "notRunning", "clientVersion": null, "protocol": null, "port": null, "discoveryPath": discovery, "behind": null },
        "models": {
            "orukeet": installed_model("orukeet", data_dir),
            "nemotron": { "id": "nemotron", "name": "Nemotron 3 Diarization", "sizeBytes": 107_000_000u64, "stage": "missing", "percent": null, "location": null, "path": null, "removable": false, "active": null, "error": null }
        },
        // Filled in from the rest of the state each time it goes out (State::speakers).
        "speakers": null,
        "storage": { "folder": data_dir.join("meetings"), "usedBytes": 0, "freeBytes": null },
        "screen": { "available": true, "hardwareEncoder": "NVIDIA H.264 Encoder MFT", "ocr": ["en-US", "nl-NL"], "monitors": MONITORS.len() },
        "updates": { "state": "idle", "version": null },
        "ownkey": { "running": false, "keys": [], "configFound": false, "configPath": "%APPDATA%\\Ownkey\\config.json", "lastImport": null, "models": [], "report": null },
        "sync": {
            "available": true, "state": "off", "waiting": 0, "waitingBytes": 0, "oldestWaiting": null,
            "lastUpload": null, "lastError": null, "failingSince": null, "lost": 0, "screenRecording": false,
            "key": { "exists": false, "id": null, "exported": false },
            "restore": { "state": "idle", "available": 0, "done": 0, "total": 0, "error": null },
            "otherKeys": 0
        }
    })
}

/// The recovery code the fake shows: zeros, never a key.
pub const FAKE_RECOVERY_CODE: &str =
    "RKV1-00000-00000-00000-00000-00000-00000-00000-00000-00000-00000-00000";

/// The meetings the fake's bucket holds that aren't "on this computer".
fn fake_remote_meetings() -> Value {
    json!([
        { "id": "m-20261028-140000-a1b2", "title": "Design review", "startedAt": 1_793_196_000_000u64, "duration": 2_712_000 },
        { "id": "m-20261021-090000-c3d4", "title": "Weekly", "startedAt": 1_792_573_200_000u64, "duration": 1_800_000 }
    ])
}

/// Bucket sync's connection fields: changing one needs a new Test connection.
const SYNC_FIELDS: &[&str] = &[
    "storage.sync.service",
    "storage.sync.endpoint",
    "storage.sync.region",
    "storage.sync.bucket",
    "storage.sync.prefix",
    "storage.sync.accessKeyId",
];

fn models_for(provider: &str) -> Option<Vec<&'static str>> {
    Some(match provider {
        "mistral" => vec![
            "mistral-small-latest",
            "voxtral-mini-latest",
            "voxtral-mini-transcribe-realtime-3",
            "mistral-medium-latest",
            "voxtral-small-latest",
        ],
        "openai" => vec!["whisper-1", "gpt-4o-mini-transcribe", "gpt-4o-transcribe"],
        "google" => vec!["gemini-2.5-flash", "gemini-2.5-pro"],
        "anthropic" => vec!["claude-haiku-4-5", "claude-sonnet-4-5"],
        "openrouter" => vec!["openai/gpt-4o-mini", "mistralai/mistral-small"],
        "orukeet" => vec!["orukeet-onnx-int8"],
        _ => return None,
    })
}

impl EngineApi for FakeEngine {
    fn pill_snapshot(&self) -> PillSnapshot {
        self.lock().pill.clone()
    }

    fn pill_ack(&self, seq: u64) {
        let mut st = self.lock();
        st.acked = st.acked.max(seq);
    }

    fn pill_screen_clicked(&self) {
        let info = self.apply(|st, _| {
            st.screen_clicks += 1;
            st.meeting.as_ref().filter(|m| matches!(m.run, Run::Recording | Run::Paused)).map(|m| json!({
                "meeting": m.id, "screen": m.screen, "monitor": MONITORS[m.monitor],
                "monitors": MONITORS.iter().map(|name| json!({"id": name, "name": name})).collect::<Vec<_>>()
            }))
        });
        if let Some(info) = info {
            self.sink.screen_menu(info);
        }
    }

    fn pill_screen_set(
        &self,
        meeting: &str,
        mode: &str,
        monitor: Option<&str>,
    ) -> Result<(), ApiError> {
        if !matches!(mode, "off" | "frames" | "video") {
            return Err(ApiError::new("bad_args", "Unknown screen mode."));
        }
        self.apply(|st, out| {
            let m = st
                .meeting
                .as_mut()
                .filter(|m| m.id == meeting && matches!(m.run, Run::Recording | Run::Paused))
                .ok_or_else(|| ApiError::new("not_recording", "The meeting stopped."))?;
            if let Some(monitor) = monitor {
                m.monitor = MONITORS
                    .iter()
                    .position(|name| *name == monitor)
                    .ok_or_else(|| ApiError::new("bad_args", "Unknown monitor."))?;
                m.notice = Some((
                    Notice {
                        key: "pill.screenMoved".into(),
                        args: Map::from_iter([("name".into(), json!(MONITORS[m.monitor]))]),
                    },
                    Instant::now() + self.timing.notice,
                ));
            }
            m.screen = mode.into();
            st.meeting_pill(out);
            Ok(())
        })
    }

    fn pill_stop_clicked(&self) {
        let now = Instant::now();
        let timing = self.timing;
        let stop = self.apply(|st, out| {
            st.stop_clicks += 1;
            let Some(m) = st.meeting.as_mut() else {
                return false;
            };
            if !matches!(m.run, Run::Recording | Run::Paused) {
                return false;
            }
            if m.confirm_until.is_some_and(|until| now < until) {
                return true;
            }
            m.confirm_until = Some(now + timing.confirm);
            st.meeting_pill(out);
            false
        });
        if stop {
            let _ = self.stop_meeting(StopReason::User);
        }
    }

    fn settings_state(&self) -> Value {
        self.lock().settings.clone()
    }

    fn settings_set(&self, key: &str, value: Value) -> Result<(), ApiError> {
        let dev_mode = self.dev_mode;
        self.apply(|st, out| {
            let config = &st.settings["config"];
            let hotkeys = |except: &str| -> Vec<String> {
                [
                    "dictation.hotkey",
                    "dictation.rewriteHotkey",
                    "meetings.stopKey",
                ]
                .iter()
                .filter(|k| **k != except)
                .filter_map(|k| {
                    let (group, name) = k.split_once('.').unwrap();
                    config[group][name].as_str().map(str::to_string)
                })
                .collect()
            };
            let one_of = |options: &[&str], allow_off: bool| -> Result<(), ApiError> {
                match value.as_str() {
                    Some(v) if options.contains(&v) || (allow_off && v == "off") => Ok(()),
                    _ => Err(invalid(format!("{key} can't be {value}"))),
                }
            };
            let boolean = || {
                value
                    .as_bool()
                    .map(|_| ())
                    .ok_or_else(|| invalid(format!("{key} needs true or false")))
            };
            let string = || {
                value
                    .as_str()
                    .map(|_| ())
                    .ok_or_else(|| invalid(format!("{key} needs text")))
            };
            match key {
                "dictation.hotkey" | "dictation.rewriteHotkey" | "meetings.stopKey" => {
                    one_of(KEYS, key != "dictation.hotkey")?;
                    let v = value.as_str().unwrap_or_default();
                    if v != "off" && hotkeys(key).iter().any(|k| k == v) {
                        return Err(ApiError::new("same_key", format!("{v} is already in use")));
                    }
                }
                "dictation.startAtLogin" => {
                    if dev_mode {
                        return Err(ApiError::new(
                            "dev_mode",
                            "A development copy never starts at login.",
                        ));
                    }
                    boolean()?;
                }
                "dictation.pasteMode"
                | "dictation.readyChime"
                | "ui.hidePillFromCapture"
                | "fillers.enabled"
                | "rewriting.auto"
                | "rewriting.formatting"
                | "meetings.liveSpeakers"
                | "updates.check" => boolean()?,
                "storage.sync.enabled" => {
                    boolean()?;
                    if value == true && config["storage"]["sync"]["tested"] != true {
                        return Err(ApiError::new(
                            "needs_test",
                            "Test the connection before you turn sync on.",
                        ));
                    }
                }
                "storage.sync.secret" => {
                    return Err(invalid("Save the secret with settings_sync_save_secret."))
                }
                "storage.sync.service" => one_of(&["upcloud", "r2", "other"], false)?,
                "storage.sync.retention" => one_of(&["keep", "days365", "days90"], false)?,
                "ui.pillEdge" => one_of(&["bottom", "top"], false)?,
                "ui.language" => one_of(&["system", "en", "nl"], false)?,
                "ui.theme" => one_of(&["system", "light", "dark"], false)?,
                "rewriting.tone" => one_of(
                    &["auto", "professional", "casual", "friendly", "concise"],
                    false,
                )?,
                "meetings.retention" => one_of(&["days7", "keep", "after_transcription"], false)?,
                "meetings.screen" => one_of(&["off", "frames", "video"], false)?,
                "local.modelFolder" | "fillers.custom" | "rewriting.instructions" => string()?,
                k if SYNC_FIELDS.contains(&k) => string()?,
                "local.idleMinutes" => match value.as_u64() {
                    Some(m) if m <= 1440 => {}
                    _ => return Err(invalid("Idle minutes go from 0 to 1440.")),
                },
                "fillers.languages" => {
                    let ok = value.as_array().is_some_and(|l| {
                        l.iter()
                            .all(|c| c.as_str().is_some_and(|c| FILLER_LANGUAGES.contains(&c)))
                    });
                    if !ok {
                        return Err(invalid("Filler languages are en, nl, de, fr and es."));
                    }
                }
                "dictionary.vocabulary" => {
                    if !value
                        .as_array()
                        .is_some_and(|l| l.iter().all(Value::is_string))
                    {
                        return Err(invalid("The vocabulary is a list of words."));
                    }
                }
                "dictionary.corrections" => {
                    let ok = value.as_array().is_some_and(|l| {
                        l.iter()
                            .all(|c| c["from"].is_string() && c["to"].is_string())
                    });
                    if !ok {
                        return Err(invalid("Corrections are a list of { from, to }."));
                    }
                }
                _ => return Err(invalid(format!("{key} can't be set here"))),
            }
            if SYNC_FIELDS.contains(&key) {
                let (_, field) = key.rsplit_once('.').unwrap_or_default();
                if st.settings["config"]["storage"]["sync"][field] != value {
                    st.untest_sync();
                }
            }
            set_path(&mut st.settings["config"], key, value.clone());
            if key == "storage.sync.enabled" {
                st.settings["sync"]["state"] = json!(if value == true { "idle" } else { "off" });
            }
            if matches!(
                key,
                "ui.theme" | "ui.language" | "ui.pillEdge" | "ui.hidePillFromCapture"
            ) {
                st.restyle(out);
            }
            st.settings_changed(out);
            Ok(())
        })
    }

    fn settings_save_provider(&self, save: Value) -> Result<(), ApiError> {
        self.apply(|st, out| {
            let target = save["target"].as_str().unwrap_or_default();
            let kind = match target {
                "transcription" | "meetings" => "audio",
                "rewriting" => "rewrite",
                _ => return Err(invalid(format!("Unknown provider form {target}"))),
            };
            let provider = save["provider"].as_str().unwrap_or_default().to_string();
            let known = st.settings["providers"][kind]
                .as_array()
                .is_some_and(|l| l.iter().any(|p| p["id"] == provider.as_str()));
            if !known || (target == "rewriting" && provider == "orukeet") {
                return Err(invalid(format!("Unknown provider {provider}")));
            }
            let (endpoint, model) = if provider == "orukeet" {
                (String::new(), "orukeet-onnx-int8".to_string())
            } else {
                let endpoint = save["endpoint"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let model = save["model"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if endpoint.is_empty() || model.is_empty() {
                    return Err(ApiError::new(
                        "need_endpoint_model",
                        "A cloud provider needs an endpoint and a model.",
                    ));
                }
                (endpoint, model)
            };
            if target != "meetings" && model.starts_with(REALTIME_PREFIX) {
                return Err(invalid(
                    "Realtime models work for meeting transcription only.",
                ));
            }
            let form = &mut st.settings["config"][target];
            let same = form["provider"] == provider.as_str();
            let key_saved = match &save["apiKey"] {
                Value::Null => same && form["keySaved"].as_bool().unwrap_or(false),
                Value::String(key) => !key.is_empty(),
                _ => return Err(invalid("apiKey is text or null")),
            };
            form["provider"] = json!(provider);
            form["endpoint"] = json!(endpoint);
            form["model"] = json!(model);
            form["keySaved"] = json!(key_saved && provider != "orukeet");
            if target == "transcription" {
                let language = if provider == "orukeet" {
                    "auto"
                } else {
                    save["language"].as_str().unwrap_or("auto")
                };
                form["language"] = json!(language);
            }
            if target == "meetings" {
                form["consent"] = if save["consent"] == true && provider != "orukeet" {
                    // Always with `mode`, as the real engine sends it (API.md, `meetings.consent`).
                    let mode = if model.starts_with(REALTIME_PREFIX) {
                        "stream"
                    } else {
                        "windows"
                    };
                    json!({ "provider": provider, "host": host_of(&endpoint), "mode": mode })
                } else {
                    Value::Null
                };
            }
            st.settings_changed(out);
            Ok(())
        })
    }

    fn settings_remove_key(&self, target: &str) -> Result<(), ApiError> {
        self.apply(|st, out| {
            if target == "sync" {
                st.untest_sync();
                st.settings["config"]["storage"]["sync"]["secretSaved"] = json!(false);
                st.settings_changed(out);
                return Ok(());
            }
            if !matches!(target, "transcription" | "rewriting" | "meetings") {
                return Err(invalid(format!("Unknown provider form {target}")));
            }
            st.settings["config"][target]["keySaved"] = json!(false);
            st.settings_changed(out);
            Ok(())
        })
    }

    fn settings_list_models(&self, request: Value) -> Result<Vec<String>, ApiError> {
        let provider = request["provider"].as_str().unwrap_or_default();
        let endpoint = request["endpoint"].as_str().unwrap_or_default();
        let mut models: Vec<String> = models_for(provider)
            .ok_or_else(|| {
                ApiError::new(
                    "network",
                    format!("Could not reach {endpoint} (fake engine)."),
                )
            })?
            .into_iter()
            .map(str::to_string)
            .collect();
        models.sort();
        models.dedup();
        Ok(models)
    }

    fn settings_model(&self, model: &str, action: &str) -> Result<(), ApiError> {
        self.apply(|st, out| {
            let id = match model {
                "orukeet" => "orukeet",
                "nemotron" => "nemotron",
                _ => return Err(ApiError::new("not_found", format!("Unknown model {model}"))),
            };
            let labelling = id == "nemotron" && st.labelling();
            let downloading = st.download.is_some_and(|(d, _)| d == id);
            let state = &mut st.settings["models"][id];
            match action {
                "download" => {
                    if state["stage"] == "installed" || downloading {
                        return Ok(());
                    }
                    st.download = Some((id, Instant::now()));
                }
                "cancel" => {
                    state["stage"] = json!("missing");
                    state["percent"] = Value::Null;
                    if downloading {
                        st.download = None;
                    }
                }
                "remove" => {
                    if state["removable"] != true {
                        return Err(ApiError::new("not_removable", "This model isn't in Voice's own folder."));
                    }
                    if labelling {
                        return Err(ApiError::new("busy", "A meeting is labelling speakers with this model."));
                    }
                    *state = json!({ "id": model, "name": state["name"], "sizeBytes": state["sizeBytes"], "stage": "missing", "percent": null,
                        "location": null, "path": null, "removable": false, "active": null, "error": null });
                }
                _ => return Err(invalid(format!("Unknown model action {action}"))),
            }
            st.settings_changed(out);
            Ok(())
        })
    }

    fn settings_set_folder(&self, purpose: &str, path: PathBuf) -> Result<(), ApiError> {
        self.apply(|st, out| {
            match purpose {
                "models" => st.settings["config"]["local"]["modelFolder"] = json!(path),
                // As the real engine: 0.1 keeps meetings in the data folder.
                _ => return Err(invalid(format!("Unknown folder purpose {purpose}"))),
            }
            st.settings_changed(out);
            Ok(())
        })
    }

    fn settings_clean_preview(&self, preview: Value) -> Result<String, ApiError> {
        let text = preview["text"]
            .as_str()
            .ok_or_else(|| invalid("preview.text is missing"))?;
        let cfg = voice_core::cleanup::CleanupConfig {
            remove_fillers: preview["enabled"].as_bool().unwrap_or(true),
            filler_languages: preview["languages"]
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(|c| c.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            custom_fillers: preview["custom"].as_str().unwrap_or_default().to_string(),
            corrections: Vec::new(),
        };
        Ok(voice_core::cleanup::clean(text, &cfg))
    }

    fn settings_check_updates(&self) -> Value {
        // Never on the network: the fake answers as an up-to-date copy.
        self.apply(|st, out| {
            st.settings["updates"] = json!({ "state": "upToDate", "version": null });
            st.settings_changed(out);
            st.settings["updates"].clone()
        })
    }

    fn settings_new_token(&self) -> Result<(), ApiError> {
        Ok(())
    }

    fn settings_import_ownkey(&self) -> Result<Value, ApiError> {
        Err(ApiError::new(
            "dev_mode",
            "A development copy imports Ownkey's settings only from a copy, with --import-ownkey <path>.",
        ))
    }

    fn settings_sync_test(&self) -> Result<Value, ApiError> {
        self.apply(|st, out| {
            let sync = &mut st.settings["config"]["storage"]["sync"];
            if sync["secretSaved"] != true {
                return Err(ApiError::new(
                    "no_secret",
                    "Save the secret access key first.",
                ));
            }
            if sync["bucket"] == "fail" {
                return Err(ApiError::new(
                    "bucket",
                    "The bucket answered NoSuchBucket for fail.",
                ));
            }
            sync["tested"] = json!(true);
            if sync["enabled"] == true {
                st.settings["sync"]["state"] = json!("idle");
            }
            st.settings_changed(out);
            Ok(json!({ "otherKeys": 0 }))
        })
    }

    fn settings_sync_save_secret(&self, secret: String) -> Result<(), ApiError> {
        if secret.trim().is_empty() || secret.chars().count() > 1024 {
            return Err(invalid("The secret is 1 to 1024 characters."));
        }
        // The fake keeps only that there is one.
        self.apply(|st, out| {
            st.untest_sync();
            st.settings["config"]["storage"]["sync"]["secretSaved"] = json!(true);
            st.settings_changed(out);
            Ok(())
        })
    }

    fn settings_sync_key(&self, action: &str, code: Option<String>) -> Result<Value, ApiError> {
        self.apply(|st, out| {
            let result = match action {
                "show" => {
                    st.settings["sync"]["key"]["exists"] = json!(true);
                    st.settings["sync"]["key"]["id"] = json!("00000000");
                    json!({ "code": FAKE_RECOVERY_CODE, "id": "00000000" })
                }
                "saved" => {
                    st.settings["sync"]["key"]["exported"] = json!(true);
                    Value::Null
                }
                "import" => {
                    let letters: String = code
                        .unwrap_or_default()
                        .chars()
                        .filter(|c| !c.is_whitespace() && *c != '-')
                        .collect();
                    // RKV1 and 55 characters of base32, as a real code has.
                    if !letters.to_uppercase().starts_with("RKV1") || letters.len() != 59 {
                        return Err(invalid("That isn't a Rukoo Voice key."));
                    }
                    st.settings["sync"]["key"] =
                        json!({ "exists": true, "id": "00000000", "exported": true });
                    json!({ "id": "00000000" })
                }
                _ => return Err(invalid("action is show, saved or import.")),
            };
            st.settings_changed(out);
            Ok(result)
        })
    }

    fn settings_sync_key_copy(&self) -> Result<(), ApiError> {
        // The fake never touches the clipboard.
        Ok(())
    }

    fn settings_sync_restore(&self, action: &str) -> Result<Value, ApiError> {
        self.apply(|st, out| {
            let sync = &st.settings["config"]["storage"]["sync"];
            if sync["enabled"] != true || sync["tested"] != true {
                return Err(ApiError::new("off", "Turn sync on first."));
            }
            match action {
                "list" => {
                    let meetings = fake_remote_meetings();
                    let available = meetings.as_array().map_or(0, Vec::len);
                    st.settings["sync"]["restore"] = json!({
                        "state": "available", "available": available, "done": 0, "total": 0, "error": null
                    });
                    st.settings_changed(out);
                    Ok(json!({ "meetings": meetings }))
                }
                "start" => {
                    st.settings["sync"]["restore"] = json!({
                        "state": "done", "available": 0, "done": 2, "total": 2, "error": null
                    });
                    st.settings_changed(out);
                    Ok(Value::Null)
                }
                _ => Err(invalid("action is list or start.")),
            }
        })
    }

    fn settings_start_page(&self) -> Option<String> {
        self.lock().start_page.take()
    }

    fn folder_path(&self, folder: Folder) -> Option<PathBuf> {
        Some(match folder {
            Folder::Logs => voice_core::paths::logs_dir(&self.data_dir),
            // As the real engine: the folder chosen in Settings, else Voice's own models/.
            Folder::Models => {
                let chosen = self.lock().settings["config"]["local"]["modelFolder"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if chosen.is_empty() {
                    voice_core::paths::models_dir(&self.data_dir)
                } else {
                    PathBuf::from(chosen)
                }
            }
            Folder::Recordings => {
                PathBuf::from(self.lock().settings["storage"]["folder"].as_str()?)
            }
            Folder::Bridge => voice_protocol::paths::bridge_path_here(voice_protocol::Side::Voice)?
                .parent()?
                .to_path_buf(),
        })
    }

    fn start_meeting(&self) -> Result<String, ApiError> {
        let timing = self.timing;
        self.apply(|st, out| {
            if let Some(m) = st.meeting.as_ref() {
                if matches!(m.run, Run::Recording | Run::Paused) {
                    return Err(ApiError::new(
                        "busy",
                        format!("Meeting {} is already recording.", m.id),
                    ));
                }
            }
            st.meetings_started += 1;
            let now = Instant::now();
            let id = format!("fake-{}", st.meetings_started);
            st.meeting = Some(Meeting {
                id: id.clone(),
                run: Run::Recording,
                started: now,
                confirm_until: None,
                compact_at: Some(now + timing.compact),
                compact: false,
                resend_at: now + timing.resend,
                monitor: 0,
                screen: st.settings["config"]["meetings"]["screen"]
                    .as_str()
                    .unwrap_or("off")
                    .into(),
                notice: None,
            });
            st.meeting_pill(out);
            out.push(Out::Tray(st.tray()));
            // Settings > Speaker labels lists the meeting.
            st.settings_changed(out);
            Ok(id)
        })
    }

    fn stop_meeting(&self, _reason: StopReason) -> Result<(), ApiError> {
        let timing = self.timing;
        self.apply(|st, out| {
            let Some(m) = st.meeting.as_mut() else {
                return Err(ApiError::new("not_recording", "No meeting is recording."));
            };
            if !matches!(m.run, Run::Recording | Run::Paused) {
                return Err(ApiError::new("not_recording", "No meeting is recording."));
            }
            m.run = Run::Saving {
                until: Instant::now() + timing.saving,
            };
            m.confirm_until = None;
            m.compact = false;
            st.meeting_pill(out);
            out.push(Out::Tray(st.tray()));
            Ok(())
        })
    }

    fn pause_meeting(&self) -> Result<(), ApiError> {
        self.apply(|st, out| {
            match st.meeting.as_mut() {
                Some(m) if m.run == Run::Recording => {
                    m.run = Run::Paused;
                    m.compact = false;
                    m.compact_at = None;
                }
                _ => return Err(ApiError::new("not_recording", "No meeting is recording.")),
            }
            st.meeting_pill(out);
            out.push(Out::Tray(st.tray()));
            Ok(())
        })
    }

    fn resume_meeting(&self) -> Result<(), ApiError> {
        let timing = self.timing;
        self.apply(|st, out| {
            match st.meeting.as_mut() {
                Some(m) if m.run == Run::Paused => {
                    m.run = Run::Recording;
                    m.compact_at = Some(Instant::now() + timing.compact);
                }
                _ => return Err(ApiError::new("not_recording", "No meeting is paused.")),
            }
            st.meeting_pill(out);
            out.push(Out::Tray(st.tray()));
            Ok(())
        })
    }

    fn ui(&self) -> UiOptions {
        self.lock().ui()
    }

    fn system_looks_changed(&self) {
        self.apply(|st, out| {
            st.restyle(out);
            st.settings_changed(out);
        });
    }

    fn tray_state(&self) -> TrayState {
        self.lock().tray()
    }

    fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
pub mod tests {
    use voice_engine::Notification;

    use super::*;

    /// Records what the engine sends.
    #[derive(Default)]
    pub struct Recorder {
        pub pills: Mutex<Vec<PillSnapshot>>,
        pub settings: Mutex<Vec<Value>>,
        pub trays: Mutex<Vec<TrayState>>,
        pub levels: Mutex<u32>,
        pub ui_changes: Mutex<u32>,
    }

    impl UiSink for Recorder {
        fn pill(&self, snapshot: PillSnapshot) {
            self.pills.lock().unwrap().push(snapshot);
        }
        fn level(&self, _level: f32) {
            *self.levels.lock().unwrap() += 1;
        }
        fn settings(&self, state: Value) {
            self.settings.lock().unwrap().push(state);
        }
        fn tray(&self, tray: TrayState) {
            self.trays.lock().unwrap().push(tray);
        }
        fn notify(&self, _n: Notification) {}
        fn ui_changed(&self) {
            *self.ui_changes.lock().unwrap() += 1;
        }
    }

    impl Recorder {
        pub fn last_pill(&self) -> PillSnapshot {
            self.pills
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("a snapshot")
        }
    }

    pub fn fast() -> Timing {
        Timing {
            confirm: Duration::from_millis(300),
            compact: Duration::from_millis(150),
            saving: Duration::from_millis(150),
            resend: Duration::from_secs(60),
            notice: Duration::from_millis(150),
            dictation_every: Duration::from_secs(8),
            cycle_step: Duration::from_millis(50),
        }
    }

    pub fn engine(scenario: &str) -> (Arc<FakeEngine>, Arc<Recorder>) {
        let recorder = Arc::new(Recorder::default());
        let options = EngineOptions {
            data_dir: std::env::temp_dir().join("voice-app-fake-test"),
            dev_mode: true,
            exe_dir: PathBuf::from("."),
            test_sources: None,
            test_screen: None,
            test_monitor: None,
            import_ownkey_from: None,
            version: "0.1.0".into(),
        };
        let engine = FakeEngine::start_with(options, recorder.clone(), scenario, fast()).unwrap();
        (engine, recorder)
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ok() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn unknown_scenarios_are_refused() {
        let recorder = Arc::new(Recorder::default());
        let options = EngineOptions {
            data_dir: std::env::temp_dir(),
            dev_mode: true,
            exe_dir: PathBuf::from("."),
            test_sources: None,
            test_screen: None,
            test_monitor: None,
            import_ownkey_from: None,
            version: "0.1.0".into(),
        };
        assert!(FakeEngine::start(options, recorder, "party").is_err());
    }

    #[test]
    fn the_stop_button_asks_first_and_stops_on_the_second_click() {
        let (engine, recorder) = engine("meeting");
        let first = recorder.last_pill();
        assert!(first.visible && first.phase == PillPhase::Meeting);
        assert!(first.meeting.as_ref().unwrap().stop_enabled);
        assert_eq!(engine.tray_state().meeting.as_deref(), Some("recording"));

        engine.pill_stop_clicked();
        assert!(recorder.last_pill().meeting.unwrap().confirm_stop);
        engine.pill_stop_clicked();
        let saving = recorder.last_pill().meeting.unwrap();
        assert_eq!(saving.state, "processing");
        assert!(!saving.stop_enabled);
        wait_for("the pill to hide after saving", || {
            !recorder.last_pill().visible
        });
        assert_eq!(engine.tray_state(), TrayState::default());
        assert_eq!(engine.stop_clicks(), 2);
        engine.shutdown();
    }

    #[test]
    fn an_unconfirmed_stop_expires() {
        let (engine, recorder) = engine("meeting");
        engine.pill_stop_clicked();
        assert!(recorder.last_pill().meeting.unwrap().confirm_stop);
        wait_for("Stop? to expire", || {
            !recorder.last_pill().meeting.unwrap().confirm_stop
        });
        // The next click asks again instead of stopping.
        engine.pill_stop_clicked();
        assert_eq!(recorder.last_pill().meeting.unwrap().state, "recording");
        engine.shutdown();
    }

    #[test]
    fn the_meeting_pill_compacts_and_pause_shows_it_in_full() {
        let (engine, recorder) = engine("meeting");
        wait_for("compact", || recorder.last_pill().meeting.unwrap().compact);
        engine.pause_meeting().unwrap();
        let paused = recorder.last_pill().meeting.unwrap();
        assert_eq!(paused.state, "paused");
        assert!(!paused.compact);
        assert_eq!(engine.pause_meeting().unwrap_err().code, "not_recording");
        engine.resume_meeting().unwrap();
        assert_eq!(recorder.last_pill().meeting.unwrap().state, "recording");
        assert_eq!(engine.start_meeting().unwrap_err().code, "busy");
        engine.shutdown();
    }

    #[test]
    fn the_screen_menu_changes_the_current_meeting_without_changing_defaults() {
        let (engine, recorder) = engine("meeting");
        let first = recorder.last_pill().meeting.unwrap();
        assert_eq!(first.screen, "frames");
        assert!(first.screen_switch);
        let json = serde_json::to_value(recorder.last_pill()).unwrap();
        assert_eq!(json["meeting"]["screenSwitch"], true);
        let state = engine.settings_state();
        assert_eq!(state["features"]["screen"], true);
        assert_eq!(state["screen"]["monitors"], 2);
        assert_eq!(state["config"]["meetings"]["screen"], "frames");

        engine.pill_screen_clicked();
        let id = engine.lock().meeting.as_ref().unwrap().id.clone();
        engine.pill_screen_set(&id, "off", None).unwrap();
        assert_eq!(recorder.last_pill().meeting.unwrap().screen, "off");
        engine
            .pill_screen_set(&id, "video", Some("Display 2"))
            .unwrap();
        assert_eq!(engine.lock().meeting.as_ref().unwrap().monitor, 1);
        assert_eq!(recorder.last_pill().meeting.unwrap().screen, "video");
        assert_eq!(
            engine.settings_state()["config"]["meetings"]["screen"],
            "frames"
        );
        assert_eq!(engine.screen_clicks(), 1);

        // An audio-only meeting still offers the menu.
        engine.stop_meeting(StopReason::User).unwrap();
        wait_for("the pill to hide", || !recorder.last_pill().visible);
        engine
            .settings_set("meetings.screen", json!("off"))
            .unwrap();
        engine.start_meeting().unwrap();
        let off = recorder.last_pill().meeting.unwrap();
        assert_eq!((off.screen.as_str(), off.screen_switch), ("off", true));
        engine.shutdown();
    }

    #[test]
    fn seq_grows_with_every_snapshot() {
        let (engine, recorder) = engine("cycle");
        wait_for("a few states", || recorder.pills.lock().unwrap().len() >= 6);
        let pills = recorder.pills.lock().unwrap().clone();
        assert!(pills.windows(2).all(|w| w[1].seq > w[0].seq));
        assert!(pills.iter().all(|p| p.dev));
        engine.shutdown();
    }

    #[test]
    fn nemotron_downloads_and_speaker_labels_follow() {
        let (engine, _recorder) = engine("idle");
        let state = engine.settings_state();
        assert_eq!(state["features"]["speakers"], true);
        assert_eq!(state["speakers"]["reason"], "model");
        assert_eq!(state["speakers"]["live"], json!([]));
        engine.settings_model("nemotron", "download").unwrap();
        wait_for("Nemotron to install", || {
            engine.settings_state()["models"]["nemotron"]["stage"] == "installed"
        });
        let state = engine.settings_state();
        assert_eq!(state["models"]["nemotron"]["removable"], true);
        assert_eq!(
            state["models"]["orukeet"]["id"], "orukeet",
            "Orukeet is untouched"
        );
        assert_eq!(state["speakers"]["ready"], true);
        assert_eq!(state["speakers"]["runtime"]["commit"], "97a15af");
        engine
            .settings_set("meetings.liveSpeakers", json!(false))
            .unwrap();
        assert_eq!(engine.settings_state()["speakers"]["reason"], "off");
        engine
            .settings_set("meetings.liveSpeakers", json!(true))
            .unwrap();
        // A meeting labels with the model, so it stays until the meeting is saved.
        let id = engine.start_meeting().unwrap();
        let live = engine.settings_state()["speakers"]["live"].clone();
        assert_eq!(live[0]["meeting"], id.as_str());
        assert_eq!(live[0]["state"], "listening");
        assert_eq!(
            engine
                .settings_model("nemotron", "remove")
                .unwrap_err()
                .code,
            "busy"
        );
        engine.stop_meeting(StopReason::User).unwrap();
        wait_for("the meeting to be saved", || {
            engine.settings_state()["speakers"]["live"] == json!([])
        });
        engine.settings_model("nemotron", "remove").unwrap();
        assert_eq!(engine.settings_state()["speakers"]["reason"], "model");
        engine.shutdown();
    }

    #[test]
    fn the_meeting_scenario_labels_its_meeting() {
        let (engine, _recorder) = engine("meeting");
        let state = engine.settings_state();
        assert_eq!(state["models"]["nemotron"]["location"], "ownkey");
        assert_eq!(state["speakers"]["ready"], true);
        assert_eq!(state["speakers"]["live"][0]["mode"], "nemotron");
        assert_eq!(state["speakers"]["live"][0]["speakers"], 2);
        engine.shutdown();
    }

    #[test]
    fn realtime_models_stream_meetings_only() {
        let (engine, _recorder) = engine("idle");
        let realtime = "voxtral-mini-transcribe-realtime-3";
        let form = |target: &str, consent: bool| {
            json!({
                "target": target, "provider": "mistral",
                "endpoint": "https://api.mistral.ai/v1/audio/transcriptions",
                "model": realtime, "apiKey": "sk-test", "consent": consent
            })
        };
        assert_eq!(
            engine
                .settings_save_provider(form("transcription", true))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        engine
            .settings_save_provider(form("meetings", false))
            .unwrap();
        let state = engine.settings_state();
        assert_eq!(state["speakers"]["engine"], "mistral");
        assert_eq!(state["speakers"]["reason"], "consent");
        engine
            .settings_save_provider(form("meetings", true))
            .unwrap();
        let state = engine.settings_state();
        assert_eq!(state["config"]["meetings"]["consent"]["mode"], "stream");
        assert_eq!(state["speakers"]["ready"], true);
        engine.shutdown();
    }

    #[test]
    fn hosts_come_from_endpoints() {
        assert_eq!(
            host_of("https://api.mistral.ai/v1/audio/transcriptions"),
            "api.mistral.ai"
        );
        assert_eq!(host_of("http://localhost:1234/v1"), "localhost");
        assert_eq!(host_of("http://[::1]:8080/x"), "::1");
        assert_eq!(host_of("HTTPS://User@Example.COM"), "example.com");
        assert_eq!(host_of(""), "");
    }

    #[test]
    fn bucket_sync_follows_api_md_with_scripted_answers() {
        let (engine, _) = engine("idle");
        let sync = || engine.settings_state()["config"]["storage"]["sync"].clone();
        assert_eq!(engine.settings_state()["features"]["sync"], true);
        assert_eq!(engine.settings_state()["sync"]["state"], "off");
        // The secret never goes through settings_set, and the switch waits for a test.
        assert_eq!(
            engine
                .settings_set("storage.sync.secret", json!("s"))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            engine
                .settings_set("storage.sync.enabled", json!(true))
                .unwrap_err()
                .code,
            "needs_test"
        );
        assert_eq!(engine.settings_sync_test().unwrap_err().code, "no_secret");
        assert_eq!(
            engine
                .settings_sync_save_secret(" ".into())
                .unwrap_err()
                .code,
            "invalid_value"
        );
        engine.settings_sync_save_secret("secret-1".into()).unwrap();
        assert_eq!(sync()["secretSaved"], true);
        engine
            .settings_set("storage.sync.bucket", json!("fail"))
            .unwrap();
        assert_eq!(engine.settings_sync_test().unwrap_err().code, "bucket");
        engine
            .settings_set("storage.sync.bucket", json!("meetings"))
            .unwrap();
        assert_eq!(
            engine.settings_sync_test().unwrap(),
            json!({ "otherKeys": 0 })
        );
        engine
            .settings_set("storage.sync.enabled", json!(true))
            .unwrap();
        assert_eq!(engine.settings_state()["sync"]["state"], "idle");
        engine
            .settings_set("storage.sync.retention", json!("days90"))
            .unwrap();
        assert_eq!(
            sync()["enabled"],
            true,
            "retention isn't a connection field"
        );
        assert_eq!(
            engine
                .settings_set("storage.sync.retention", json!("forever"))
                .unwrap_err()
                .code,
            "invalid_value"
        );

        // The key: zeros, never a real one.
        let shown = engine.settings_sync_key("show", None).unwrap();
        assert_eq!(shown["code"], FAKE_RECOVERY_CODE);
        engine.settings_sync_key("saved", None).unwrap();
        assert_eq!(engine.settings_state()["sync"]["key"]["exported"], true);
        assert_eq!(
            engine
                .settings_sync_key("import", Some("hello".into()))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        engine
            .settings_sync_key("import", Some(FAKE_RECOVERY_CODE.to_lowercase()))
            .unwrap();
        engine.settings_sync_key_copy().unwrap();

        let list = engine.settings_sync_restore("list").unwrap();
        let titles: Vec<&str> = list["meetings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["Design review", "Weekly"]);
        engine.settings_sync_restore("start").unwrap();
        assert_eq!(engine.settings_state()["sync"]["restore"]["state"], "done");

        // Changing the bucket or removing the secret pauses sync until a new test; the switch
        // stays on.
        engine
            .settings_set("storage.sync.bucket", json!("elsewhere"))
            .unwrap();
        assert_eq!(
            (sync()["enabled"].clone(), sync()["tested"].clone()),
            (json!(true), json!(false))
        );
        assert_eq!(engine.settings_state()["sync"]["state"], "paused");
        engine.settings_remove_key("sync").unwrap();
        assert_eq!(sync()["secretSaved"], false);
        assert_eq!(
            engine.settings_sync_restore("list").unwrap_err().code,
            "off"
        );
        engine.shutdown();
    }
}
