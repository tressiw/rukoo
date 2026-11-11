//! Fakes for the engine tests: no real keyboard, clipboard, microphone, speakers, registry or
//! network is ever touched.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use voice_core::local::{
    LocalState, LocalStatus, Located, ModelError, ModelErrorKind, ModelRegistry, ModelSpec,
    Progress, Stage,
};
use voice_core::providers::{Activity, ProviderConfig};
use voice_core::{Hints, ProviderError, Recognized, RewriteRequest, Rewriter, SpeechToText};
use voice_engine::media::{RecordedScreen, ScreenMedia};
use voice_engine::meetings::controller::ControllerTiming;
use voice_engine::meetings::screen::ScreenTiming;
use voice_engine::meetings::PipelineOptions;
use voice_engine::permissions::{Permission, PermissionState, Permissions};
use voice_engine::services::{
    AudioSink, Autostart, Chime, KeyState, MicStream, Microphone, Services, Speech, System,
    TextInput, Updates,
};
use voice_engine::{
    Engine, EngineOptions, Locale, Notification, PillPhase, PillSnapshot, Theme, TrayState, UiSink,
};
use voice_input::{BeforeKeys, InputError, InsertMode, Key, TextTarget};
use voice_sync::protect::{Protector, TestProtector};
use voice_sync::{ScreenFiles, SyncTiming};

/// CI machines are slow; every wait is generous.
pub const WAIT: Duration = Duration::from_secs(20);

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// A temp folder, removed when the test passes and kept when it fails.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "voice-engine-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, part: &str) -> PathBuf {
        self.0.join(part)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // A test that failed mid-meeting leaves its recorder running. On Windows that recorder inherited
        // the test's output pipe, so cargo would wait for it, up to the recorder's 8-hour limit.
        stop_recorders(&self.0, 4);
        if thread::panicking() {
            return;
        }
        // Windows lets go of a closed database or a stopped recorder's files a moment later.
        for _ in 0..50 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Stops every recorder still recording into a meeting under `dir`, by the process id in its
/// recorder.json: only recorders this test started, and never by image name.
fn stop_recorders(dir: &Path, depth: usize) {
    if let Some(file) = voice_recorder::meeting::read_recorder_file(dir) {
        if file.pid != std::process::id() {
            kill(file.pid);
        }
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            stop_recorders(&entry.path(), depth - 1);
        }
    }
}

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

// The sink -----------------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeSink {
    pub pills: Mutex<Vec<PillSnapshot>>,
    pub levels: AtomicUsize,
    pub settings: Mutex<Vec<Value>>,
    pub notes: Mutex<Vec<Notification>>,
    pub trays: Mutex<Vec<TrayState>>,
    pub ui_changes: AtomicUsize,
    pub screen_menus: Mutex<Vec<Value>>,
}

impl UiSink for FakeSink {
    fn pill(&self, snapshot: PillSnapshot) {
        lock(&self.pills).push(snapshot);
    }

    fn level(&self, _level: f32) {
        self.levels.fetch_add(1, Ordering::SeqCst);
    }

    fn settings(&self, state: Value) {
        lock(&self.settings).push(state);
    }

    fn tray(&self, tray: TrayState) {
        lock(&self.trays).push(tray);
    }

    fn notify(&self, notification: Notification) {
        lock(&self.notes).push(notification);
    }

    fn ui_changed(&self) {
        self.ui_changes.fetch_add(1, Ordering::SeqCst);
    }

    fn screen_menu(&self, info: Value) {
        lock(&self.screen_menus).push(info);
    }
}

impl FakeSink {
    pub fn last_pill(&self) -> Option<PillSnapshot> {
        lock(&self.pills).last().cloned()
    }

    /// The phases shown, without repeats.
    pub fn phases(&self) -> Vec<PillPhase> {
        let mut phases: Vec<PillPhase> = Vec::new();
        for pill in lock(&self.pills).iter() {
            if phases.last() != Some(&pill.phase) {
                phases.push(pill.phase);
            }
        }
        phases
    }

    pub fn wait_phase(&self, phase: PillPhase) -> PillSnapshot {
        let mut found = None;
        wait_for(&format!("the pill to show {phase:?}"), || {
            found = lock(&self.pills)
                .iter()
                .rev()
                .find(|p| p.phase == phase)
                .cloned();
            found.is_some()
        });
        found.unwrap()
    }

    pub fn wait_hidden_after(&self, phase: PillPhase) {
        wait_for(&format!("the pill to hide after {phase:?}"), || {
            let pills = lock(&self.pills);
            let Some(at) = pills.iter().rposition(|p| p.phase == phase) else {
                return false;
            };
            pills[at..].iter().any(|p| !p.visible)
        });
    }

    pub fn notes(&self) -> Vec<Notification> {
        lock(&self.notes).clone()
    }

    pub fn wait_note(&self, title: &str) -> Notification {
        let mut found = None;
        wait_for(&format!("a notification {title}"), || {
            found = lock(&self.notes)
                .iter()
                .find(|n| n.title_key == title)
                .cloned();
            found.is_some()
        });
        found.unwrap()
    }

    pub fn last_settings(&self) -> Value {
        lock(&self.settings).last().cloned().unwrap_or(Value::Null)
    }
}

// Keys ---------------------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct Keys(Arc<Mutex<HashSet<Key>>>);

impl Keys {
    pub fn down(&self, key: Key) {
        lock(&self.0).insert(key);
    }

    /// Lets go, and waits long enough for the poller to see it: a tap shorter than one poll
    /// is never seen, as on a real keyboard.
    pub fn up(&self, key: Key) {
        lock(&self.0).remove(&key);
        thread::sleep(Duration::from_millis(40));
    }

    pub fn is_down(&self, key: Key) -> bool {
        lock(&self.0).contains(&key)
    }

    pub fn state(&self) -> KeyState {
        let keys = self.clone();
        Arc::new(move |key| keys.is_down(key))
    }
}

// Input --------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Inserted {
    pub text: String,
    pub mode: InsertMode,
    /// Hotkeys held at that moment.
    pub held: Vec<Key>,
}

pub struct FakeInput {
    pub keys: Keys,
    pub inserted: Mutex<Vec<Inserted>>,
    pub selection: Mutex<Option<String>>,
    /// Ctrl+C sent, with the hotkeys held at that moment.
    pub captures: AtomicUsize,
    pub capture_held: Mutex<Vec<Vec<Key>>>,
    pub target: Mutex<TextTarget>,
    pub fail: Mutex<Option<InputError>>,
    /// How long an insertion or a capture waits for the clipboard before its keys, as behind the
    /// previous paste's restore.
    pub clipboard_wait: Mutex<Duration>,
    /// Insertions and captures that started.
    pub entered: AtomicUsize,
    /// What `copy_hidden` put on the fake clipboard, and whether `clear_if` emptied it.
    pub clipboard: Mutex<Option<String>>,
    pub hidden_copies: Mutex<Vec<String>>,
}

impl FakeInput {
    pub fn new(keys: Keys) -> FakeInput {
        FakeInput {
            keys,
            inserted: Mutex::new(Vec::new()),
            selection: Mutex::new(None),
            captures: AtomicUsize::new(0),
            capture_held: Mutex::new(Vec::new()),
            target: Mutex::new(TextTarget::Selected),
            fail: Mutex::new(None),
            clipboard_wait: Mutex::new(Duration::ZERO),
            entered: AtomicUsize::new(0),
            clipboard: Mutex::new(None),
            hidden_copies: Mutex::new(Vec::new()),
        }
    }

    fn held(&self) -> Vec<Key> {
        Key::ALL
            .into_iter()
            .filter(|k| self.keys.is_down(*k))
            .collect()
    }

    /// Waits for the clipboard, then runs the caller's check right before the keys go out.
    fn before_keys(&self, before_keys: BeforeKeys) -> Result<(), InputError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let wait = *lock(&self.clipboard_wait);
        thread::sleep(wait);
        before_keys()
    }

    pub fn texts(&self) -> Vec<String> {
        lock(&self.inserted)
            .iter()
            .map(|i| i.text.clone())
            .collect()
    }
}

impl TextInput for FakeInput {
    fn insert(
        &self,
        text: &str,
        mode: InsertMode,
        before_keys: BeforeKeys,
    ) -> Result<(), InputError> {
        self.before_keys(before_keys)?;
        if let Some(error) = lock(&self.fail).clone() {
            return Err(error);
        }
        lock(&self.inserted).push(Inserted {
            text: text.to_string(),
            mode,
            held: self.held(),
        });
        Ok(())
    }

    fn capture_selection(
        &self,
        _timeout: Duration,
        before_keys: BeforeKeys,
    ) -> Result<Option<String>, InputError> {
        self.before_keys(before_keys)?;
        lock(&self.capture_held).push(self.held());
        self.captures.fetch_add(1, Ordering::SeqCst);
        Ok(lock(&self.selection).clone())
    }

    fn text_target(&self) -> TextTarget {
        *lock(&self.target)
    }

    fn copy_hidden(&self, text: &str) -> Result<(), InputError> {
        lock(&self.hidden_copies).push(text.to_string());
        *lock(&self.clipboard) = Some(text.to_string());
        Ok(())
    }

    fn clear_if(&self, text: &str) -> Result<bool, InputError> {
        let mut clipboard = lock(&self.clipboard);
        let holds = clipboard.as_deref() == Some(text);
        if holds {
            *clipboard = None;
        }
        Ok(holds)
    }
}

// Microphone ---------------------------------------------------------------------------------

/// What the next open of the fake microphone does.
#[derive(Clone, Debug)]
pub enum MicPlan {
    /// Delivers these blocks 2 ms apart, then nothing.
    Blocks(Vec<Vec<i16>>),
    Fail(String),
}

/// `blocks` of 10 ms of a tone at `amplitude` (0 is silence).
pub fn tone(blocks: usize, amplitude: f32) -> MicPlan {
    MicPlan::Blocks(
        (0..blocks)
            .map(|b| {
                (0..160)
                    .map(|i| (((b * 160 + i) as f32 * 0.17).sin() * amplitude) as i16)
                    .collect()
            })
            .collect(),
    )
}

#[derive(Default)]
pub struct FakeMic {
    pub plans: Mutex<VecDeque<MicPlan>>,
    pub opens: AtomicUsize,
    pub delivered: Arc<AtomicUsize>,
}

struct FakeStream {
    closed: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MicStream for FakeStream {}

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Microphone for FakeMic {
    fn open(&self, mut on_audio: AudioSink) -> Result<Box<dyn MicStream>, String> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        let plan = lock(&self.plans)
            .pop_front()
            .unwrap_or_else(|| tone(40, 8000.0));
        let blocks = match plan {
            MicPlan::Fail(message) => return Err(message),
            MicPlan::Blocks(blocks) => blocks,
        };
        let closed = Arc::new(AtomicBool::new(false));
        let delivered = Arc::clone(&self.delivered);
        let thread = thread::spawn({
            let closed = Arc::clone(&closed);
            move || {
                for block in blocks {
                    if closed.load(Ordering::SeqCst) {
                        return;
                    }
                    on_audio(&block);
                    delivered.fetch_add(block.len(), Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(2));
                }
            }
        });
        Ok(Box::new(FakeStream {
            closed,
            thread: Some(thread),
        }))
    }
}

// Speech -------------------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeSpeech {
    pub transcripts: Mutex<VecDeque<Result<String, ProviderError>>>,
    pub rewrites: Mutex<VecDeque<Result<String, ProviderError>>>,
    /// Samples per transcription, in order.
    pub heard: Mutex<Vec<usize>>,
    pub rewrite_requests: Mutex<Vec<RewriteRequest>>,
    pub transcribe_delay: Mutex<Duration>,
    pub local_state: Mutex<LocalState>,
    pub local_load: Mutex<Option<ProviderError>>,
    pub local_loads: AtomicUsize,
    /// How long loading Orukeet takes, as for a first load that checks the model.
    pub local_load_delay: Mutex<Duration>,
    /// How long `shutdown` blocks, as behind a meeting window that holds the worker.
    pub shutdown_delay: Mutex<Duration>,
    pub shutdowns: AtomicUsize,
    pub models: Mutex<Vec<String>>,
    pub model_requests: Mutex<Vec<ProviderConfig>>,
    /// Samples per meeting window, in order. Meetings have a recognizer of their own, so a
    /// meeting window never takes a transcript queued for a dictation.
    pub meeting_heard: Mutex<Vec<usize>>,
    /// The provider of each meeting recognizer the engine built.
    pub meeting_providers: Mutex<Vec<String>>,
}

impl FakeSpeech {
    pub fn say(&self, text: &str) {
        lock(&self.transcripts).push_back(Ok(text.to_string()));
    }

    pub fn heard(&self) -> Vec<usize> {
        lock(&self.heard).clone()
    }
}

pub struct FakeStt(Arc<FakeSpeech>);

impl SpeechToText for FakeStt {
    fn transcribe(&self, pcm: &[i16], _hints: &Hints) -> Result<Recognized, ProviderError> {
        let delay = *lock(&self.0.transcribe_delay);
        thread::sleep(delay);
        lock(&self.0.heard).push(pcm.len());
        let next = lock(&self.0.transcripts)
            .pop_front()
            .unwrap_or_else(|| Ok("hello world".into()));
        next.map(|text| Recognized {
            text,
            language: None,
        })
    }
}

/// Meeting windows: "Meeting words." for each.
pub struct FakeMeetingStt(Arc<FakeSpeech>);

impl SpeechToText for FakeMeetingStt {
    fn transcribe(&self, pcm: &[i16], _hints: &Hints) -> Result<Recognized, ProviderError> {
        lock(&self.0.meeting_heard).push(pcm.len());
        Ok(Recognized {
            text: "Meeting words.".into(),
            language: None,
        })
    }
}

pub struct FakeRewriter(Arc<FakeSpeech>);

impl Rewriter for FakeRewriter {
    fn rewrite(&self, req: &RewriteRequest) -> Result<String, ProviderError> {
        lock(&self.0.rewrite_requests).push(req.clone());
        lock(&self.0.rewrites)
            .pop_front()
            .unwrap_or_else(|| Ok("Rewritten.".into()))
    }
}

/// The speech services around one shared fake.
pub struct SpeechHandle(pub Arc<FakeSpeech>);

impl Speech for SpeechHandle {
    fn recognizer(&self, cfg: &ProviderConfig) -> Result<Arc<dyn SpeechToText>, ProviderError> {
        if cfg.provider != "orukeet" && cfg.model.trim().is_empty() {
            return Err(ProviderError::config("Select an audio model in Settings."));
        }
        Ok(Arc::new(FakeStt(Arc::clone(&self.0))))
    }

    fn meeting_recognizer(&self, cfg: &ProviderConfig) -> Result<PipelineOptions, ProviderError> {
        lock(&self.0.meeting_providers).push(cfg.provider.clone());
        if cfg.provider != "orukeet" && cfg.api_key.trim().is_empty() {
            return Err(ProviderError::config("Add an API key in Settings."));
        }
        Ok(PipelineOptions::new(Arc::new(FakeMeetingStt(Arc::clone(
            &self.0,
        )))))
    }

    fn rewriter(&self, cfg: &ProviderConfig) -> Result<Arc<dyn Rewriter>, ProviderError> {
        if cfg.model.trim().is_empty() {
            return Err(ProviderError::config("Select a rewrite model in Settings."));
        }
        Ok(Arc::new(FakeRewriter(Arc::clone(&self.0))))
    }

    fn list_models(
        &self,
        _activity: Activity,
        cfg: &ProviderConfig,
    ) -> Result<Vec<String>, ProviderError> {
        lock(&self.0.model_requests).push(cfg.clone());
        Ok(lock(&self.0.models).clone())
    }

    fn local_preload(&self) {}

    fn local_status(&self) -> LocalStatus {
        LocalStatus {
            state: *lock(&self.0.local_state),
            ..LocalStatus::default()
        }
    }

    fn local_load(&self, _registry: ModelRegistry) -> Result<(), ProviderError> {
        self.0.local_loads.fetch_add(1, Ordering::SeqCst);
        let delay = *lock(&self.0.local_load_delay);
        thread::sleep(delay);
        match lock(&self.0.local_load).clone() {
            Some(error) => Err(error),
            None => {
                *lock(&self.0.local_state) = LocalState::Loaded;
                Ok(())
            }
        }
    }

    fn local_configure(&self, _registry: ModelRegistry, _idle: Duration) {}

    fn local_unload(&self) {
        *lock(&self.0.local_state) = LocalState::Unloaded;
    }

    fn download(
        &self,
        _registry: &ModelRegistry,
        _spec: &ModelSpec,
        progress: &mut dyn FnMut(Progress),
        _cancel: &AtomicBool,
    ) -> Result<Located, ModelError> {
        progress(Progress {
            stage: Stage::Downloading,
            done: 50,
            total: Some(100),
        });
        progress(Progress {
            stage: Stage::Verifying,
            done: 0,
            total: None,
        });
        Err(ModelError::new(
            ModelErrorKind::Damaged,
            "Checksum mismatch for encoder.int8.onnx.",
        ))
    }

    fn shutdown(&self) {
        self.0.shutdowns.fetch_add(1, Ordering::SeqCst);
        let delay = *lock(&self.0.shutdown_delay);
        thread::sleep(delay);
    }
}

// The rest -----------------------------------------------------------------------------------

#[derive(Default)]
pub struct FakeChime(pub AtomicUsize);

pub struct ChimeHandle(pub Arc<FakeChime>);

impl Chime for ChimeHandle {
    fn play(&self) {
        self.0 .0.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub struct FakeAutostart {
    pub on: AtomicBool,
    pub writes: AtomicUsize,
}

pub struct AutostartHandle(pub Arc<FakeAutostart>);

impl Autostart for AutostartHandle {
    fn is_on(&self) -> bool {
        self.0.on.load(Ordering::SeqCst)
    }

    fn set(&self, on: bool) -> Result<(), String> {
        self.0.writes.fetch_add(1, Ordering::SeqCst);
        self.0.on.store(on, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeUpdates(pub Mutex<VecDeque<Result<Option<String>, String>>>);

pub struct UpdatesHandle(pub Arc<FakeUpdates>);

impl Updates for UpdatesHandle {
    fn check(&self, _current: &str) -> Result<Option<String>, String> {
        lock(&self.0 .0).pop_front().unwrap_or(Ok(None))
    }
}

/// Free space on the test's disk, as the engine reads it.
pub type FreeSpace = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

/// Plenty of room: more than every free-space floor (bucket sync holds media above 10 GiB).
pub const ROOMY: u64 = 500 << 30;

pub struct FakeSystem(pub Arc<Mutex<FreeSpace>>);

impl System for FakeSystem {
    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn locale(&self) -> Locale {
        Locale::En
    }

    fn ownkey_running(&self) -> bool {
        false
    }

    fn free_bytes(&self, path: &Path) -> Option<u64> {
        let free = Arc::clone(&lock(&self.0));
        free(path)
    }
}

/// The macOS permissions Settings shows, as a platform without them (every state `None`) or with
/// the states a test set. Missing entries are `unknown`, as a preflight this version can't read.
pub struct FakePermissions {
    pub supported: bool,
    pub states: Mutex<HashMap<Permission, PermissionState>>,
}

impl FakePermissions {
    pub fn set(&self, permission: Permission, state: PermissionState) {
        lock(&self.states).insert(permission, state);
    }
}

impl Permissions for FakePermissions {
    fn state(&self, permission: Permission) -> Option<PermissionState> {
        if !self.supported {
            return None;
        }
        Some(
            lock(&self.states)
                .get(&permission)
                .copied()
                .unwrap_or(PermissionState::Unknown),
        )
    }
}

/// An engine on fakes in a temp data folder.
pub struct Harness {
    pub dir: TempDir,
    pub keys: Keys,
    pub sink: Arc<FakeSink>,
    pub input: Arc<FakeInput>,
    pub mic: Arc<FakeMic>,
    pub speech: Arc<FakeSpeech>,
    pub chime: Arc<FakeChime>,
    pub autostart: Arc<FakeAutostart>,
    pub updates: Arc<FakeUpdates>,
    pub permissions: Arc<FakePermissions>,
    pub ownkey_config: Option<PathBuf>,
    pub ownkey_models: Option<PathBuf>,
    pub recorder_exe: PathBuf,
    pub timing: ControllerTiming,
    /// `rukoo-voice-screen`; a path that doesn't exist (the default) records no screen.
    pub screen_exe: PathBuf,
    pub screen_timing: ScreenTiming,
    pub dev_mode: bool,
    pub import_from: Option<PathBuf>,
    pub test_sources: Option<String>,
    /// Free space as the engine sees it; [`ROOMY`] unless a test changes it.
    pub free: Arc<Mutex<FreeSpace>>,
    pub screen_files: Arc<dyn ScreenFiles>,
    pub screen_media: Arc<dyn ScreenMedia>,
    pub sync_timing: SyncTiming,
    /// The stand-in protector works on every platform; a test of DPAPI itself sets `Dpapi`.
    pub protector: Arc<dyn Protector>,
    pub test_screen: Option<String>,
    pub test_monitor: Option<String>,
}

impl Harness {
    pub fn new(name: &str) -> Harness {
        let keys = Keys::default();
        let dir = TempDir::new(name);
        let roomy: FreeSpace = Arc::new(|_| Some(ROOMY));
        // The screen folder as the app reads it; a meeting without a screen recording has none.
        let screen = Arc::new(RecordedScreen::default());
        Harness {
            free: Arc::new(Mutex::new(roomy)),
            screen_files: Arc::clone(&screen) as Arc<dyn ScreenFiles>,
            screen_media: screen,
            sync_timing: SyncTiming::default(),
            protector: Arc::new(TestProtector),
            recorder_exe: dir.join("no-recorder.exe"),
            screen_exe: dir.join("no-screen.exe"),
            screen_timing: ScreenTiming::default(),
            dir,
            input: Arc::new(FakeInput::new(keys.clone())),
            keys,
            sink: Arc::new(FakeSink::default()),
            mic: Arc::new(FakeMic::default()),
            speech: Arc::new(FakeSpeech::default()),
            chime: Arc::new(FakeChime::default()),
            autostart: Arc::new(FakeAutostart::default()),
            updates: Arc::new(FakeUpdates::default()),
            permissions: Arc::new(FakePermissions {
                supported: cfg!(target_os = "macos"),
                states: Mutex::new(HashMap::new()),
            }),
            ownkey_config: None,
            ownkey_models: None,
            timing: ControllerTiming::default(),
            dev_mode: true,
            import_from: None,
            test_sources: None,
            test_screen: None,
            test_monitor: None,
        }
    }

    pub fn data(&self) -> PathBuf {
        self.dir.join("data")
    }

    /// Writes `config.json` before the engine starts: Orukeet for dictation plus `patch`.
    pub fn config(&self, patch: Value) {
        let data = self.data();
        std::fs::create_dir_all(&data).unwrap();
        let mut cfg = voice_core::Config::defaults(self.dev_mode);
        cfg.switch_audio_provider("orukeet");
        let mut value = serde_json::to_value(&cfg).unwrap();
        for (key, v) in patch.as_object().unwrap() {
            value[key] = v.clone();
        }
        std::fs::write(
            data.join("config.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    pub fn saved_config(&self) -> voice_core::Config {
        voice_core::Config::load_as(&self.data(), self.dev_mode).unwrap()
    }

    pub fn services(&self) -> Services {
        Services {
            key_state: Some(self.keys.state()),
            hotkey_interval: Duration::from_millis(5),
            input: self.input.clone(),
            microphone: self.mic.clone(),
            speech: Arc::new(SpeechHandle(Arc::clone(&self.speech))),
            chime: Arc::new(ChimeHandle(Arc::clone(&self.chime))),
            autostart: Arc::new(AutostartHandle(Arc::clone(&self.autostart))),
            updates: Arc::new(UpdatesHandle(Arc::clone(&self.updates))),
            system: Arc::new(FakeSystem(Arc::clone(&self.free))),
            permissions: Arc::clone(&self.permissions) as Arc<dyn Permissions>,
            ownkey_config: self.ownkey_config.clone(),
            ownkey_models: self.ownkey_models.clone(),
            recorder_exe: self.recorder_exe.clone(),
            screen_exe: self.screen_exe.clone(),
            discovery_path: Some(self.dir.join("bridge.json")),
            controller: self.timing.clone(),
            screen_timing: self.screen_timing.clone(),
            screen_files: Arc::clone(&self.screen_files),
            screen_media: Arc::clone(&self.screen_media),
            protector: Arc::clone(&self.protector),
            sync_timing: self.sync_timing.clone(),
        }
    }

    /// Sets the free space the engine sees from now on.
    pub fn set_free(&self, free: impl Fn(&Path) -> Option<u64> + Send + Sync + 'static) {
        *lock(&self.free) = Arc::new(free);
    }

    pub fn options(&self) -> EngineOptions {
        EngineOptions {
            data_dir: self.data(),
            dev_mode: self.dev_mode,
            exe_dir: self.dir.path().to_path_buf(),
            test_sources: self.test_sources.clone(),
            test_screen: self.test_screen.clone(),
            test_monitor: self.test_monitor.clone(),
            import_ownkey_from: self.import_from.clone(),
            version: "0.1.0-test".into(),
        }
    }

    pub fn start(&self) -> Engine {
        voice_engine::start_with(self.options(), self.sink.clone(), self.services())
            .expect("the engine starts")
    }

    /// Holds `key` until the microphone delivered some audio, then lets go.
    pub fn hold(&self, key: Key) {
        let before = self.mic.delivered.load(Ordering::SeqCst);
        self.keys.down(key);
        wait_for("audio from the microphone", || {
            self.mic.delivered.load(Ordering::SeqCst) >= before + 3200
        });
        self.keys.up(key);
    }
}

// Bucket sync -------------------------------------------------------------------------------

/// The fake bucket's name, access key and the secret the tests save.
pub const BUCKET: &str = "test";
pub const ACCESS_KEY: &str = "test";
pub const BUCKET_SECRET: &str = "test-secret-4711";

/// A fake S3 on a free port of this computer, objects in memory.
pub fn fake_s3() -> voice_s3_fake::FakeS3 {
    voice_s3_fake::FakeS3::start(BUCKET, ACCESS_KEY, None).expect("the fake S3 starts")
}

/// Fills in the fake bucket in Settings, saves the secret, passes Test connection and turns sync
/// on, as a user would.
pub fn connect_bucket(engine: &Engine, endpoint: &str) {
    for (key, value) in [
        ("storage.sync.service", "other"),
        ("storage.sync.endpoint", endpoint),
        ("storage.sync.region", "us-east-1"),
        ("storage.sync.bucket", BUCKET),
        ("storage.sync.accessKeyId", ACCESS_KEY),
    ] {
        engine
            .settings_set(key, serde_json::json!(value))
            .unwrap_or_else(|e| panic!("{key}: {e}"));
    }
    engine
        .settings_sync_save_secret(BUCKET_SECRET.into())
        .expect("the secret is saved");
    engine.settings_sync_test().expect("Test connection passes");
    engine
        .settings_set("storage.sync.enabled", serde_json::json!(true))
        .expect("sync turns on");
}

/// Settings' `sync` now.
pub fn sync_state(engine: &Engine) -> Value {
    engine.settings_state()["sync"].clone()
}

/// The 32 bytes of the master key in a recovery code: Crockford base32 after `RKV1-`.
pub fn key_bytes(code: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let body = code.strip_prefix("RKV1-").expect("a recovery code");
    let mut bits: u64 = 0;
    let mut count = 0;
    let mut bytes = Vec::new();
    for c in body.bytes().filter(|c| *c != b'-') {
        let value = ALPHABET
            .iter()
            .position(|a| *a == c.to_ascii_uppercase())
            .expect("Crockford base32") as u64;
        bits = (bits << 5) | value;
        count += 5;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    bytes.truncate(32);
    bytes
}

/// Every object of the fake bucket and its plaintext, opened with the key of `code`. Fails when
/// an object doesn't open: then it isn't RKV1 under that key and that name.
pub fn decrypt_all(fake: &voice_s3_fake::FakeS3, code: &str) -> Vec<(String, Vec<u8>)> {
    use voice_sync::format::{Header, Sealer, CHUNK, HEADER_LEN, TAG};
    let key = voice_sync::recovery::decode(code).expect("the code decodes");
    fake.objects()
        .into_iter()
        .map(|(name, bytes)| {
            assert!(bytes.starts_with(b"RKV1"), "{name} isn't RKV1");
            let header = Header::parse(&bytes[..HEADER_LEN as usize]).expect("a header");
            let sealer = Sealer::open(&key, &name, &header)
                .unwrap_or_else(|e| panic!("{name} doesn't open: {e}"));
            let chunks = header.plain_len().div_ceil(CHUNK).max(1);
            let mut plain = Vec::new();
            for i in 0..chunks {
                let from = (HEADER_LEN + i * (CHUNK + TAG)) as usize;
                let to = (from + (CHUNK + TAG) as usize).min(bytes.len());
                plain.extend(
                    sealer
                        .open_chunk(&header, i, &bytes[from..to])
                        .unwrap_or_else(|e| panic!("chunk {i} of {name}: {e}")),
                );
            }
            (name, plain)
        })
        .collect()
}

/// What a decrypted object is: `audio`, `transcript`, `meeting`, `timeline`, `manifest`,
/// `index`, `jpeg`, `mp4` or `other`.
pub fn kind_of(plain: &[u8]) -> &'static str {
    if plain.starts_with(b"RIFF") {
        return "audio";
    }
    if plain.starts_with(b"SQLite format 3\0") {
        return "transcript";
    }
    if plain.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "jpeg";
    }
    if plain.len() > 8 && &plain[4..8] == b"ftyp" {
        return "mp4";
    }
    if let Ok(value) = serde_json::from_slice::<Value>(plain) {
        return if value.get("files").is_some() {
            "manifest"
        } else if value.get("meetings").is_some() {
            "index"
        } else if value.get("tracks").is_some() {
            "meeting"
        } else {
            "other"
        };
    }
    let text = String::from_utf8_lossy(plain);
    let first = text.lines().next().unwrap_or_default();
    match serde_json::from_str::<Value>(first) {
        Ok(line) if line.get("kind").is_some() => "timeline",
        _ => "other",
    }
}

/// The kinds of the bucket's objects, decrypted with `code`, sorted.
pub fn bucket_kinds(fake: &voice_s3_fake::FakeS3, code: &str) -> Vec<&'static str> {
    let mut kinds: Vec<&str> = decrypt_all(fake, code)
        .iter()
        .map(|(_, plain)| kind_of(plain))
        .collect();
    kinds.sort();
    kinds
}

/// Bucket sync's clocks for tests: scans, backoff and debounces in fractions of a second.
pub fn fast_sync() -> SyncTiming {
    SyncTiming {
        scan_live: Duration::from_millis(300),
        scan_all: Duration::from_secs(2),
        backoff_min: Duration::from_millis(100),
        backoff_max: Duration::from_secs(1),
        manifest_live: Duration::from_secs(1),
        index_debounce: Duration::from_millis(500),
        connect_timeout: Duration::from_secs(2),
        request_timeout: Duration::from_secs(5),
        ..SyncTiming::default()
    }
}
