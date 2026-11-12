//! The contract between the engine and the Tauri app.
//!
//! The app is a thin shell: every Tauri command in `voice/ui/API.md` calls one method here, and every
//! event the pages listen to comes out of [`UiSink`]. Anything that can be tested without a window
//! belongs behind this trait. JSON payloads follow `voice/ui/API.md` exactly; the app passes them
//! through without reshaping them.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How the app starts the engine.
#[derive(Clone, Debug)]
pub struct EngineOptions {
    /// Voice's data folder: `RUKOO_VOICE_DATA_DIR`, or the platform default.
    pub data_dir: PathBuf,
    /// `RUKOO_VOICE_DATA_DIR` is set: F13/F14, no automatic Ownkey import, no autostart.
    pub dev_mode: bool,
    /// The folder holding `rukoo-voice-recorder`, `rukoo-voice-screen` and `rukoo-voice-local`.
    pub exe_dir: PathBuf,
    /// Development only: `RUKOO_VOICE_TEST_SOURCES`, e.g. `mic=file:a.wav,call=synthetic:tone:440`.
    pub test_sources: Option<String>,
    /// Development only: `RUKOO_VOICE_TEST_SCREEN`, the screen process's `--test` spec, e.g.
    /// `synthetic:1920x1080,change-every=10000,text=Slide`.
    pub test_screen: Option<String>,
    /// Development only: `RUKOO_VOICE_TEST_MONITOR`, the monitor every screen process records,
    /// e.g. `\\.\DISPLAY2`.
    pub test_monitor: Option<String>,
    /// `--import-ownkey <copy>`: the only way a development copy imports Ownkey's settings.
    pub import_ownkey_from: Option<PathBuf>,
    /// Voice's version, `CARGO_PKG_VERSION` of the app.
    pub version: String,
}

/// Where the engine's output goes. The app turns these into Tauri events and window changes.
pub trait UiSink: Send + Sync + 'static {
    /// `pill://snapshot`. The app also shows, raises or hides the pill window from `visible`.
    fn pill(&self, snapshot: PillSnapshot);
    /// `pill://level`, only while the pill is listening, at most 50 times a second.
    fn level(&self, level: f32);
    /// `settings://state`: the whole `SettingsState` of API.md, sent whenever any part changes.
    fn settings(&self, state: Value);
    /// The tray menu and tooltip changed.
    fn tray(&self, tray: TrayState);
    /// A tray notification for a full error text or a notice the pill only names.
    fn notify(&self, notification: Notification);
    /// Theme, language or pill options changed; the app re-reads [`EngineApi::ui`].
    fn ui_changed(&self);
    /// Show the pill's native screen mode and monitor menu for this live meeting.
    fn screen_menu(&self, _info: Value) {}
    /// The pill didn't ack snapshot `seq` after a show or a phase change: 500 ms after it the
    /// app nudges the window (resize by a pixel and raise it), after 2 s it builds a new window
    /// and replays [`EngineApi::pill_snapshot`] (pill-settings-design.md 3.2, step 5).
    fn pill_repair(&self, seq: u64, repair: PillRepair) {
        let _ = (seq, repair);
    }
}

/// What the app does to a pill window that didn't paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillRepair {
    Nudge,
    Rebuild,
}

/// `pill://snapshot` in API.md.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PillSnapshot {
    pub seq: u64,
    pub visible: bool,
    pub phase: PillPhase,
    pub activity: Activity,
    pub notice: Option<Notice>,
    pub words: Option<u32>,
    pub meeting: Option<MeetingPill>,
    pub edge: Edge,
    pub theme: Theme,
    pub locale: Locale,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PillPhase {
    Hidden,
    Arming,
    Listening,
    Transcribing,
    Rewriting,
    Inserted,
    Rewrote,
    NothingHeard,
    Warning,
    Error,
    Meeting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    Dictate,
    Rewrite,
}

/// A catalog key from `voice/ui/i18n` with its placeholders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub key: String,
    #[serde(default)]
    pub args: serde_json::Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPill {
    /// `recording`, `paused`, `processing`, `done` or `failed` (PROTOCOL.md `meeting.state`).
    pub state: String,
    pub at_ms: u64,
    /// What the meeting records of the screen now: `off`, `frames` or `video`.
    pub screen: String,
    /// The pill offers screen mode and monitor controls for this live meeting.
    pub screen_switch: bool,
    pub compact: bool,
    pub confirm_stop: bool,
    pub stop_enabled: bool,
    pub rukoo_connected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Bottom,
    Top,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Locale {
    En,
    Nl,
}

/// What the app needs to draw its windows and tray, resolved from Settings.
#[derive(Clone, Debug, PartialEq)]
pub struct UiOptions {
    pub theme: Theme,
    pub locale: Locale,
    pub pill_edge: Edge,
    pub hide_pill_from_capture: bool,
    pub dev: bool,
}

/// The tray's menu and tooltip.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TrayState {
    /// `None` without a meeting, otherwise `recording`, `paused` or `processing`.
    pub meeting: Option<String>,
    /// The meeting time when this state was made, for the tooltip.
    pub meeting_at_ms: Option<u64>,
    /// A short status line for the tooltip, already translated.
    pub status: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    /// Catalog key of the title, e.g. `app.notify.dictationFailed`.
    pub title_key: String,
    /// The body, already in the user's language or an English provider message. With a
    /// `body_key` it is the English fallback.
    pub body: String,
    /// Catalog key of a body with English and Dutch text, which the app shows in the user's
    /// language instead of `body`.
    pub body_key: Option<String>,
}

/// Rejection of a command, `{ code, message }` in API.md.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// Folders the app opens in Explorer or lets the user choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folder {
    Logs,
    Bridge,
    Recordings,
    Models,
}

/// Who asked to stop a meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    User,
    Rukoo,
}

/// Everything the app calls. Commands in API.md map one to one; JSON arguments and results are passed
/// through as they are.
pub trait EngineApi: Send + Sync + 'static {
    // The pill (API.md "The pill").
    fn pill_snapshot(&self) -> PillSnapshot;
    fn pill_ack(&self, seq: u64);
    /// The native stop window took a click: the first asks "Stop?", a second within 3 s stops.
    fn pill_stop_clicked(&self);
    /// The native screen window took a click: open the screen menu.
    fn pill_screen_clicked(&self);
    fn pill_screen_set(
        &self,
        meeting: &str,
        mode: &str,
        monitor: Option<&str>,
    ) -> Result<(), ApiError>;

    // Settings (API.md "Settings"). Values are the JSON shapes API.md defines.
    fn settings_state(&self) -> Value;
    fn settings_set(&self, key: &str, value: Value) -> Result<(), ApiError>;
    fn settings_save_provider(&self, save: Value) -> Result<(), ApiError>;
    fn settings_remove_key(&self, target: &str) -> Result<(), ApiError>;
    fn settings_list_models(&self, request: Value) -> Result<Vec<String>, ApiError>;
    fn settings_model(&self, model: &str, action: &str) -> Result<(), ApiError>;
    /// After the app's folder picker: `models` or `recordings`.
    fn settings_set_folder(&self, purpose: &str, path: PathBuf) -> Result<(), ApiError>;
    fn settings_clean_preview(&self, preview: Value) -> Result<String, ApiError>;
    fn settings_check_updates(&self) -> Value;
    fn settings_new_token(&self) -> Result<(), ApiError>;
    fn settings_import_ownkey(&self) -> Result<Value, ApiError>;
    /// Test connection for `config.storage.sync` with the saved secret: `{ otherKeys }`, and the
    /// fields count as tested.
    fn settings_sync_test(&self) -> Result<Value, ApiError>;
    /// Keeps the bucket's secret with DPAPI, never in `config.json`; uploads pause until the next
    /// passed test, and the switch stays on.
    fn settings_sync_save_secret(&self, secret: String) -> Result<(), ApiError>;
    /// The encryption key: `show` (`{ code, id }`), `saved`, or `import` with `code` (`{ id }`).
    fn settings_sync_key(&self, action: &str, code: Option<String>) -> Result<Value, ApiError>;
    /// Copies the recovery code with voice-input's hidden clipboard path and clears it after 60 s
    /// if unchanged. The page never gets the code from this command.
    fn settings_sync_key_copy(&self) -> Result<(), ApiError>;
    /// Meetings in the bucket that aren't here: `list` (`{ meetings }`) or `start` a restore.
    fn settings_sync_restore(&self, action: &str) -> Result<Value, ApiError>;
    /// The page or group Settings should open at after a start, e.g. `import` after the first run.
    fn settings_start_page(&self) -> Option<String>;
    fn folder_path(&self, folder: Folder) -> Option<PathBuf>;

    // Meetings, for the tray and the pill. Rukoo reaches the same controller through the bridge.
    fn start_meeting(&self) -> Result<String, ApiError>;
    fn stop_meeting(&self, reason: StopReason) -> Result<(), ApiError>;
    fn pause_meeting(&self) -> Result<(), ApiError>;
    fn resume_meeting(&self) -> Result<(), ApiError>;

    // The shell.
    fn ui(&self) -> UiOptions;
    fn tray_state(&self) -> TrayState;
    /// Stops dictation, transcription and the bridge, and waits for the clipboard to come back.
    /// Never stops a recorder that is recording: the meeting goes on and the next start reattaches.
    fn shutdown(&self);

    /// For tests: what Rukoo's bridge commands reach, called without a socket.
    #[doc(hidden)]
    fn meeting_control(&self) -> Option<Arc<dyn voice_bridge::MeetingControl>> {
        None
    }
}

/// Shared handle the app keeps.
pub type Engine = Arc<dyn EngineApi>;
