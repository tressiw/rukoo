//! Voice's settings: Ownkey's config schema with its load-time migrations, plus Voice's own keys.
//!
//! `config.json` in the data folder keeps Ownkey's flat snake_case keys, so an import is a copy
//! and the porting spec's tables apply as written. Loading never writes; every migration happens
//! in memory and persists on the next save. Unknown keys survive a load and save, so a newer
//! Voice's settings outlive a round trip through an older one.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::cleanup::{self, CleanupConfig, Correction};
use crate::consent::{self, ConsentState, Disclosure};
use crate::fsutil;
use crate::paths;
use crate::providers::{
    default_endpoint, normalize_provider, provider_from_endpoint, Activity, Hints, ProviderConfig,
    Purpose, AUDIO_PROVIDER_IDS, DICTATION_TIMEOUT, MEETING_TIMEOUT, ORUKEET_MODEL_ID,
    REWRITE_PROVIDER_IDS,
};

pub const CONFIG_VERSION: u64 = 1;

/// The keys Voice offers for dictation, the rewrite key and the meeting stop key, as stored. A
/// config from another platform keeps its key, so every name is accepted on load.
pub const HOTKEYS: [&str; 9] = [
    "right alt",
    "right command",
    "right ctrl",
    "right shift",
    "f13",
    "f14",
    "f15",
    "pause",
    "scroll lock",
];
/// The keys a Windows keyboard has, in Ownkey's order.
pub const WINDOWS_HOTKEYS: [&str; 8] = [
    "right alt",
    "right ctrl",
    "right shift",
    "f13",
    "f14",
    "f15",
    "pause",
    "scroll lock",
];
/// The keys a Mac keyboard has. Pause and Scroll Lock don't exist there; Right Command is the one
/// Mac key Voice adds, and Right Option is the same key as Right Alt.
pub const MACOS_HOTKEYS: [&str; 7] = [
    "right alt",
    "right command",
    "right ctrl",
    "right shift",
    "f13",
    "f14",
    "f15",
];

/// The keys this platform can offer in Settings, so a key the keyboard lacks can't be chosen.
pub fn offered_hotkeys() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &MACOS_HOTKEYS
    } else {
        &WINDOWS_HOTKEYS
    }
}
pub const LANGUAGES: [&str; 11] = [
    "auto", "en", "nl", "de", "fr", "es", "it", "pt", "pl", "ja", "zh",
];
pub const DEFAULT_IDLE_MINUTES: f64 = 20.0;

pub const DEFAULT_AUDIO_ENDPOINT: &str = "https://api.mistral.ai/v1/audio/transcriptions";
pub const DEFAULT_AUDIO_MODEL: &str = "voxtral-mini-latest";
pub const DEFAULT_REWRITE_ENDPOINT: &str = "https://api.mistral.ai/v1/chat/completions";
pub const DEFAULT_REWRITE_MODEL: &str = "mistral-small-latest";

/// `meetings_audio_provider` value that follows the dictation provider.
pub const SAME_AS_DICTATION: &str = "same";

/// Keys that hold API keys. They never appear in Debug output or import reports.
pub const SECRET_KEYS: [&str; 3] = ["audio_api_key", "rewrite_api_key", "meetings_audio_api_key"];

/// Keys an old Ownkey config used for one shared provider; read for migration, then dropped.
const LEGACY_KEYS: [&str; 4] = ["api_key", "endpoint", "model", "chat_endpoint"];
/// Keys Ownkey retired (local rewriting, pyannote) and keys Voice doesn't use.
const DROPPED_KEYS: [&str; 10] = [
    "local_rewrite_directory",
    "local_rewrite_idle_timeout_minutes",
    "pyannote_api_key",
    "meetings_upload_policy",
    "meetings_live_speakers_policy",
    "sample_rate",
    "meetings_remote_policy",
    "meetings_transcription_policy",
    "meetings_live_transcription_policy",
    "meetings_auto_summary",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewriteTone {
    #[default]
    Auto,
    Professional,
    Casual,
    Friendly,
    Concise,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    #[default]
    #[serde(rename = "days7")]
    Days7,
    Keep,
    AfterTranscription,
}

impl Retention {
    pub fn label(self) -> &'static str {
        match self {
            Retention::Days7 => "7 days after transcription",
            Retention::Keep => "Until I delete the meeting",
            Retention::AfterTranscription => "Remove after transcription",
        }
    }
}

/// What a new meeting records of the screen. Only `off` is built in 0.1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenMode {
    #[default]
    Off,
    Frames,
    Video,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiLanguage {
    #[default]
    System,
    En,
    Nl,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PillEdge {
    #[default]
    Bottom,
    Top,
}

/// The user's agreement to send meeting audio to one cloud provider at one host. A different
/// provider or host needs a new agreement, and so does another way of sending.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Consent {
    pub provider: String,
    pub host: String,
    /// An agreement stored before streaming existed reads as `windows`.
    #[serde(default)]
    pub mode: ConsentMode,
}

/// How meeting audio goes to the provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentMode {
    /// Recorded windows of up to 28 seconds, one request each.
    #[default]
    Windows,
    /// A continuous stream to a realtime model while the meeting records.
    Stream,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: u64,

    // Dictation
    pub hotkey: String,
    /// "off" or one of HOTKEYS, never equal to `hotkey`.
    pub rewrite_hotkey: String,
    pub paste_mode: bool,
    pub ready_chime: bool,
    pub pill_edge: PillEdge,
    pub pill_hide_from_capture: bool,

    // Transcription
    pub audio_provider: String,
    pub audio_api_key: String,
    pub audio_endpoint: String,
    pub audio_model: String,
    pub language: String,
    /// A folder that already holds the extracted Orukeet files; "" uses Ownkey's or Voice's own.
    pub local_model_directory: String,
    /// 0 keeps the model loaded.
    pub local_model_idle_timeout_minutes: f64,

    // Dictionary and filler words
    pub vocabulary: Vec<String>,
    pub corrections: Vec<Correction>,
    pub remove_fillers: bool,
    pub filler_languages: Vec<String>,
    /// Comma-separated.
    pub custom_fillers: String,

    // Rewriting
    pub rewrite_provider: String,
    pub rewrite_api_key: String,
    pub rewrite_endpoint: String,
    pub rewrite_model: String,
    pub auto_rewrite: bool,
    pub rewrite_tone: RewriteTone,
    pub rewrite_formatting: bool,
    pub rewrite_custom_instructions: String,

    // Meetings
    /// "same" (follow dictation), "orukeet" or a cloud audio provider.
    pub meetings_audio_provider: String,
    pub meetings_audio_api_key: String,
    pub meetings_audio_endpoint: String,
    pub meetings_audio_model: String,
    pub meetings_transcription_consent: Option<Consent>,
    pub meetings_retention: Retention,
    pub meetings_screen: ScreenMode,
    /// "off" or one of HOTKEYS, never equal to the dictation or rewrite key.
    pub meetings_stop_key: String,
    pub meetings_auto_speakers: bool,
    /// Speaker labels while a meeting records. `meetings_auto_speakers` is Ownkey's "label after
    /// Stop", which the import keeps filling but nothing reads.
    pub meetings_live_speakers: bool,

    // General
    pub ui_language: UiLanguage,
    pub ui_theme: Theme,
    /// The daily update check. Off unless the user turns it on.
    pub updates_check: bool,
    /// When Ownkey's settings were last imported, in ms since the Unix epoch.
    pub ownkey_imported_at: Option<u64>,

    /// Keys this version doesn't know, kept so they survive a save.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    /// config.json was not valid JSON. It was moved to `kept_at` so a save can't overwrite the
    /// user's keys and dictionary; Voice continues with defaults.
    Damaged {
        kept_at: PathBuf,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "Couldn't read Voice's settings: {error}"),
            ConfigError::Damaged { kept_at } => write!(
                f,
                "Voice's settings file was damaged. Voice started with default settings and kept the old file at {}.",
                kept_at.display()
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(error: io::Error) -> Self {
        ConfigError::Io(error)
    }
}

impl Config {
    /// Ownkey's defaults plus Voice's. In development mode the keys are F13 and F14, so a dev
    /// copy never answers the installed Ownkey's Right Alt and Right Ctrl.
    pub fn defaults(dev_mode: bool) -> Config {
        let (hotkey, rewrite_hotkey) = default_hotkeys(dev_mode);
        Config {
            version: CONFIG_VERSION,
            hotkey: hotkey.into(),
            rewrite_hotkey: rewrite_hotkey.into(),
            paste_mode: true,
            ready_chime: false,
            pill_edge: PillEdge::Bottom,
            pill_hide_from_capture: true,
            audio_provider: "mistral".into(),
            audio_api_key: String::new(),
            audio_endpoint: DEFAULT_AUDIO_ENDPOINT.into(),
            audio_model: DEFAULT_AUDIO_MODEL.into(),
            language: "auto".into(),
            local_model_directory: String::new(),
            local_model_idle_timeout_minutes: DEFAULT_IDLE_MINUTES,
            vocabulary: Vec::new(),
            corrections: Vec::new(),
            remove_fillers: true,
            filler_languages: cleanup::DEFAULT_FILLER_LANGUAGES
                .iter()
                .map(|c| c.to_string())
                .collect(),
            custom_fillers: String::new(),
            rewrite_provider: "mistral".into(),
            rewrite_api_key: String::new(),
            rewrite_endpoint: DEFAULT_REWRITE_ENDPOINT.into(),
            rewrite_model: DEFAULT_REWRITE_MODEL.into(),
            auto_rewrite: false,
            rewrite_tone: RewriteTone::Auto,
            rewrite_formatting: true,
            rewrite_custom_instructions: String::new(),
            // Decision 10: meetings transcribe on this computer unless the user picks a cloud provider.
            meetings_audio_provider: "orukeet".into(),
            meetings_audio_api_key: String::new(),
            meetings_audio_endpoint: String::new(),
            meetings_audio_model: ORUKEET_MODEL_ID.into(),
            meetings_transcription_consent: None,
            meetings_retention: Retention::Days7,
            meetings_screen: ScreenMode::Off,
            meetings_stop_key: "off".into(),
            meetings_auto_speakers: false,
            meetings_live_speakers: true,
            ui_language: UiLanguage::System,
            ui_theme: Theme::System,
            updates_check: false,
            ownkey_imported_at: None,
            extra: Map::new(),
        }
    }

    /// Loads `config.json` from the data folder; defaults when there is none.
    pub fn load(data_dir: &Path) -> Result<Config, ConfigError> {
        Config::load_as(data_dir, paths::dev_mode())
    }

    pub fn load_as(data_dir: &Path, dev_mode: bool) -> Result<Config, ConfigError> {
        let path = paths::config_file(data_dir);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Config::defaults(dev_mode))
            }
            Err(error) => return Err(error.into()),
        };
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value @ Value::Object(_)) => Ok(Config::from_value(&value, dev_mode)),
            // Differs from Ownkey on purpose: Ownkey silently started over with defaults and
            // overwrote the file, keys and dictionary included, on the next save.
            _ => {
                let kept_at = data_dir.join(format!("config.damaged-{}.json", fsutil::now_ms()));
                fs::rename(&path, &kept_at)?;
                Err(ConfigError::Damaged { kept_at })
            }
        }
    }

    /// Whether this data folder has no settings yet: Voice's first start there.
    pub fn is_first_run(data_dir: &Path) -> bool {
        !paths::config_file(data_dir).exists()
    }

    /// A config from any JSON value, with every migration and check applied.
    pub fn from_value(value: &Value, dev_mode: bool) -> Config {
        let normalized = normalize(value, dev_mode);
        serde_json::from_value(Value::Object(normalized))
            .unwrap_or_else(|_| Config::defaults(dev_mode))
    }

    /// Writes `config.json` atomically: a temporary file in the same folder, flushed to disk,
    /// then renamed over the old one. On Unix the file is readable by its owner only.
    pub fn save(&self, data_dir: &Path) -> io::Result<()> {
        fs::create_dir_all(data_dir)?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        bytes.push(b'\n');
        fsutil::write_atomic(&paths::config_file(data_dir), &bytes)
    }

    pub fn cleanup(&self) -> CleanupConfig {
        CleanupConfig {
            remove_fillers: self.remove_fillers,
            filler_languages: self.filler_languages.clone(),
            custom_fillers: self.custom_fillers.clone(),
            corrections: self.corrections.clone(),
        }
    }

    pub fn dictation_audio(&self) -> ProviderConfig {
        ProviderConfig {
            provider: self.audio_provider.clone(),
            api_key: self.audio_api_key.clone(),
            endpoint: self.audio_endpoint.clone(),
            model: self.audio_model.clone(),
            timeout: DICTATION_TIMEOUT,
            purpose: Purpose::Dictation,
        }
    }

    /// The meeting transcription provider, with "same" resolved to the dictation provider.
    pub fn meeting_audio(&self) -> ProviderConfig {
        if self.meetings_audio_provider == SAME_AS_DICTATION {
            return ProviderConfig {
                timeout: MEETING_TIMEOUT,
                ..self.dictation_audio()
            };
        }
        ProviderConfig {
            provider: self.meetings_audio_provider.clone(),
            api_key: self.meetings_audio_api_key.clone(),
            endpoint: self.meetings_audio_endpoint.clone(),
            model: self.meetings_audio_model.clone(),
            timeout: MEETING_TIMEOUT,
            purpose: Purpose::Meetings,
        }
    }

    pub fn rewrite(&self) -> ProviderConfig {
        ProviderConfig {
            provider: self.rewrite_provider.clone(),
            api_key: self.rewrite_api_key.clone(),
            endpoint: self.rewrite_endpoint.clone(),
            model: self.rewrite_model.clone(),
            timeout: DICTATION_TIMEOUT,
            purpose: Purpose::Rewrite,
        }
    }

    /// Dictionary words and language for a dictation recording. Orukeet ignores the language.
    pub fn dictation_hints(&self) -> Hints {
        self.hints_for(&self.audio_provider)
    }

    pub fn meeting_hints(&self) -> Hints {
        self.hints_for(&self.meeting_audio().provider)
    }

    fn hints_for(&self, provider: &str) -> Hints {
        let language = if provider == "orukeet" || self.language == "auto" {
            None
        } else {
            Some(self.language.clone())
        };
        Hints {
            vocabulary: self.vocabulary.clone(),
            language,
        }
    }

    /// How long the local model stays loaded without use; zero keeps it loaded.
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.local_model_idle_timeout_minutes.max(0.0) * 60.0)
    }

    /// Replaces "same as dictation" with the dictation provider it follows, its key, endpoint
    /// and model. Voice has no "same" choice (decision 10): it would move meeting audio to the
    /// cloud whenever someone changed the dictation provider. Consent stays valid, because it
    /// names the same provider and host. Returns whether anything changed.
    pub fn resolve_same_meeting_provider(&mut self) -> bool {
        if self.meetings_audio_provider != SAME_AS_DICTATION {
            return false;
        }
        self.meetings_audio_provider = self.audio_provider.clone();
        self.meetings_audio_api_key = self.audio_api_key.clone();
        if self.audio_provider == "orukeet" {
            self.meetings_audio_endpoint.clear();
            self.meetings_audio_model = ORUKEET_MODEL_ID.into();
        } else {
            self.meetings_audio_endpoint = self.audio_endpoint.clone();
            self.meetings_audio_model = self.audio_model.clone();
        }
        true
    }

    /// Whether meeting audio may go to the meeting provider.
    pub fn meeting_consent(&self) -> ConsentState {
        consent::state(
            &self.meeting_audio(),
            self.meetings_transcription_consent.as_ref(),
        )
    }

    /// What Settings shows before the user agrees to send meeting audio; None when the
    /// provider keeps audio on this computer.
    pub fn meeting_disclosure(&self) -> Option<Disclosure> {
        consent::disclosure(&self.meeting_audio())
    }

    /// Records the user's agreement for the current meeting provider and host. Returns false
    /// when that provider needs none.
    pub fn grant_meeting_consent(&mut self) -> bool {
        match self.meeting_consent() {
            ConsentState::NotNeeded => false,
            ConsentState::Granted => true,
            ConsentState::Required {
                provider,
                host,
                mode,
                ..
            } => {
                self.meetings_transcription_consent = Some(Consent {
                    provider,
                    host,
                    mode,
                });
                true
            }
        }
    }

    /// Selects another dictation provider. Credentials are provider-specific: the key is cleared
    /// so it can't be sent to the new provider by accident.
    pub fn switch_audio_provider(&mut self, provider: &str) {
        let id = normalize_provider(provider, "mistral");
        self.audio_api_key.clear();
        if id == "orukeet" {
            self.audio_endpoint.clear();
            self.audio_model = ORUKEET_MODEL_ID.into();
        } else {
            self.audio_endpoint = default_endpoint(&id, Activity::Audio).into();
            self.audio_model.clear();
        }
        self.audio_provider = id;
    }

    pub fn switch_rewrite_provider(&mut self, provider: &str) {
        let id = normalize_provider(provider, "mistral");
        self.rewrite_api_key.clear();
        self.rewrite_endpoint = default_endpoint(&id, Activity::Rewrite).into();
        self.rewrite_model.clear();
        self.rewrite_provider = id;
    }

    pub fn switch_meeting_provider(&mut self, provider: &str) {
        let choice = provider.trim().to_lowercase();
        let id = if choice == SAME_AS_DICTATION {
            choice
        } else {
            normalize_provider(&choice, SAME_AS_DICTATION)
        };
        self.meetings_audio_api_key.clear();
        match id.as_str() {
            "orukeet" => {
                self.meetings_audio_endpoint.clear();
                self.meetings_audio_model = ORUKEET_MODEL_ID.into();
            }
            SAME_AS_DICTATION => {
                self.meetings_audio_endpoint.clear();
                self.meetings_audio_model.clear();
            }
            _ => {
                self.meetings_audio_endpoint = default_endpoint(&id, Activity::Audio).into();
                self.meetings_audio_model.clear();
            }
        }
        self.meetings_audio_provider = id;
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut value {
            for key in SECRET_KEYS {
                if map
                    .get(key)
                    .and_then(Value::as_str)
                    .is_some_and(|k| !k.is_empty())
                {
                    map.insert(key.into(), json!("<redacted>"));
                }
            }
            // A bucket secret from before Voice kept it with DPAPI, until the engine moves it there.
            if let Some(secret) = map
                .get_mut("storage_sync")
                .and_then(|sync| sync.get_mut("secret"))
                .filter(|s| s.as_str().is_some_and(|s| !s.is_empty()))
            {
                *secret = json!("<redacted>");
            }
        }
        write!(f, "Config {value}")
    }
}

/// The problem with a provider form in Settings, as Ownkey's Save reported it, or None.
pub fn provider_form_problem(
    purpose: Purpose,
    provider: &str,
    endpoint: &str,
    model: &str,
) -> Option<&'static str> {
    let missing = endpoint.trim().is_empty() || model.trim().is_empty();
    match purpose {
        Purpose::Dictation if provider != "orukeet" && missing => {
            Some("Choose an audio endpoint and model before saving.")
        }
        Purpose::Rewrite if missing => Some("Choose a rewrite endpoint and model before saving."),
        Purpose::Meetings if provider != "orukeet" && provider != SAME_AS_DICTATION && missing => {
            Some("Choose an endpoint and model for meeting transcription before saving.")
        }
        _ => None,
    }
}

fn default_hotkeys(dev_mode: bool) -> (&'static str, &'static str) {
    default_hotkeys_on(dev_mode, cfg!(target_os = "macos"))
}

/// The keys a fresh config gets. A development copy uses F13 and F14 on every platform, so it
/// never answers the installed Ownkey's keys; on a Mac an installed copy starts with Right Option
/// and Right Command: most Mac laptops have no Right Control key, and Fn/Globe already opens the
/// emoji picker and macOS dictation.
pub fn default_hotkeys_on(dev_mode: bool, macos: bool) -> (&'static str, &'static str) {
    if dev_mode {
        ("f13", "f14")
    } else if macos {
        ("right alt", "right command")
    } else {
        ("right alt", "right ctrl")
    }
}

/// Python truthiness for a JSON value.
pub(crate) fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The first of `keys` that holds a non-empty string, like Python's `a or b or default`.
fn first_text(map: &Map<String, Value>, keys: &[&str], default: &str) -> String {
    keys.iter()
        .filter_map(|key| map.get(*key).and_then(Value::as_str))
        .find(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn sanitized_choice(value: Option<&Value>, allowed: &[&str], default: &str) -> String {
    let choice = text(value).trim().to_lowercase();
    if allowed.contains(&choice.as_str()) {
        choice
    } else {
        default.to_string()
    }
}

/// An enum value from its stored name, or its default.
fn sanitized_enum<T: DeserializeOwned + Serialize + Default>(value: Option<&Value>) -> Value {
    let name = text(value).trim().to_lowercase();
    let parsed: T = serde_json::from_value(Value::String(name)).unwrap_or_default();
    serde_json::to_value(parsed).unwrap_or(Value::Null)
}

fn idle_minutes(value: Option<&Value>) -> f64 {
    let minutes = match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    };
    minutes
        .filter(|m| m.is_finite() && *m >= 0.0)
        .unwrap_or(DEFAULT_IDLE_MINUTES)
}

fn consent_value(value: Option<&Value>) -> Value {
    let Some(Value::Object(map)) = value else {
        return Value::Null;
    };
    let provider = normalize_provider(&text(map.get("provider")), "");
    let host = text(map.get("host")).trim().to_lowercase();
    if provider.is_empty()
        || provider == "orukeet"
        || !AUDIO_PROVIDER_IDS.contains(&provider.as_str())
        || host.is_empty()
    {
        return Value::Null;
    }
    // Only an explicit stream agreement covers streaming; anything else, including an agreement
    // stored before streaming existed, is one for recorded windows.
    let mode = if text(map.get("mode")).trim().eq_ignore_ascii_case("stream") {
        ConsentMode::Stream
    } else {
        ConsentMode::Windows
    };
    json!({ "provider": provider, "host": host, "mode": mode })
}

/// Ownkey's `load_config` migrations and checks, then Voice's own, producing a complete object.
pub(crate) fn normalize(value: &Value, dev_mode: bool) -> Map<String, Value> {
    let disk = match value {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    let defaults = Config::defaults(dev_mode);
    let Ok(Value::Object(mut cfg)) = serde_json::to_value(&defaults) else {
        unreachable!("a Config always serializes to an object");
    };
    for (key, value) in &disk {
        cfg.insert(key.clone(), value.clone());
    }

    // Old Ownkey configs had one provider for both activities. Explicit new keys win.
    let audio_endpoint = first_text(
        &disk,
        &["audio_endpoint", "endpoint"],
        DEFAULT_AUDIO_ENDPOINT,
    );
    let rewrite_endpoint = first_text(
        &disk,
        &["rewrite_endpoint", "chat_endpoint"],
        DEFAULT_REWRITE_ENDPOINT,
    );
    let audio_provider = match disk
        .get("audio_provider")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    {
        Some(provider) => provider.to_string(),
        None => provider_from_endpoint(&audio_endpoint, "mistral"),
    };
    let rewrite_provider = match disk
        .get("rewrite_provider")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    {
        Some(provider) => provider.to_string(),
        None => provider_from_endpoint(&rewrite_endpoint, "mistral"),
    };
    cfg.insert("audio_endpoint".into(), json!(audio_endpoint));
    cfg.insert("rewrite_endpoint".into(), json!(rewrite_endpoint));
    cfg.insert("audio_provider".into(), json!(audio_provider));
    cfg.insert("rewrite_provider".into(), json!(rewrite_provider));
    cfg.insert(
        "audio_api_key".into(),
        json!(first_text(&disk, &["audio_api_key", "api_key"], "")),
    );
    cfg.insert(
        "rewrite_api_key".into(),
        json!(first_text(&disk, &["rewrite_api_key", "api_key"], "")),
    );
    cfg.insert(
        "audio_model".into(),
        json!(first_text(
            &disk,
            &["audio_model", "model"],
            DEFAULT_AUDIO_MODEL
        )),
    );
    for key in LEGACY_KEYS {
        cfg.remove(key);
    }

    let audio = normalize_provider(&text(cfg.get("audio_provider")), "mistral");
    if !AUDIO_PROVIDER_IDS.contains(&audio.as_str()) {
        cfg.insert("audio_endpoint".into(), json!(DEFAULT_AUDIO_ENDPOINT));
        cfg.insert("audio_model".into(), json!(DEFAULT_AUDIO_MODEL));
        cfg.insert("audio_provider".into(), json!("mistral"));
    } else {
        cfg.insert("audio_provider".into(), json!(audio));
    }
    if audio == "orukeet" {
        // The local model has no endpoint and one model id; Ownkey refilled the Mistral URL here.
        cfg.insert("audio_endpoint".into(), json!(""));
        cfg.insert("audio_model".into(), json!(ORUKEET_MODEL_ID));
    }

    // Retired local rewriting must never silently switch existing users to cloud edits.
    let rewrite_raw = text(cfg.get("rewrite_provider")).trim().to_lowercase();
    if rewrite_raw == "local_rewrite" || rewrite_raw == "local (small llm)" {
        cfg.insert("rewrite_provider".into(), json!("mistral"));
        cfg.insert("rewrite_api_key".into(), json!(""));
        cfg.insert("rewrite_endpoint".into(), json!(DEFAULT_REWRITE_ENDPOINT));
        cfg.insert("rewrite_model".into(), json!(DEFAULT_REWRITE_MODEL));
        cfg.insert("auto_rewrite".into(), json!(false));
        cfg.insert("rewrite_hotkey".into(), json!("off"));
    }
    for key in DROPPED_KEYS {
        cfg.remove(key);
    }
    let rewrite = normalize_provider(&text(cfg.get("rewrite_provider")), "mistral");
    if !REWRITE_PROVIDER_IDS.contains(&rewrite.as_str()) {
        cfg.insert("rewrite_endpoint".into(), json!(DEFAULT_REWRITE_ENDPOINT));
        cfg.insert("rewrite_model".into(), json!(DEFAULT_REWRITE_MODEL));
        cfg.insert("rewrite_provider".into(), json!("mistral"));
    } else {
        cfg.insert("rewrite_provider".into(), json!(rewrite));
    }

    let (default_hotkey, default_rewrite_hotkey) = default_hotkeys(dev_mode);
    let hotkey = sanitized_choice(cfg.get("hotkey"), &HOTKEYS, default_hotkey);
    let mut rewrite_keys = vec!["off"];
    rewrite_keys.extend(HOTKEYS);
    let mut rewrite_hotkey = sanitized_choice(
        cfg.get("rewrite_hotkey"),
        &rewrite_keys,
        default_rewrite_hotkey,
    );
    if rewrite_hotkey == hotkey {
        rewrite_hotkey = "off".into();
    }
    let mut stop_key = sanitized_choice(cfg.get("meetings_stop_key"), &rewrite_keys, "off");
    if stop_key == hotkey || stop_key == rewrite_hotkey {
        stop_key = "off".into();
    }
    cfg.insert("hotkey".into(), json!(hotkey));
    cfg.insert("rewrite_hotkey".into(), json!(rewrite_hotkey));
    cfg.insert("meetings_stop_key".into(), json!(stop_key));
    cfg.insert(
        "language".into(),
        json!(sanitized_choice(cfg.get("language"), &LANGUAGES, "auto")),
    );
    cfg.insert(
        "rewrite_tone".into(),
        sanitized_enum::<RewriteTone>(cfg.get("rewrite_tone")),
    );

    cfg.insert(
        "vocabulary".into(),
        json!(cleanup::normalize_vocabulary(
            cfg.get("vocabulary").unwrap_or(&Value::Null)
        )),
    );
    cfg.insert(
        "corrections".into(),
        json!(cleanup::normalize_corrections(
            cfg.get("corrections").unwrap_or(&Value::Null)
        )),
    );
    cfg.insert(
        "filler_languages".into(),
        json!(cleanup::normalize_filler_languages(
            cfg.get("filler_languages").unwrap_or(&Value::Null)
        )),
    );
    let custom = cleanup::normalize_vocabulary(cfg.get("custom_fillers").unwrap_or(&Value::Null));
    cfg.insert("custom_fillers".into(), json!(custom.join(", ")));

    for key in [
        "paste_mode",
        "ready_chime",
        "pill_hide_from_capture",
        "remove_fillers",
        "auto_rewrite",
        "rewrite_formatting",
        "meetings_auto_speakers",
        "updates_check",
    ] {
        let flag = truthy(cfg.get(key));
        cfg.insert(key.into(), json!(flag));
    }
    let live_speakers = cfg
        .get("meetings_live_speakers")
        .is_none_or(|v| truthy(Some(v)));
    cfg.insert("meetings_live_speakers".into(), json!(live_speakers));
    for key in [
        "audio_api_key",
        "audio_model",
        "rewrite_api_key",
        "rewrite_model",
        "rewrite_custom_instructions",
        "local_model_directory",
    ] {
        let value = text(cfg.get(key));
        cfg.insert(key.into(), json!(value));
    }
    cfg.insert(
        "local_model_idle_timeout_minutes".into(),
        json!(idle_minutes(cfg.get("local_model_idle_timeout_minutes"))),
    );

    let meeting_choice = {
        let raw = text(cfg.get("meetings_audio_provider"))
            .trim()
            .to_lowercase();
        let raw = if raw.is_empty() {
            SAME_AS_DICTATION.to_string()
        } else {
            raw
        };
        if raw == SAME_AS_DICTATION {
            raw
        } else {
            let id = normalize_provider(&raw, SAME_AS_DICTATION);
            if AUDIO_PROVIDER_IDS.contains(&id.as_str()) {
                id
            } else {
                SAME_AS_DICTATION.to_string()
            }
        }
    };
    for key in [
        "meetings_audio_api_key",
        "meetings_audio_endpoint",
        "meetings_audio_model",
    ] {
        let value = text(cfg.get(key)).trim().to_string();
        cfg.insert(key.into(), json!(value));
    }
    if meeting_choice == "orukeet" {
        cfg.insert("meetings_audio_endpoint".into(), json!(""));
        cfg.insert("meetings_audio_model".into(), json!(ORUKEET_MODEL_ID));
    }
    cfg.insert("meetings_audio_provider".into(), json!(meeting_choice));
    cfg.insert(
        "meetings_retention".into(),
        json!(sanitized_choice(
            cfg.get("meetings_retention"),
            &["days7", "keep", "after_transcription"],
            "days7"
        )),
    );
    cfg.insert(
        "meetings_screen".into(),
        sanitized_enum::<ScreenMode>(cfg.get("meetings_screen")),
    );
    cfg.insert(
        "ui_language".into(),
        sanitized_enum::<UiLanguage>(cfg.get("ui_language")),
    );
    cfg.insert(
        "ui_theme".into(),
        sanitized_enum::<Theme>(cfg.get("ui_theme")),
    );
    cfg.insert(
        "pill_edge".into(),
        sanitized_enum::<PillEdge>(cfg.get("pill_edge")),
    );
    let consent = consent_value(cfg.get("meetings_transcription_consent"));
    cfg.insert("meetings_transcription_consent".into(), consent);
    let imported = cfg
        .get("ownkey_imported_at")
        .and_then(Value::as_u64)
        .map_or(Value::Null, |ms| json!(ms));
    cfg.insert("ownkey_imported_at".into(), imported);
    // A newer Voice may have written a higher version; never lower it.
    let version = cfg
        .get("version")
        .and_then(Value::as_u64)
        .unwrap_or(CONFIG_VERSION)
        .max(CONFIG_VERSION);
    cfg.insert("version".into(), json!(version));
    cfg
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_hides_an_old_bucket_secret() {
        let mut cfg = Config::defaults(true);
        cfg.extra.insert(
            "storage_sync".into(),
            json!({"bucket": "meetings", "secret": "bucket-secret-1234"}),
        );
        let text = format!("{cfg:?}");
        assert!(!text.contains("bucket-secret-1234"), "{text}");
        assert!(text.contains("<redacted>"));
        assert!(text.contains("meetings"), "the rest stays readable");
        // An empty secret stays as it is.
        cfg.extra
            .insert("storage_sync".into(), json!({"secret": ""}));
        assert!(!format!("{cfg:?}").contains("<redacted>"));
    }
}
