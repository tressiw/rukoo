//! Settings as voice/ui/API.md defines it: the `SettingsState` JSON, and every `settings_*`
//! command on top of voice-core's `Config`.
//!
//! The page uses dotted keys (`dictation.hotkey`); `config.json` keeps Ownkey's flat keys
//! (`hotkey`). [`keys`] maps between them and checks every value.

pub mod keys;
pub mod ownkey;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use voice_core::cleanup::{self, CleanupConfig};
use voice_core::config::{provider_form_problem, Config, Consent, LANGUAGES};
use voice_core::consent::ConsentState;
use voice_core::local::{self, LocalState, Location, ModelErrorKind, ModelSpec, Stage};
use voice_core::providers::stream::{streams, REALTIME_MODEL_PREFIX};
use voice_core::providers::{
    normalize_provider, provider_endpoints, Activity, ProviderConfig, Purpose, AUDIO_PROVIDER_IDS,
    MODELS_TIMEOUT, ORUKEET_MODEL_ID, REWRITE_PROVIDER_IDS,
};
use voice_core::speakers::{cpu_features, Unavailable};
use voice_core::{ErrorKind, ProviderError};
use voice_meetings::{LabelState, LabelStatus};

use crate::api::{ApiError, Folder, Locale, Theme};
use crate::engine::{lock, Core};

/// Download progress goes out at most ten times a second (API.md).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Slow parts of the state (folder sizes, Ownkey's files and process) are read this often.
const SLOW_REFRESH: Duration = Duration::from_secs(10);

/// The models Settings shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Model {
    Orukeet,
    Nemotron,
}

impl Model {
    pub fn parse(id: &str) -> Option<Model> {
        match id {
            "orukeet" => Some(Model::Orukeet),
            "nemotron" => Some(Model::Nemotron),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Model::Orukeet => "orukeet",
            Model::Nemotron => "nemotron",
        }
    }

    pub fn spec(self) -> ModelSpec {
        match self {
            Model::Orukeet => ModelSpec::orukeet(),
            Model::Nemotron => ModelSpec::nemotron(),
        }
    }
}

#[derive(Default)]
struct Download {
    stage: Option<&'static str>,
    percent: Option<u64>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
    running: bool,
}

#[derive(Clone)]
struct Slow {
    at: Instant,
    used_bytes: u64,
    ownkey_running: bool,
    ownkey_keys: Vec<String>,
    ownkey_found: bool,
}

/// Settings' state that isn't in `config.json`.
#[derive(Default)]
pub struct SettingsRuntime {
    downloads: Mutex<HashMap<Model, Download>>,
    updates: Mutex<(String, Option<String>)>,
    slow: Mutex<Option<Slow>>,
    start_page: Mutex<Option<String>>,
    last_progress: Mutex<Option<Instant>>,
}

impl SettingsRuntime {
    pub fn new(start_page: Option<String>) -> SettingsRuntime {
        SettingsRuntime {
            updates: Mutex::new(("idle".into(), None)),
            start_page: Mutex::new(start_page),
            ..SettingsRuntime::default()
        }
    }

    /// The page Settings opens at, once: the import after the first start.
    pub fn take_start_page(&self) -> Option<String> {
        lock(&self.start_page).take()
    }

    /// The next state reads folder sizes and Ownkey's files again.
    pub fn forget_storage(&self) {
        lock(&self.slow).take();
    }
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new("invalid_value", message)
}

pub fn provider_error(error: &ProviderError) -> ApiError {
    let code = match error.kind {
        ErrorKind::Auth => "auth",
        ErrorKind::Network => "network",
        ErrorKind::Timeout => "timeout",
        ErrorKind::Provider => "provider",
        ErrorKind::Config => "config",
        ErrorKind::LocalMissingModel => "failed",
    };
    ApiError::new(code, error.message.clone())
}

// The state --------------------------------------------------------------------------------

pub fn state(core: &Core) -> Value {
    let cfg = core.config();
    let looks = core.looks();
    let start_at_login = !core.opts.dev_mode && core.svc.autostart.is_on();
    let slow = slow(core);
    let (update_state, update_version) = lock(&core.settings.updates).clone();
    let sync = crate::sync::sync_view(core);
    let secret_saved = core.sync.as_ref().is_some_and(|s| s.secret_saved());
    json!({
        "seq": core.settings_seq.load(Ordering::SeqCst),
        "version": core.opts.version,
        "devMode": core.opts.dev_mode,
        "platform": platform(),
        // Which features' Settings this Voice shows (voice/ui/API.md). Bucket sync's shows where
        // its store opened on a platform that keeps the secret safe (`sync.available`).
        "features": { "screen": true, "speakers": true, "sync": sync["available"] },
        "theme": match looks.theme { Theme::Light => "light", Theme::Dark => "dark" },
        "locale": match looks.locale { Locale::En => "en", Locale::Nl => "nl" },
        "config": keys::view(&cfg, start_at_login, secret_saved),
        "providers": providers(),
        "dictation": { "status": dictation_status(core, &cfg) },
        "rukoo": rukoo(core),
        "permissions": crate::permissions::view(&*core.svc.permissions),
        "models": {
            "orukeet": model_state(core, &cfg, Model::Orukeet),
            "nemotron": model_state(core, &cfg, Model::Nemotron),
        },
        "speakers": speakers_state(core, &cfg),
        "storage": {
            "folder": core.meetings.library().root().display().to_string(),
            "usedBytes": slow.used_bytes,
            "freeBytes": core.svc.system.free_bytes(core.meetings.library().root()).unwrap_or(0),
        },
        "screen": screen(core),
        "updates": { "state": update_state, "version": update_version },
        "ownkey": ownkey::state(core, &cfg, &slow.ownkey_keys, slow.ownkey_found, slow.ownkey_running),
        "sync": sync,
    })
}

/// What this computer can record of the screen, from `rukoo-voice-screen probe`: `null` until
/// the probe answered.
fn screen(core: &Core) -> Value {
    let supervisor = &core.meetings.screen;
    match supervisor.capabilities() {
        Some(caps) => json!({
            "available": supervisor.available(),
            "hardwareEncoder": caps.hardware_encoder.map(|e| e.name),
            "ocr": caps.ocr,
            "monitors": caps.monitors.len(),
        }),
        None => Value::Null,
    }
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn slow(core: &Core) -> Slow {
    let mut cached = lock(&core.settings.slow);
    if let Some(slow) = cached.as_ref().filter(|s| s.at.elapsed() < SLOW_REFRESH) {
        return slow.clone();
    }
    let (ownkey_found, ownkey_keys) = ownkey::keys(core);
    let slow = Slow {
        at: Instant::now(),
        used_bytes: core.meetings.library().usage_bytes(),
        ownkey_running: core.svc.system.ownkey_running(),
        ownkey_keys,
        ownkey_found,
    };
    *cached = Some(slow.clone());
    slow
}

/// voice-core's registry in Ownkey's order (ownkey-providers.md 2). An audio provider with a
/// realtime model names its prefix in `streamingPrefix`: such a model streams meetings.
pub fn providers() -> Value {
    let audio: Vec<Value> = AUDIO_PROVIDER_IDS
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "endpoints": provider_endpoints(id, Activity::Audio),
                "streamingPrefix": (*id == "mistral").then_some(REALTIME_MODEL_PREFIX),
            })
        })
        .collect();
    let rewrite: Vec<Value> = REWRITE_PROVIDER_IDS
        .iter()
        .map(|id| json!({ "id": id, "endpoints": provider_endpoints(id, Activity::Rewrite) }))
        .collect();
    json!({ "audio": audio, "rewrite": rewrite })
}

/// Settings > Speaker labels: who labels new meetings, whether they can, and what each meeting
/// that transcribes now is doing (voice/ui/API.md, `SpeakersState`).
fn speakers_state(core: &Core, cfg: &Config) -> Value {
    let audio = cfg.meeting_audio();
    let mistral = streams(&audio);
    let availability = core.svc.speech.speaker_availability();
    let mut detail = None;
    let mut missing: Vec<String> = Vec::new();
    let reason: Option<&str> = if !cfg.meetings_live_speakers {
        Some("off")
    } else if mistral {
        // The stream labels the call; Nemotron's state doesn't matter for these meetings. The
        // stream needs a key (MistralRealtime::new) and a `stream` agreement before it starts.
        if audio.api_key.trim().is_empty() {
            Some("key")
        } else {
            matches!(cfg.meeting_consent(), ConsentState::Required { .. }).then_some("consent")
        }
    } else {
        match &availability {
            Ok(_) => None,
            Err(why) => {
                match why {
                    Unavailable::Cpu { missing: names } => missing.clone_from(names),
                    Unavailable::RuntimeFailed { detail: text }
                    | Unavailable::ModelDamaged { detail: text } => detail = Some(text.clone()),
                    _ => {}
                }
                Some(why.code())
            }
        }
    };
    let dev = core.opts.dev_mode;
    let live: Vec<Value> = core
        .meetings
        .labels()
        .into_iter()
        .map(|(meeting, s)| {
            json!({
                "meeting": meeting,
                "mode": s.mode,
                "state": s.state,
                "speakers": s.speakers,
                "reason": s.reason,
                // For the live run, which kills the worker on purpose.
                "workerPid": s.worker_pid.filter(|_| dev),
            })
        })
        .collect();
    json!({
        "engine": if mistral { "mistral" } else { "nemotron" },
        "ready": reason.is_none(),
        "reason": reason,
        "detail": detail,
        "missing": missing,
        "cpu": cpu_features(),
        "runtime": availability.ok().map(|info| json!({
            "location": info.location,
            "commit": info.commit.chars().take(7).collect::<String>(),
        })),
        "live": live,
    })
}

/// Whether a meeting labels speakers with Nemotron now, or soon again after a restart of its
/// worker: removing the model would stop it.
fn labelling_with_nemotron(live: &[(String, LabelStatus)]) -> bool {
    live.iter().any(|(_, s)| {
        s.mode != "mistral"
            && !matches!(
                s.state,
                LabelState::Finished | LabelState::Failed | LabelState::Off
            )
    })
}

fn dictation_status(core: &Core, cfg: &Config) -> &'static str {
    let audio = cfg.dictation_audio();
    if audio.provider == "orukeet" {
        if core.registry(cfg).locate(&ModelSpec::orukeet()).is_none() {
            return "modelMissing";
        }
        if core.svc.speech.local_status().state == LocalState::Loading {
            return "loadingModel";
        }
    } else if voice_core::providers::provider_requires_key(&audio.provider, &audio.endpoint)
        && audio.api_key.trim().is_empty()
    {
        return "noKey";
    }
    if core.dictation.mic_error().is_some() {
        return "noMic";
    }
    "ready"
}

fn rukoo(core: &Core) -> Value {
    let bridge = core.meetings.bridge.as_ref();
    let info = bridge.map(|b| b.rukoo()).unwrap_or_default();
    json!({
        "state": if info.state.is_empty() { "notRunning" } else { info.state },
        "clientVersion": info.client_version,
        "protocol": info.protocol,
        "port": bridge.and_then(|b| b.port()),
        "discoveryPath": bridge.map(|b| b.discovery_path().display().to_string()),
        "behind": info.behind,
    })
}

fn model_state(core: &Core, cfg: &Config, model: Model) -> Value {
    let spec = model.spec();
    let located = core.registry(cfg).locate(&spec);
    let downloads = lock(&core.settings.downloads);
    let download = downloads.get(&model);
    let active_status = (model == Model::Orukeet && cfg.audio_provider == "orukeet")
        .then(|| core.svc.speech.local_status());
    let stage = match download.and_then(|d| d.stage) {
        Some(stage) => stage,
        None if located.is_some() => "installed",
        None if download.is_some_and(|d| d.error.is_some()) => "failed",
        None => "missing",
    };
    let error = download
        .and_then(|d| d.error.clone())
        .or_else(|| active_status.as_ref().and_then(|s| s.error.clone()));
    json!({
        "id": model.id(),
        "name": if model == Model::Orukeet { "Orukeet" } else { spec.label.as_str() },
        "sizeBytes": spec.archive.as_ref().map_or_else(|| spec.extracted_size(), |a| a.size),
        "stage": stage,
        "percent": download.and_then(|d| d.percent),
        "location": located.as_ref().map(|l| match l.location {
            Location::Custom => "folder",
            Location::Ownkey => "ownkey",
            Location::Voice => "voice",
        }),
        "path": located.as_ref().map(|l| l.dir.display().to_string()),
        "removable": located.as_ref().is_some_and(local::Located::removable),
        "active": active_status.map(|s| match s.state {
            LocalState::Unloaded => "unloaded",
            LocalState::Loading => "loading",
            LocalState::Loaded => "loaded",
            LocalState::Error => "error",
        }),
        "error": error,
    })
}

// Commands ---------------------------------------------------------------------------------

pub fn set(core: &Arc<Core>, key: &str, value: Value) -> Result<(), ApiError> {
    if key == "dictation.startAtLogin" {
        if core.opts.dev_mode {
            return Err(ApiError::new(
                "dev_mode",
                "A development copy never starts at login.",
            ));
        }
        let on = value
            .as_bool()
            .ok_or_else(|| invalid("Start at login takes true or false."))?;
        core.svc
            .autostart
            .set(on)
            .map_err(|e| ApiError::new("failed", e))?;
        core.settings_changed();
        return Ok(());
    }
    core.commit(|cfg| keys::set(cfg, key, value))
}

/// One provider form, checked and saved as a whole (ownkey-dictation.md 13.8).
pub fn save_provider(core: &Arc<Core>, save: Value) -> Result<(), ApiError> {
    let text = |name: &str| save[name].as_str().unwrap_or_default().trim().to_string();
    let target = text("target");
    let (purpose, allowed): (Purpose, &[&str]) = match target.as_str() {
        "transcription" => (Purpose::Dictation, &AUDIO_PROVIDER_IDS),
        "meetings" => (Purpose::Meetings, &AUDIO_PROVIDER_IDS),
        "rewriting" => (Purpose::Rewrite, &REWRITE_PROVIDER_IDS),
        _ => return Err(invalid("target is transcription, rewriting or meetings.")),
    };
    let provider = normalize_provider(&text("provider"), "");
    if !allowed.contains(&provider.as_str()) {
        return Err(invalid(format!("{provider:?} isn't offered here.")));
    }
    let local = provider == "orukeet";
    let (endpoint, model) = if local {
        (String::new(), ORUKEET_MODEL_ID.to_string())
    } else {
        (text("endpoint"), text("model"))
    };
    if let Some(problem) = provider_form_problem(purpose, &provider, &endpoint, &model) {
        return Err(ApiError::new("need_endpoint_model", problem));
    }
    // A realtime model only streams, and only meetings stream.
    if purpose != Purpose::Meetings && model.starts_with(REALTIME_MODEL_PREFIX) {
        return Err(invalid(
            "Realtime models work for meeting transcription only.",
        ));
    }
    let api_key = match &save["apiKey"] {
        Value::Null => None,
        Value::String(key) => Some(key.trim().to_string()),
        _ => return Err(invalid("apiKey is null or a string.")),
    };
    let language = match save.get("language") {
        Some(Value::String(language)) if purpose == Purpose::Dictation => {
            let language = language.trim().to_lowercase();
            if !LANGUAGES.contains(&language.as_str()) {
                return Err(invalid(format!(
                    "{language:?} isn't a language Voice offers."
                )));
            }
            Some(language)
        }
        _ => None,
    };
    let consent = save["consent"].as_bool().unwrap_or(false);

    // The form's fields only, so they go onto whatever Settings hold when it is saved: a change
    // saved meanwhile, for instance while Orukeet loads, stays.
    let apply = |cfg: &mut Config| {
        // A key is never carried to another provider: `null` keeps it only for the same one.
        let keep = |saved_provider: &str, saved_key: &str| match &api_key {
            None if saved_provider == provider => saved_key.to_string(),
            None => String::new(),
            Some(key) => key.clone(),
        };
        match purpose {
            Purpose::Dictation => {
                cfg.audio_api_key = if local {
                    String::new()
                } else {
                    keep(&cfg.audio_provider, &cfg.audio_api_key)
                };
                cfg.audio_provider = provider.clone();
                cfg.audio_endpoint = endpoint.clone();
                cfg.audio_model = model.clone();
                // Orukeet ignores the language; Ownkey saved it only for a cloud provider.
                if let Some(language) = language.clone().filter(|_| !local) {
                    cfg.language = language;
                }
            }
            Purpose::Rewrite => {
                cfg.rewrite_api_key = keep(&cfg.rewrite_provider, &cfg.rewrite_api_key);
                cfg.rewrite_provider = provider.clone();
                cfg.rewrite_endpoint = endpoint.clone();
                cfg.rewrite_model = model.clone();
            }
            Purpose::Meetings => {
                cfg.meetings_audio_api_key = if local {
                    String::new()
                } else {
                    keep(&cfg.meetings_audio_provider, &cfg.meetings_audio_api_key)
                };
                cfg.meetings_audio_provider = provider.clone();
                cfg.meetings_audio_endpoint = endpoint.clone();
                cfg.meetings_audio_model = model.clone();
                cfg.meetings_transcription_consent = None;
                if consent {
                    // Stored only where Ownkey's rule asks for it: a cloud provider on another host.
                    cfg.grant_meeting_consent();
                }
            }
        }
    };
    // A broken model never becomes active: Orukeet loads and is checked before the save. That can
    // take seconds, so it runs outside Settings' lock, with the model folder the form would save.
    if purpose == Purpose::Dictation && local {
        let mut next = (*core.config()).clone();
        apply(&mut next);
        core.svc
            .speech
            .local_load(core.registry(&next))
            .map_err(|e| provider_error(&e))?;
    }
    core.commit(|cfg| {
        apply(cfg);
        Ok(())
    })
}

pub fn remove_key(core: &Arc<Core>, target: &str) -> Result<(), ApiError> {
    if target == "sync" {
        return change_secret(core, |sync| {
            sync.remove_secret().map_err(|e| sync_error(&e))
        });
    }
    core.commit(|cfg| {
        match target {
            "transcription" => cfg.audio_api_key.clear(),
            "rewriting" => cfg.rewrite_api_key.clear(),
            "meetings" => cfg.meetings_audio_api_key.clear(),
            _ => {
                return Err(invalid(
                    "target is transcription, rewriting, meetings or sync.",
                ))
            }
        }
        Ok(())
    })
}

// Bucket sync (voice/ui/API.md, settings_sync_*) ------------------------------------------------

/// The longest bucket secret Voice keeps.
const SECRET_MAX_CHARS: usize = 1024;
/// The recovery code leaves the clipboard after this long, unless the user copied something else.
const KEY_COPY_CLEAR: Duration = Duration::from_secs(60);

fn bucket_sync(core: &Core) -> Result<&Arc<voice_sync::BucketSync>, ApiError> {
    core.sync.as_ref().ok_or_else(|| {
        ApiError::new(
            "failed",
            "Bucket sync isn't available: Voice couldn't open its sync store.",
        )
    })
}

fn sync_error(error: &voice_sync::SyncError) -> ApiError {
    crate::sync::api_error(error)
}

/// The key dialogs never open while a meeting records the screen, which could capture the code.
fn refuse_while_screen_records(core: &Core) -> Result<(), ApiError> {
    if core.screen_recording() {
        return Err(ApiError::new(
            "recording_screen",
            "Stop the meeting's screen recording before you show or enter a key.",
        ));
    }
    Ok(())
}

/// The recovery code and the key's id, making the key when there is none. Under the engine's key
/// lock, so the code shown is the key that stays.
fn show_key(
    core: &Core,
    sync: &voice_sync::BucketSync,
) -> Result<(voice_sync::Zeroizing<String>, String), ApiError> {
    let shown = {
        let _key = lock(&core.sync_rt.key);
        sync.show_key()
    };
    let (code, id) = shown.map_err(|e| sync_error(&e))?;
    Ok((voice_sync::Zeroizing::new(code), id))
}

/// Test connection for the fields in Settings with the saved secret. A pass stores the URL style
/// that worked and the fields' fingerprint in one commit, which lets the switch turn on.
pub fn sync_test(core: &Arc<Core>) -> Result<Value, ApiError> {
    let sync = bucket_sync(core)?;
    let cfg = core.config();
    let fields = crate::sync::sync_value(&cfg);
    let bucket = voice_sync::SyncConfig::from_json(&fields).map_err(|e| sync_error(&e))?;
    let report = sync.test(&bucket).map_err(|e| sync_error(&e))?;
    let tested = voice_sync::SyncConfig {
        url_style: report.url_style,
        ..bucket.clone()
    };
    // A change of the secret clears the pass before it replaces the secret, under this lock: the
    // pass of a test that read the old secret never lands after that clear.
    let _secret = lock(&core.sync_rt.secret);
    core.commit(|cfg| {
        // The fields or the secret may have changed while the test ran: then it tested something
        // else.
        let mut now = crate::sync::sync_value(cfg);
        let same = voice_sync::SyncConfig::from_json(&now).is_ok_and(|current| {
            voice_sync::SyncConfig {
                url_style: report.url_style,
                ..current
            } == tested
        }) && sync.secret_generation() == report.secret_generation;
        if !same {
            return Err(ApiError::new(
                "failed",
                "The settings changed during the test. Test again.",
            ));
        }
        now["urlStyle"] = json!(report.url_style.as_str());
        now["tested"] = json!(tested.fingerprint());
        cfg.extra.insert(crate::sync::SYNC_KEY.into(), now);
        Ok(())
    })?;
    Ok(json!({ "otherKeys": report.other_keys }))
}

/// Keeps the bucket's secret with DPAPI. Uploads pause until Test connection passes with it; the
/// switch stays on, so retention keeps holding what hasn't reached the bucket.
pub fn sync_save_secret(core: &Arc<Core>, secret: String) -> Result<(), ApiError> {
    let secret = voice_sync::Zeroizing::new(secret);
    if secret.trim().is_empty() || secret.chars().count() > SECRET_MAX_CHARS {
        return Err(invalid(format!(
            "The secret is 1 to {SECRET_MAX_CHARS} characters."
        )));
    }
    change_secret(core, |sync| {
        sync.save_secret(secret.trim()).map_err(|e| {
            ApiError::new(
                "failed",
                format!("Windows couldn't protect the secret: {}", e.message),
            )
        })
    })
}

/// Saves or removes the bucket secret (spec D6: a test counts only for the current fields and
/// secret). The passed Test connection leaves `config.json` first, which pauses uploads, and only
/// then does the secret change. When `config.json` can't be saved, the tested secret stays and the
/// error comes back; a failed `change` leaves the test cleared for a secret that stayed.
fn change_secret(
    core: &Arc<Core>,
    change: impl FnOnce(&voice_sync::BucketSync) -> Result<(), ApiError>,
) -> Result<(), ApiError> {
    let sync = bucket_sync(core)?;
    let result = {
        let _secret = lock(&core.sync_rt.secret);
        core.commit(|cfg| {
            keys::untest_sync(cfg);
            Ok(())
        })?;
        change(sync)
    };
    // The secret changed even when nothing in config.json did.
    core.settings_changed();
    result
}

/// `show` gives the recovery code (making the key when there is none), `saved` records that the
/// user kept it, `import` switches to a key from another computer.
pub fn sync_key(core: &Arc<Core>, action: &str, code: Option<String>) -> Result<Value, ApiError> {
    let code = code.map(voice_sync::Zeroizing::new);
    let result = match action {
        "show" => {
            refuse_while_screen_records(core)?;
            let sync = bucket_sync(core)?;
            let (code, id) = show_key(core, sync)?;
            json!({ "code": code.as_str(), "id": crate::sync::short_id(&id) })
        }
        "saved" => {
            bucket_sync(core)?
                .mark_key_saved()
                .map_err(|e| sync_error(&e))?;
            Value::Null
        }
        "import" => {
            refuse_while_screen_records(core)?;
            let code = code
                .as_deref()
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
                .ok_or_else(|| invalid("Paste the key."))?;
            if voice_sync::recovery::decode(code).is_err() {
                return Err(invalid("That isn't a Rukoo Voice key."));
            }
            let sync = bucket_sync(core)?;
            let imported = {
                let _key = lock(&core.sync_rt.key);
                sync.import_key(code)
            };
            let id = imported.map_err(|e| match e.kind {
                voice_sync::SyncErrorKind::KeyInUse => ApiError::new("key_in_use", e.message),
                voice_sync::SyncErrorKind::Crypto => invalid(e.message),
                _ => sync_error(&e),
            })?;
            json!({ "id": crate::sync::short_id(&id) })
        }
        _ => return Err(invalid("action is show, saved or import.")),
    };
    core.settings_changed();
    Ok(result)
}

/// Puts the recovery code on the clipboard hidden from clipboard history and cloud clipboard, and
/// empties the clipboard after 60 s while it still holds the code.
pub fn sync_key_copy(core: &Arc<Core>) -> Result<(), ApiError> {
    let sync = bucket_sync(core)?;
    let (code, _) = show_key(core, sync)?;
    core.svc
        .input
        .copy_hidden(&code)
        .map_err(|e| ApiError::new("failed", format!("Voice couldn't copy the key: {e}")))?;
    let input = Arc::clone(&core.svc.input);
    let _ = thread::Builder::new()
        .name("key-clipboard".into())
        .spawn(move || {
            thread::sleep(KEY_COPY_CLEAR);
            let _ = input.clear_if(&code);
        });
    Ok(())
}

/// `list` the meetings in the bucket that aren't on this computer, or `start` restoring them.
pub fn sync_restore(core: &Arc<Core>, action: &str) -> Result<Value, ApiError> {
    let sync = bucket_sync(core)?;
    match action {
        "list" => {
            let meetings = sync.remote_meetings().map_err(|e| sync_error(&e))?;
            let meetings: Vec<Value> = meetings
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "title": m.title,
                        "startedAt": m.started_at,
                        "duration": m.duration_ms,
                    })
                })
                .collect();
            core.settings_changed();
            Ok(json!({ "meetings": meetings }))
        }
        "start" => {
            let ids: Vec<String> = sync
                .remote_meetings()
                .map_err(|e| sync_error(&e))?
                .into_iter()
                .map(|m| m.id)
                .collect();
            let weak = Arc::downgrade(core);
            sync.restore(
                ids,
                Box::new(move |id: &str| {
                    // Its pipeline sends the meeting to Rukoo: the new bridge.db has none of it.
                    if let Some(core) = weak.upgrade() {
                        core.follow(id);
                        core.settings.forget_storage();
                        core.settings_changed();
                    }
                }),
            )
            .map_err(|e| sync_error(&e))?;
            core.settings_changed();
            Ok(Value::Null)
        }
        _ => Err(invalid("action is list or start.")),
    }
}

pub fn list_models(core: &Arc<Core>, request: Value) -> Result<Vec<String>, ApiError> {
    let text = |name: &str| {
        request[name]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let target = text("target");
    let cfg = core.config();
    let (activity, purpose, saved_provider, saved_key) = match target.as_str() {
        "transcription" => (
            Activity::Audio,
            Purpose::Dictation,
            &cfg.audio_provider,
            &cfg.audio_api_key,
        ),
        "meetings" => (
            Activity::Audio,
            Purpose::Meetings,
            &cfg.meetings_audio_provider,
            &cfg.meetings_audio_api_key,
        ),
        "rewriting" => (
            Activity::Rewrite,
            Purpose::Rewrite,
            &cfg.rewrite_provider,
            &cfg.rewrite_api_key,
        ),
        _ => return Err(invalid("target is transcription, rewriting or meetings.")),
    };
    let provider = normalize_provider(&text("provider"), "");
    if provider == "orukeet" {
        return Ok(vec![ORUKEET_MODEL_ID.to_string()]);
    }
    let endpoint = text("endpoint");
    if endpoint.is_empty() {
        return Err(ApiError::new("config", "Choose an endpoint first."));
    }
    let api_key = match &request["apiKey"] {
        Value::String(key) => key.trim().to_string(),
        _ if *saved_provider == provider => saved_key.clone(),
        _ => String::new(),
    };
    let provider_cfg = ProviderConfig {
        provider,
        api_key,
        endpoint,
        model: String::new(),
        timeout: MODELS_TIMEOUT,
        purpose,
    };
    let mut models = core
        .svc
        .speech
        .list_models(activity, &provider_cfg)
        .map_err(|e| provider_error(&e))?;
    models.sort_by_key(|m| m.to_lowercase());
    models.dedup();
    Ok(models)
}

pub fn model(core: &Arc<Core>, id: &str, action: &str) -> Result<(), ApiError> {
    let model =
        Model::parse(id).ok_or_else(|| ApiError::new("not_found", format!("No model {id:?}.")))?;
    match action {
        "download" => download(core, model),
        "cancel" => {
            if let Some(download) = lock(&core.settings.downloads).get(&model) {
                download.cancel.store(true, Ordering::SeqCst);
            }
            Ok(())
        }
        "remove" => remove(core, model),
        _ => Err(invalid("action is download, cancel or remove.")),
    }
}

fn download(core: &Arc<Core>, model: Model) -> Result<(), ApiError> {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = lock(&core.settings.downloads);
        let entry = downloads.entry(model).or_default();
        if entry.running {
            return Ok(());
        }
        *entry = Download {
            stage: Some("downloading"),
            percent: Some(0),
            error: None,
            cancel: Arc::clone(&cancel),
            running: true,
        };
    }
    core.settings_changed();
    let core = Arc::clone(core);
    thread::Builder::new()
        .name("model-download".into())
        .spawn(move || {
            let cfg = core.config();
            let registry = core.registry(&cfg);
            let spec = model.spec();
            let mut progress = |p: local::Progress| {
                let stage = match p.stage {
                    Stage::Downloading => "downloading",
                    Stage::Verifying => "verifying",
                    Stage::Extracting => "extracting",
                    Stage::Installed => "installed",
                };
                let percent = p
                    .total
                    .filter(|t| *t > 0)
                    .map(|t| (p.done.min(t) * 100) / t);
                let changed = {
                    let mut downloads = lock(&core.settings.downloads);
                    let entry = downloads.entry(model).or_default();
                    let changed = entry.stage != Some(stage);
                    entry.stage = Some(stage);
                    entry.percent = percent;
                    changed
                };
                let mut last = lock(&core.settings.last_progress);
                if changed || last.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL) {
                    *last = Some(Instant::now());
                    drop(last);
                    core.settings_changed();
                }
            };
            let result = core
                .svc
                .speech
                .download(&registry, &spec, &mut progress, &cancel);
            {
                let mut downloads = lock(&core.settings.downloads);
                let entry = downloads.entry(model).or_default();
                entry.running = false;
                entry.stage = None;
                entry.percent = None;
                entry.error = match result {
                    Ok(_) => None,
                    Err(e) if e.kind == ModelErrorKind::Cancelled => None,
                    Err(e) => Some(e.message),
                };
            }
            let cfg = core.config();
            core.svc
                .speech
                .local_configure(core.registry(&cfg), cfg.idle_timeout());
            core.settings_changed();
        })
        .map_err(|e| ApiError::new("failed", e.to_string()))?;
    Ok(())
}

fn remove(core: &Arc<Core>, model: Model) -> Result<(), ApiError> {
    let cfg = core.config();
    let registry = core.registry(&cfg);
    let spec = model.spec();
    let Some(located) = registry.locate(&spec) else {
        return Err(ApiError::new(
            "not_removable",
            "There is no download to remove.",
        ));
    };
    if !located.removable() {
        // Decision 13: Voice never removes anything in Ownkey's folder or a folder the user chose.
        let message = match located.location {
            Location::Ownkey => "This model belongs to Ownkey. Remove it in Ownkey.",
            _ => "This model is in the folder you chose. Voice doesn't remove it.",
        };
        return Err(ApiError::new("not_removable", message));
    }
    if model == Model::Nemotron && labelling_with_nemotron(&core.meetings.labels()) {
        return Err(ApiError::new(
            "busy",
            "A meeting is labelling speakers with this model.",
        ));
    }
    if model == Model::Orukeet {
        core.svc.speech.local_unload();
    }
    registry
        .remove(&spec)
        .map_err(|e| ApiError::new("failed", e.message))?;
    core.settings_changed();
    Ok(())
}

pub fn set_folder(core: &Arc<Core>, purpose: &str, path: PathBuf) -> Result<(), ApiError> {
    match purpose {
        "models" => core.commit(|cfg| {
            cfg.local_model_directory = path.display().to_string();
            Ok(())
        }),
        // Meetings live in the data folder (voice/RECORDING.md); voice-meetings opens them there.
        "recordings" => Err(invalid(
            "Meetings are kept in Voice's data folder in this version.",
        )),
        _ => Err(invalid("purpose is models or recordings.")),
    }
}

pub fn clean_preview(core: &Arc<Core>, preview: Value) -> Result<String, ApiError> {
    let text = preview["text"]
        .as_str()
        .ok_or_else(|| invalid("text is a string."))?;
    let languages = keys::filler_languages(&preview["languages"])?;
    let cfg = core.config();
    Ok(cleanup::clean(
        text,
        &CleanupConfig {
            remove_fillers: preview["enabled"].as_bool().unwrap_or(true),
            filler_languages: languages,
            custom_fillers: preview["custom"].as_str().unwrap_or_default().to_string(),
            corrections: cfg.corrections.clone(),
        },
    ))
}

/// Checks now, also when the daily check is off, because the user asked.
pub fn check_updates(core: &Arc<Core>) -> Value {
    *lock(&core.settings.updates) = ("checking".into(), None);
    core.settings_changed();
    let result = core.svc.updates.check(&core.opts.version);
    let next = match result {
        Ok(Some(version)) => ("available".to_string(), Some(version)),
        Ok(None) => ("upToDate".to_string(), None),
        Err(_) => ("failed".to_string(), None),
    };
    *lock(&core.settings.updates) = next.clone();
    core.settings_changed();
    json!({ "state": next.0, "version": next.1 })
}

/// How often the scheduler asks whether the daily check is due.
const UPDATE_SCHEDULE: Duration = Duration::from_secs(60);

/// The opt-in daily check: nothing touches the network unless `updates.check` is on. The last
/// check is kept in memory, so a new start checks once and then daily.
pub fn schedule_updates(core: std::sync::Weak<Core>) {
    let mut last: Option<u64> = None;
    loop {
        let deadline = Instant::now() + UPDATE_SCHEDULE;
        while Instant::now() < deadline {
            thread::sleep(Duration::from_millis(250));
            match core.upgrade() {
                Some(core) if !core.shutting_down() => {}
                _ => return,
            }
        }
        let Some(core) = core.upgrade() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        if voice_core::update::is_due(&core.config(), last, now) {
            last = Some(now);
            check_updates(&core);
        }
    }
}

pub fn new_token(core: &Arc<Core>) -> Result<(), ApiError> {
    let bridge = core
        .meetings
        .bridge
        .as_ref()
        .ok_or_else(|| ApiError::new("failed", "The bridge for Rukoo isn't running."))?;
    bridge
        .new_token()
        .map_err(|e| ApiError::new("failed", format!("The bridge couldn't restart: {e}")))?;
    core.settings_changed();
    Ok(())
}

pub fn import_ownkey(core: &Arc<Core>) -> Result<Value, ApiError> {
    if core.opts.dev_mode {
        return Err(ApiError::new(
            "dev_mode",
            "A development copy imports Ownkey's settings only with --import-ownkey <copy>.",
        ));
    }
    let path = core
        .svc
        .ownkey_config
        .clone()
        .ok_or_else(|| ApiError::new("not_found", "Ownkey's settings weren't found."))?;
    ownkey::import_into(core, &path)
}

pub fn folder_path(core: &Core, folder: Folder) -> Option<PathBuf> {
    let data = &core.opts.data_dir;
    Some(match folder {
        Folder::Logs => voice_core::paths::logs_dir(data),
        Folder::Recordings => core.meetings.library().root().to_path_buf(),
        Folder::Bridge => core
            .meetings
            .bridge
            .as_ref()?
            .discovery_path()
            .parent()?
            .to_path_buf(),
        Folder::Models => {
            let cfg = core.config();
            let chosen = cfg.local_model_directory.trim();
            if chosen.is_empty() {
                voice_core::paths::models_dir(data)
            } else {
                PathBuf::from(chosen)
            }
        }
    })
}

/// The consent object for the page. `mode` says whether the agreement covers recorded windows
/// or a stream; one doesn't cover the other.
pub fn consent_view(consent: Option<&Consent>) -> Value {
    consent.map_or(
        Value::Null,
        |c| json!({"provider": c.provider, "host": c.host, "mode": c.mode}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(mode: &str, state: LabelState) -> (String, LabelStatus) {
        (
            "m-20261102-101500-1a2b".into(),
            LabelStatus {
                mode: mode.into(),
                state,
                reason: None,
                epoch: 1,
                speakers: 2,
                settled_ms: Some(1000),
                worker_pid: None,
            },
        )
    }

    #[test]
    fn nemotron_can_be_removed_only_while_no_meeting_labels_with_it() {
        assert!(!labelling_with_nemotron(&[]));
        for state in [
            LabelState::Starting,
            LabelState::Matching,
            LabelState::Listening,
            LabelState::CatchingUp,
            LabelState::Restarting,
        ] {
            assert!(
                labelling_with_nemotron(&[status("nemotron", state)]),
                "{state:?}"
            );
        }
        for state in [LabelState::Finished, LabelState::Failed, LabelState::Off] {
            assert!(
                !labelling_with_nemotron(&[status("nemotron", state)]),
                "{state:?}"
            );
        }
        // Mistral's stream doesn't use the model.
        assert!(!labelling_with_nemotron(&[status(
            "mistral",
            LabelState::Listening
        )]));
        assert!(labelling_with_nemotron(&[
            status("nemotron", LabelState::Finished),
            status("nemotron", LabelState::Restarting),
        ]));
    }

    #[test]
    fn consent_shows_its_mode() {
        let consent = Consent {
            provider: "mistral".into(),
            host: "api.mistral.ai".into(),
            mode: voice_core::config::ConsentMode::Stream,
        };
        assert_eq!(
            consent_view(Some(&consent)),
            json!({"provider": "mistral", "host": "api.mistral.ai", "mode": "stream"})
        );
        assert_eq!(consent_view(None), Value::Null);
    }
}
