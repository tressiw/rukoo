//! The read-only Ownkey import, always against a synthetic config the test writes itself.

mod support;

use std::path::Path;

use serde_json::{json, Value};
use support::TempDir;
use voice_core::cleanup::Correction;
use voice_core::config::{Config, Consent, ConsentMode, Retention, RewriteTone, Theme};
use voice_core::ownkey::{import, ImportError};

const AUDIO_KEY: &str = "sk-audio-0123456789";
const REWRITE_KEY: &str = "sk-rewrite-9876543210";
const MEETING_KEY: &str = "sk-meeting-5555";

fn write_ownkey(dir: &TempDir, value: &Value) -> std::path::PathBuf {
    let path = dir.join("config.json");
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn full_ownkey_config() -> Value {
    json!({
        "audio_provider": "openai",
        "audio_api_key": AUDIO_KEY,
        "audio_endpoint": "https://api.openai.com/v1/audio/transcriptions",
        "audio_model": "whisper-1",
        "hotkey": "right alt",
        "rewrite_hotkey": "f15",
        "language": "nl",
        "paste_mode": false,
        "ready_chime": true,
        "sample_rate": 48000,
        "local_model_idle_timeout_minutes": 5,
        "local_model_directory": "",
        "rewrite_provider": "anthropic",
        "rewrite_api_key": REWRITE_KEY,
        "rewrite_endpoint": "https://api.anthropic.com/v1/messages",
        "rewrite_model": "claude-test",
        "auto_rewrite": true,
        "rewrite_tone": "professional",
        "rewrite_formatting": false,
        "rewrite_custom_instructions": "Keep greetings.",
        "vocabulary": ["Rukoo", "Bart"],
        "corrections": [{"from": "own key", "to": "Ownkey"}],
        "remove_fillers": true,
        "filler_languages": ["en", "de"],
        "custom_fillers": "basically",
        "meetings_remote_policy": "allow",
        "meetings_transcription_policy": "allow",
        "meetings_live_transcription_policy": "ask",
        "meetings_auto_summary": true,
        "meetings_auto_speakers": true,
        "meetings_retention": "keep",
        "meetings_audio_provider": "mistral",
        "meetings_audio_api_key": MEETING_KEY,
        "meetings_audio_endpoint": "https://api.mistral.ai/v1/audio/transcriptions",
        "meetings_audio_model": "voxtral-mini-latest",
        "pyannote_api_key": "old-pyannote-key"
    })
}

fn modified(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn import_reads_without_writing_and_copies_the_settings() {
    let dir = TempDir::new("ownkey-full");
    let path = write_ownkey(&dir, &full_ownkey_config());
    // A read-only file proves the import never needs to write it.
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    let before = (std::fs::read(&path).unwrap(), modified(&path));

    let imported = import(&path).unwrap();
    let mut cfg = Config::defaults(false);
    cfg.ui_theme = Theme::Dark;
    cfg.updates_check = true;
    let report = imported.apply(&mut cfg, false);

    assert_eq!((std::fs::read(&path).unwrap(), modified(&path)), before);
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        names,
        vec!["config.json"],
        "the import wrote nothing next to Ownkey's file"
    );

    assert_eq!(cfg.hotkey, "right alt");
    assert_eq!(cfg.rewrite_hotkey, "f15");
    assert!(!cfg.paste_mode);
    assert!(cfg.ready_chime);
    assert_eq!(cfg.audio_provider, "openai");
    assert_eq!(cfg.audio_api_key, AUDIO_KEY);
    assert_eq!(cfg.audio_model, "whisper-1");
    assert_eq!(cfg.language, "nl");
    assert_eq!(cfg.local_model_idle_timeout_minutes, 5.0);
    assert_eq!(cfg.rewrite_provider, "anthropic");
    assert_eq!(cfg.rewrite_api_key, REWRITE_KEY);
    assert!(cfg.auto_rewrite);
    assert_eq!(cfg.rewrite_tone, RewriteTone::Professional);
    assert!(!cfg.rewrite_formatting);
    assert_eq!(cfg.rewrite_custom_instructions, "Keep greetings.");
    assert_eq!(cfg.vocabulary, vec!["Rukoo", "Bart"]);
    assert_eq!(cfg.corrections, vec![Correction::new("own key", "Ownkey")]);
    assert_eq!(cfg.filler_languages, vec!["en", "de"]);
    assert_eq!(cfg.custom_fillers, "basically");
    assert_eq!(cfg.meetings_audio_provider, "mistral");
    assert_eq!(cfg.meetings_audio_api_key, MEETING_KEY);
    assert_eq!(cfg.meetings_retention, Retention::Keep);
    assert!(cfg.meetings_auto_speakers);
    // "ask" for live transcription is not consent.
    assert_eq!(cfg.meetings_transcription_consent, None);
    // Voice-only settings stay.
    assert_eq!(cfg.ui_theme, Theme::Dark);
    assert!(cfg.updates_check);
    assert!(cfg.ownkey_imported_at.is_some());
    // Ownkey-only and retired keys don't come along.
    for key in [
        "sample_rate",
        "meetings_remote_policy",
        "meetings_auto_summary",
        "pyannote_api_key",
    ] {
        assert!(!cfg.extra.contains_key(key), "{key}");
    }

    let report_text = serde_json::to_string(&report).unwrap();
    for secret in [AUDIO_KEY, REWRITE_KEY, MEETING_KEY, "old-pyannote-key"] {
        assert!(!report_text.contains(secret), "the report shows a key");
    }
    assert!(!format!("{imported:?}").contains(AUDIO_KEY));
    assert_eq!(
        report.item("audio_key").unwrap().text,
        "OpenAI key imported"
    );
    assert_eq!(
        report.item("rewrite_key").unwrap().text,
        "Anthropic key imported"
    );
    assert_eq!(
        report.item("meetings_key").unwrap().text,
        "Mistral key for meetings imported"
    );
    assert_eq!(
        report.item("dictionary").unwrap().text,
        "2 words, 1 correction"
    );
    assert!(report.item("hotkeys").unwrap().imported);
    let consent = report.item("consent").unwrap();
    assert!(!consent.imported);
    assert_eq!(
        consent.text,
        "Sending meeting audio to Mistral: Voice asks in Settings first"
    );
    assert!(!report.item("summaries").unwrap().imported);
    assert_eq!(
        report.item("sample_rate").unwrap().text,
        "Sample rate 48000 Hz: Voice always records at 16 kHz"
    );
    assert_eq!(report.source, path.display().to_string());

    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).unwrap();
}

#[test]
fn development_mode_never_imports_hotkeys() {
    let dir = TempDir::new("ownkey-dev");
    let path = write_ownkey(&dir, &full_ownkey_config());
    let mut cfg = Config::defaults(true);
    let (hotkey, rewrite_hotkey) = (cfg.hotkey.clone(), cfg.rewrite_hotkey.clone());
    let report = import(&path).unwrap().apply(&mut cfg, true);
    assert_eq!(cfg.hotkey, hotkey);
    assert_eq!(cfg.rewrite_hotkey, rewrite_hotkey);
    let hotkeys = report.item("hotkeys").unwrap();
    assert!(!hotkeys.imported);
    assert_eq!(
        hotkeys.text,
        "Keys: in development, Voice keeps F13 and F14."
    );
    // Everything else still comes along.
    assert_eq!(cfg.audio_provider, "openai");
}

#[test]
fn live_transcription_allow_carries_consent_over() {
    // Ownkey's default: meetings follow the dictation provider, here Mistral in the cloud.
    let dir = TempDir::new("ownkey-consent");
    let path = write_ownkey(
        &dir,
        &json!({"audio_provider": "mistral", "audio_api_key": "k", "meetings_live_transcription_policy": "allow"}),
    );
    let mut cfg = Config::defaults(false);
    let report = import(&path).unwrap().apply(&mut cfg, false);
    // Voice has no "same as dictation": the import resolves it to the provider it followed.
    assert_eq!(cfg.meetings_audio_provider, "mistral");
    assert_eq!(cfg.meetings_audio_api_key, "k");
    assert_eq!(
        cfg.meetings_audio_endpoint,
        "https://api.mistral.ai/v1/audio/transcriptions"
    );
    assert_eq!(cfg.meetings_audio_model, cfg.audio_model);
    assert_eq!(
        report.item("meetings").unwrap().text,
        "Meeting transcription Mistral, keep audio 7 days after transcription"
    );
    assert_eq!(
        cfg.meetings_transcription_consent,
        Some(Consent {
            provider: "mistral".into(),
            host: "api.mistral.ai".into(),
            mode: ConsentMode::Windows,
        })
    );
    let consent = report.item("consent").unwrap();
    assert!(consent.imported);
    assert_eq!(
        consent.text,
        "Meeting audio may go to Mistral at api.mistral.ai, as Ownkey allowed"
    );
}

#[test]
fn ownkeys_answer_never_covers_a_realtime_model() {
    // Ownkey sent recorded windows; a realtime meeting model streams, so Settings asks.
    let dir = TempDir::new("ownkey-consent-stream");
    let path = write_ownkey(
        &dir,
        &json!({
            "meetings_audio_provider": "mistral",
            "meetings_audio_api_key": MEETING_KEY,
            "meetings_audio_endpoint": "https://api.mistral.ai/v1/audio/transcriptions",
            "meetings_audio_model": "voxtral-mini-transcribe-realtime-2602",
            "meetings_live_transcription_policy": "allow"
        }),
    );
    let mut cfg = Config::defaults(false);
    let report = import(&path).unwrap().apply(&mut cfg, false);
    assert_eq!(
        cfg.meetings_audio_model,
        "voxtral-mini-transcribe-realtime-2602"
    );
    assert_eq!(cfg.meetings_transcription_consent, None);
    let consent = report.item("consent").unwrap();
    assert!(!consent.imported);
    assert_eq!(
        consent.text,
        "Sending meeting audio to Mistral: Voice asks in Settings first"
    );
}

#[test]
fn consent_is_never_invented() {
    // Only the live transcription answer counts, and only for a cloud provider.
    for value in [
        json!({"audio_provider": "mistral", "meetings_transcription_policy": "allow"}),
        json!({"audio_provider": "mistral", "meetings_live_transcription_policy": "Allow"}),
        json!({"audio_provider": "orukeet", "meetings_live_transcription_policy": "allow"}),
        json!({
            "audio_provider": "mistral",
            "meetings_audio_provider": "custom",
            "meetings_audio_endpoint": "http://localhost:1234/v1/audio/transcriptions",
            "meetings_audio_model": "whisper",
            "meetings_live_transcription_policy": "allow"
        }),
    ] {
        let dir = TempDir::new("ownkey-no-consent");
        let path = write_ownkey(&dir, &value);
        let mut cfg = Config::defaults(false);
        import(&path).unwrap().apply(&mut cfg, false);
        assert_eq!(cfg.meetings_transcription_consent, None, "{value}");
    }
}

#[test]
fn old_ownkey_configs_are_migrated_on_import() {
    let dir = TempDir::new("ownkey-legacy");
    let path = write_ownkey(
        &dir,
        &json!({
            "api_key": "legacy-key",
            "endpoint": "https://api.mistral.ai/v1/audio/transcriptions",
            "model": "voxtral-old",
            "chat_endpoint": "https://api.mistral.ai/v1/chat/completions",
            "rewrite_model": "mistral-old",
            "rewrite_provider": "local_rewrite",
            "auto_rewrite": true
        }),
    );
    let mut cfg = Config::defaults(false);
    import(&path).unwrap().apply(&mut cfg, false);
    assert_eq!(cfg.audio_api_key, "legacy-key");
    assert_eq!(cfg.audio_model, "voxtral-old");
    // Retired local rewriting never becomes a cloud rewrite.
    assert_eq!(cfg.rewrite_provider, "mistral");
    assert_eq!(cfg.rewrite_api_key, "");
    assert!(!cfg.auto_rewrite);
    assert_eq!(cfg.rewrite_hotkey, "off");
}

#[test]
fn missing_and_broken_files_are_reported() {
    let dir = TempDir::new("ownkey-errors");
    let missing = dir.join("nope.json");
    assert!(matches!(import(&missing), Err(ImportError::NotFound(path)) if path == missing));
    let broken = dir.join("broken.json");
    std::fs::write(&broken, b"{ not json").unwrap();
    assert!(matches!(import(&broken), Err(ImportError::Invalid(_))));
    let list = dir.join("list.json");
    std::fs::write(&list, b"[1, 2]").unwrap();
    assert!(matches!(import(&list), Err(ImportError::Invalid(_))));
}
