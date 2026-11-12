//! Config defaults, Ownkey's load-time migrations, Voice's additions, atomic save and round trip.

mod support;

use std::time::Duration;

use serde_json::{json, Value};
use support::TempDir;
use voice_core::cleanup::Correction;
use voice_core::config::{
    provider_form_problem, Config, ConfigError, Consent, ConsentMode, PillEdge, Retention,
    RewriteTone, ScreenMode, Theme, UiLanguage,
};
use voice_core::consent::ConsentState;
use voice_core::providers::Purpose;

fn load(value: Value) -> Config {
    Config::from_value(&value, false)
}

#[test]
fn installed_defaults() {
    let cfg = Config::defaults(false);
    let (hotkey, rewrite_hotkey) =
        voice_core::config::default_hotkeys_on(false, cfg!(target_os = "macos"));
    assert_eq!(cfg.version, 1);
    assert_eq!(cfg.hotkey, hotkey);
    assert_eq!(cfg.rewrite_hotkey, rewrite_hotkey);
    assert!(cfg.paste_mode);
    assert!(!cfg.ready_chime);
    assert_eq!(cfg.audio_provider, "mistral");
    assert_eq!(
        cfg.audio_endpoint,
        "https://api.mistral.ai/v1/audio/transcriptions"
    );
    assert_eq!(cfg.audio_model, "voxtral-mini-latest");
    assert_eq!(cfg.language, "auto");
    assert_eq!(cfg.local_model_idle_timeout_minutes, 20.0);
    assert_eq!(cfg.rewrite_provider, "mistral");
    assert_eq!(
        cfg.rewrite_endpoint,
        "https://api.mistral.ai/v1/chat/completions"
    );
    assert_eq!(cfg.rewrite_model, "mistral-small-latest");
    assert!(!cfg.auto_rewrite);
    assert_eq!(cfg.rewrite_tone, RewriteTone::Auto);
    assert!(cfg.rewrite_formatting);
    assert!(cfg.remove_fillers);
    assert_eq!(cfg.filler_languages, vec!["en", "nl"]);
    // Decision 10: meetings transcribe on this computer by default.
    assert_eq!(cfg.meetings_audio_provider, "orukeet");
    assert_eq!(cfg.meetings_audio_model, "orukeet-onnx-int8");
    assert_eq!(cfg.meetings_transcription_consent, None);
    assert_eq!(cfg.meetings_retention, Retention::Days7);
    assert_eq!(cfg.meetings_screen, ScreenMode::Off);
    assert_eq!(cfg.meetings_stop_key, "off");
    assert_eq!(cfg.ui_language, UiLanguage::System);
    assert_eq!(cfg.ui_theme, Theme::System);
    assert_eq!(cfg.pill_edge, PillEdge::Bottom);
    assert!(cfg.pill_hide_from_capture);
    assert!(!cfg.updates_check, "the update check is opt-in");
    assert_eq!(cfg.ownkey_imported_at, None);
    assert_eq!(cfg.meeting_consent(), ConsentState::NotNeeded);
}

#[test]
fn development_defaults_never_use_ownkeys_keys() {
    let cfg = Config::defaults(true);
    assert_eq!(cfg.hotkey, "f13");
    assert_eq!(cfg.rewrite_hotkey, "f14");
    // Missing or unknown keys fall back to the development keys too.
    let cfg = Config::from_value(&json!({"hotkey": "nonsense"}), true);
    assert_eq!(
        (cfg.hotkey.as_str(), cfg.rewrite_hotkey.as_str()),
        ("f13", "f14")
    );
}

#[test]
fn defaults_follow_the_platform_and_a_development_copy() {
    use voice_core::config::default_hotkeys_on;
    // Windows keeps Ownkey's Right Alt and Right Ctrl.
    assert_eq!(
        default_hotkeys_on(false, false),
        ("right alt", "right ctrl")
    );
    // A Mac starts on Right Option and Right Command: most Mac laptops have no Right Control key, and
    // Fn/Globe already opens the emoji picker and macOS dictation.
    assert_eq!(
        default_hotkeys_on(false, true),
        ("right alt", "right command")
    );
    // Development copies use F13 and F14 on either platform.
    assert_eq!(default_hotkeys_on(true, false), ("f13", "f14"));
    assert_eq!(default_hotkeys_on(true, true), ("f13", "f14"));
    // Right Command is the one key a Mac adds; the rest stay Ownkey's names.
    assert!(voice_core::config::HOTKEYS.contains(&"right command"));
    assert!(voice_core::config::MACOS_HOTKEYS.contains(&"right command"));
    assert!(!voice_core::config::WINDOWS_HOTKEYS.contains(&"right command"));
    assert!(!voice_core::config::MACOS_HOTKEYS.contains(&"pause"));
    assert!(!voice_core::config::MACOS_HOTKEYS.contains(&"scroll lock"));
}

#[test]
fn missing_file_gives_defaults_and_first_run() {
    let dir = TempDir::new("config-missing");
    assert!(Config::is_first_run(dir.path()));
    assert_eq!(
        Config::load_as(dir.path(), true).unwrap(),
        Config::defaults(true)
    );
}

#[test]
fn save_and_load_round_trip() {
    let dir = TempDir::new("config-round-trip");
    let mut cfg = Config::defaults(false);
    cfg.hotkey = "f15".into();
    cfg.rewrite_hotkey = "off".into();
    cfg.audio_provider = "openai".into();
    cfg.audio_api_key = "sk-test".into();
    cfg.audio_endpoint = "https://api.openai.com/v1/audio/transcriptions".into();
    cfg.audio_model = "whisper-1".into();
    cfg.language = "nl".into();
    cfg.vocabulary = vec!["Rukoo".into(), "Bart van der Meeren".into()];
    cfg.corrections = vec![Correction::new("own key", "Ownkey")];
    cfg.filler_languages = vec![];
    cfg.custom_fillers = "basically, actually".into();
    cfg.rewrite_tone = RewriteTone::Friendly;
    cfg.meetings_audio_provider = "mistral".into();
    cfg.meetings_audio_endpoint = "https://api.mistral.ai/v1/audio/transcriptions".into();
    cfg.meetings_audio_model = "voxtral-mini-latest".into();
    cfg.meetings_transcription_consent = Some(Consent {
        provider: "mistral".into(),
        host: "api.mistral.ai".into(),
        mode: ConsentMode::Windows,
    });
    cfg.meetings_retention = Retention::AfterTranscription;
    cfg.meetings_screen = ScreenMode::Frames;
    cfg.meetings_stop_key = "pause".into();
    cfg.ui_language = UiLanguage::Nl;
    cfg.ui_theme = Theme::Dark;
    cfg.pill_edge = PillEdge::Top;
    cfg.updates_check = true;
    cfg.local_model_idle_timeout_minutes = 0.0;
    cfg.ownkey_imported_at = Some(1_761_800_000_000);
    cfg.extra
        .insert("from_a_newer_voice".into(), json!({"kept": true}));
    cfg.save(dir.path()).unwrap();

    assert!(!Config::is_first_run(dir.path()));
    let loaded = Config::load_as(dir.path(), false).unwrap();
    assert_eq!(loaded, cfg);
    // Nothing but config.json is left behind by the atomic write.
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["config.json"]);
    let on_disk: Value =
        serde_json::from_slice(&std::fs::read(dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(on_disk["from_a_newer_voice"], json!({"kept": true}));
    assert_eq!(on_disk["meetings_retention"], "after_transcription");
    assert_eq!(on_disk["ui_theme"], "dark");

    // Saving again replaces the file.
    let mut changed = loaded;
    changed.ready_chime = true;
    changed.save(dir.path()).unwrap();
    assert!(Config::load_as(dir.path(), false).unwrap().ready_chime);
}

#[cfg(unix)]
#[test]
fn saved_config_is_private_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("config-mode");
    Config::defaults(false).save(dir.path()).unwrap();
    let mode = std::fs::metadata(dir.join("config.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_damaged_file_is_kept_aside() {
    let dir = TempDir::new("config-damaged");
    std::fs::write(
        dir.join("config.json"),
        b"{\"audio_api_key\": \"sk-keep\", broken",
    )
    .unwrap();
    let error = Config::load_as(dir.path(), false).unwrap_err();
    let ConfigError::Damaged { kept_at } = error else {
        panic!("expected Damaged, got {error:?}");
    };
    assert_eq!(
        std::fs::read(&kept_at).unwrap(),
        b"{\"audio_api_key\": \"sk-keep\", broken"
    );
    assert!(!dir.join("config.json").exists());
    assert_eq!(
        Config::load_as(dir.path(), false).unwrap(),
        Config::defaults(false)
    );
}

#[test]
fn custom_and_openrouter_configs_keep_provider_endpoint_and_model() {
    // test_providers.py:355-372
    let cases = [
        ("http://localhost:1234/v1/chat/completions", "custom"),
        ("https://example.test/proxy/v1/responses", "custom"),
        (
            "https://openrouter.ai/api/v1/chat/completions",
            "openrouter",
        ),
    ];
    for (endpoint, provider) in cases {
        for stored in [None, Some(provider)] {
            let mut value = json!({"rewrite_endpoint": endpoint, "rewrite_model": "vendor/model"});
            if let Some(stored) = stored {
                value["rewrite_provider"] = json!(stored);
            }
            let cfg = load(value);
            assert_eq!(cfg.rewrite_provider, provider, "{endpoint}");
            assert_eq!(cfg.rewrite_endpoint, endpoint);
            assert_eq!(cfg.rewrite_model, "vendor/model");
        }
    }
}

#[test]
fn old_shared_mistral_config_migrates_to_both_activities() {
    // test_providers.py:374-393
    let cfg = load(json!({
        "api_key": "legacy-key",
        "endpoint": "https://api.mistral.ai/v1/audio/transcriptions",
        "model": "voxtral-old",
        "chat_endpoint": "https://api.mistral.ai/v1/chat/completions",
        "rewrite_model": "mistral-old"
    }));
    assert_eq!(cfg.audio_provider, "mistral");
    assert_eq!(cfg.audio_api_key, "legacy-key");
    assert_eq!(cfg.audio_model, "voxtral-old");
    assert_eq!(cfg.rewrite_provider, "mistral");
    assert_eq!(cfg.rewrite_api_key, "legacy-key");
    assert_eq!(cfg.rewrite_model, "mistral-old");
    // The legacy keys are gone once migrated.
    for key in ["api_key", "endpoint", "model", "chat_endpoint"] {
        assert!(!cfg.extra.contains_key(key), "{key}");
    }
}

#[test]
fn retired_local_rewriting_disables_edits_and_keeps_orukeet() {
    // test_local_integration.py:144-162
    for retired in ["local_rewrite", "Local (small LLM)"] {
        let cfg = load(json!({
            "audio_provider": "orukeet",
            "audio_model": "orukeet-onnx-int8",
            "local_model_directory": "D:\\models\\orukeet",
            "rewrite_provider": retired,
            "rewrite_api_key": "old",
            "rewrite_endpoint": "http://localhost:9999/v1/chat/completions",
            "rewrite_model": "small",
            "auto_rewrite": true,
            "rewrite_hotkey": "right ctrl",
            "local_rewrite_directory": "C:\\old",
            "local_rewrite_idle_timeout_minutes": 5
        }));
        assert!(!cfg.auto_rewrite);
        assert_eq!(cfg.rewrite_hotkey, "off");
        assert_eq!(cfg.rewrite_provider, "mistral");
        assert_eq!(cfg.rewrite_api_key, "");
        assert_eq!(
            cfg.rewrite_endpoint,
            "https://api.mistral.ai/v1/chat/completions"
        );
        assert_eq!(cfg.rewrite_model, "mistral-small-latest");
        assert_eq!(cfg.audio_provider, "orukeet");
        assert_eq!(cfg.local_model_directory, "D:\\models\\orukeet");
        assert!(!cfg.extra.contains_key("local_rewrite_directory"));
        assert!(!cfg.extra.contains_key("local_rewrite_idle_timeout_minutes"));
    }
}

#[test]
fn existing_rewrite_providers_keep_their_settings() {
    // test_local_integration.py:164-176
    for provider in ["openrouter", "custom", "ollama"] {
        let cfg = load(json!({
            "rewrite_provider": provider,
            "rewrite_endpoint": "https://example.invalid/v1/chat/completions",
            "rewrite_api_key": "key",
            "rewrite_model": "model",
            "auto_rewrite": true
        }));
        assert_eq!(cfg.rewrite_provider, provider);
        assert_eq!(
            cfg.rewrite_endpoint,
            "https://example.invalid/v1/chat/completions"
        );
        assert_eq!(cfg.rewrite_api_key, "key");
        assert_eq!(cfg.rewrite_model, "model");
        assert!(cfg.auto_rewrite);
    }
}

#[test]
fn meeting_settings_are_normalized() {
    // test_meetings_integration.py:256-285
    let cfg =
        load(json!({"meetings_audio_provider": "OpenAI", "meetings_audio_model": " whisper-1 "}));
    assert_eq!(cfg.meetings_audio_provider, "openai");
    assert_eq!(cfg.meetings_audio_model, "whisper-1");
    assert_eq!(
        load(json!({"meetings_audio_provider": "openrouter"})).meetings_audio_provider,
        "same"
    );
    assert_eq!(
        load(json!({"meetings_audio_provider": ""})).meetings_audio_provider,
        "same"
    );
    assert_eq!(
        load(json!({"meetings_retention": "forever"})).meetings_retention,
        Retention::Days7
    );
    assert_eq!(
        load(json!({"meetings_retention": "keep"})).meetings_retention,
        Retention::Keep
    );
}

#[test]
fn values_are_checked_on_load() {
    let cfg = load(json!({
        "hotkey": " Right Shift ",
        "rewrite_hotkey": "right shift",
        "language": "NL ",
        "local_model_idle_timeout_minutes": "15",
        "local_model_directory": 5,
        "paste_mode": 0,
        "ready_chime": "yes",
        "remove_fillers": [],
        "vocabulary": "Rukoo, rukoo\nOwnkey",
        "corrections": [["own key", "Ownkey"], ["x", "x"]],
        "filler_languages": ["DE", "xx"],
        "custom_fillers": ["Basically", " actually "],
        "audio_provider": "anthropic",
        "audio_endpoint": "https://api.anthropic.com/v1/messages",
        "audio_model": "claude",
        "meetings_stop_key": "right shift",
        "meetings_screen": "VIDEO",
        "ui_language": "fr",
        "ui_theme": "Light",
        "pill_edge": "left",
        "version": 7
    }));
    assert_eq!(cfg.hotkey, "right shift");
    assert_eq!(
        cfg.rewrite_hotkey, "off",
        "the rewrite key may not equal the dictation key"
    );
    assert_eq!(cfg.meetings_stop_key, "off");
    assert_eq!(cfg.language, "nl");
    assert_eq!(cfg.local_model_idle_timeout_minutes, 15.0);
    assert_eq!(cfg.local_model_directory, "");
    assert!(!cfg.paste_mode);
    assert!(cfg.ready_chime);
    assert!(!cfg.remove_fillers);
    assert_eq!(cfg.vocabulary, vec!["Rukoo", "Ownkey"]);
    assert_eq!(cfg.corrections, vec![Correction::new("own key", "Ownkey")]);
    assert_eq!(cfg.filler_languages, vec!["de"]);
    assert_eq!(cfg.custom_fillers, "Basically, actually");
    // Anthropic has no audio API: the audio settings fall back to Mistral's.
    assert_eq!(cfg.audio_provider, "mistral");
    assert_eq!(
        cfg.audio_endpoint,
        "https://api.mistral.ai/v1/audio/transcriptions"
    );
    assert_eq!(cfg.audio_model, "voxtral-mini-latest");
    assert_eq!(cfg.meetings_screen, ScreenMode::Video);
    assert_eq!(cfg.ui_language, UiLanguage::System);
    assert_eq!(cfg.ui_theme, Theme::Light);
    assert_eq!(cfg.pill_edge, PillEdge::Bottom);
    // A newer Voice's version number is never lowered.
    assert_eq!(cfg.version, 7);

    for (value, minutes) in [
        (json!(-1), 20.0),
        (json!("abc"), 20.0),
        (json!(null), 20.0),
        (json!(0), 0.0),
    ] {
        assert_eq!(
            load(json!({"local_model_idle_timeout_minutes": value}))
                .local_model_idle_timeout_minutes,
            minutes
        );
    }
    assert_eq!(load(json!({"language": "xx"})).language, "auto");
    assert_eq!(
        load(json!({"rewrite_hotkey": "bogus"})).rewrite_hotkey,
        voice_core::config::default_hotkeys_on(false, cfg!(target_os = "macos")).1
    );
}

#[test]
fn orukeet_has_no_endpoint_and_one_model() {
    let cfg = load(
        json!({"audio_provider": "Local (Orukeet)", "audio_endpoint": "", "audio_model": "whatever"}),
    );
    assert_eq!(cfg.audio_provider, "orukeet");
    assert_eq!(cfg.audio_endpoint, "");
    assert_eq!(cfg.audio_model, "orukeet-onnx-int8");
    assert_eq!(cfg.dictation_hints().language, None);
}

#[test]
fn debug_output_never_shows_keys() {
    let mut cfg = Config::defaults(false);
    cfg.audio_api_key = "sk-audio-secret".into();
    cfg.rewrite_api_key = "sk-rewrite-secret".into();
    cfg.meetings_audio_api_key = "sk-meeting-secret".into();
    let text = format!(
        "{cfg:?} {:?} {:?} {:?}",
        cfg.dictation_audio(),
        cfg.rewrite(),
        cfg.meeting_audio()
    );
    assert!(!text.contains("secret"), "{text}");
    assert!(text.contains("<redacted>"));
}

#[test]
fn provider_choices_for_each_part() {
    let mut cfg = Config::defaults(false);
    cfg.audio_api_key = "audio-key".into();
    cfg.language = "nl".into();
    cfg.vocabulary = vec!["Rukoo".into()];
    let dictation = cfg.dictation_audio();
    assert_eq!(dictation.provider, "mistral");
    assert_eq!(dictation.timeout, Duration::from_secs(30));
    assert_eq!(dictation.purpose, Purpose::Dictation);
    assert_eq!(cfg.dictation_hints().language.as_deref(), Some("nl"));
    assert_eq!(cfg.dictation_hints().vocabulary, vec!["Rukoo"]);

    // Orukeet meetings by default; the language is the recognizer's own business.
    assert_eq!(cfg.meeting_audio().provider, "orukeet");
    assert_eq!(cfg.meeting_hints().language, None);

    cfg.switch_meeting_provider("same");
    let meeting = cfg.meeting_audio();
    assert_eq!(meeting.provider, "mistral");
    assert_eq!(meeting.api_key, "audio-key");
    assert_eq!(meeting.timeout, Duration::from_secs(180));

    let rewrite = cfg.rewrite();
    assert_eq!(rewrite.purpose, Purpose::Rewrite);
    assert_eq!(
        rewrite.endpoint,
        "https://api.mistral.ai/v1/chat/completions"
    );

    assert_eq!(cfg.idle_timeout(), Duration::from_secs(20 * 60));
    cfg.local_model_idle_timeout_minutes = 0.0;
    assert_eq!(cfg.idle_timeout(), Duration::ZERO);
}

#[test]
fn switching_provider_clears_the_key() {
    let mut cfg = Config::defaults(false);
    cfg.audio_api_key = "mistral-key".into();
    cfg.switch_audio_provider("OpenAI");
    assert_eq!(cfg.audio_provider, "openai");
    assert_eq!(cfg.audio_api_key, "");
    assert_eq!(
        cfg.audio_endpoint,
        "https://api.openai.com/v1/audio/transcriptions"
    );
    assert_eq!(cfg.audio_model, "");
    cfg.switch_audio_provider("orukeet");
    assert_eq!(
        (cfg.audio_endpoint.as_str(), cfg.audio_model.as_str()),
        ("", "orukeet-onnx-int8")
    );

    cfg.rewrite_api_key = "k".into();
    cfg.switch_rewrite_provider("ollama");
    assert_eq!(cfg.rewrite_api_key, "");
    assert_eq!(cfg.rewrite_endpoint, "http://localhost:11434/api/chat");

    cfg.meetings_audio_api_key = "k".into();
    cfg.switch_meeting_provider("google");
    assert_eq!(cfg.meetings_audio_api_key, "");
    assert_eq!(
        cfg.meetings_audio_endpoint,
        "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
    );
}

#[test]
fn meeting_consent_follows_provider_and_host() {
    let mut cfg = Config::defaults(false);
    assert_eq!(cfg.meeting_consent(), ConsentState::NotNeeded);
    assert_eq!(cfg.meeting_disclosure(), None);

    cfg.switch_meeting_provider("mistral");
    cfg.meetings_audio_model = "voxtral-mini-latest".into();
    let required = ConsentState::Required {
        provider: "mistral".into(),
        label: "Mistral".into(),
        host: "api.mistral.ai".into(),
        mode: ConsentMode::Windows,
    };
    assert_eq!(cfg.meeting_consent(), required);
    let disclosure = cfg.meeting_disclosure().unwrap();
    assert_eq!(disclosure.title, "Voice will send meeting audio to Mistral");
    assert_eq!(disclosure.host, "api.mistral.ai");
    assert_eq!(disclosure.model, "voxtral-mini-latest");
    assert_eq!(
        disclosure.agree,
        "Send meeting audio to Mistral at api.mistral.ai"
    );
    assert!(disclosure.intro.contains("windows of up to 28 seconds"));
    assert_eq!(
        disclosure.retention,
        "What the provider keeps is set by its data policy; Voice has no control after upload."
    );

    assert!(cfg.grant_meeting_consent());
    assert_eq!(cfg.meeting_consent(), ConsentState::Granted);
    assert_eq!(
        cfg.meetings_transcription_consent,
        Some(Consent {
            provider: "mistral".into(),
            host: "api.mistral.ai".into(),
            mode: ConsentMode::Windows,
        })
    );

    // Another host needs the question again.
    cfg.meetings_audio_endpoint = "https://eu.mistral.example/v1/audio/transcriptions".into();
    assert!(matches!(
        cfg.meeting_consent(),
        ConsentState::Required { .. }
    ));

    // A server on this computer never needs it.
    cfg.switch_meeting_provider("custom");
    assert_eq!(
        cfg.meetings_audio_endpoint,
        "http://localhost:1234/v1/audio/transcriptions"
    );
    assert_eq!(cfg.meeting_consent(), ConsentState::NotNeeded);
    assert!(!cfg.grant_meeting_consent());

    // A stored consent survives a load; a malformed one doesn't.
    let kept = load(
        json!({"meetings_transcription_consent": {"provider": "Mistral", "host": "API.mistral.ai"}}),
    );
    assert_eq!(
        kept.meetings_transcription_consent,
        Some(Consent {
            provider: "mistral".into(),
            host: "api.mistral.ai".into(),
            mode: ConsentMode::Windows,
        })
    );
    let dropped =
        load(json!({"meetings_transcription_consent": {"provider": "orukeet", "host": "x"}}));
    assert_eq!(dropped.meetings_transcription_consent, None);
}

#[test]
fn provider_form_problems() {
    assert_eq!(
        provider_form_problem(Purpose::Dictation, "openai", "", "whisper-1"),
        Some("Choose an audio endpoint and model before saving.")
    );
    assert_eq!(
        provider_form_problem(Purpose::Dictation, "orukeet", "", ""),
        None
    );
    assert_eq!(
        provider_form_problem(
            Purpose::Rewrite,
            "mistral",
            "https://api.mistral.ai/v1/chat/completions",
            " "
        ),
        Some("Choose a rewrite endpoint and model before saving.")
    );
    assert_eq!(
        provider_form_problem(Purpose::Meetings, "mistral", "", ""),
        Some("Choose an endpoint and model for meeting transcription before saving.")
    );
    assert_eq!(
        provider_form_problem(Purpose::Meetings, "same", "", ""),
        None
    );
}

#[test]
fn same_as_dictation_resolves_to_the_dictation_provider() {
    let mut cfg = Config::defaults(false);
    cfg.audio_api_key = "audio-key".into();
    cfg.audio_model = "voxtral-mini-latest".into();
    cfg.switch_meeting_provider("same");
    cfg.meetings_transcription_consent = Some(Consent {
        provider: "mistral".into(),
        host: "api.mistral.ai".into(),
        mode: ConsentMode::Windows,
    });
    assert!(cfg.resolve_same_meeting_provider());
    assert_eq!(cfg.meetings_audio_provider, "mistral");
    assert_eq!(cfg.meetings_audio_api_key, "audio-key");
    assert_eq!(cfg.meetings_audio_endpoint, cfg.audio_endpoint);
    assert_eq!(cfg.meetings_audio_model, "voxtral-mini-latest");
    // The agreement named this provider and host, so it still holds.
    assert_eq!(cfg.meeting_consent(), ConsentState::Granted);
    // A second call changes nothing, and a dictation change no longer moves meetings.
    assert!(!cfg.resolve_same_meeting_provider());
    cfg.switch_audio_provider("openai");
    assert_eq!(cfg.meeting_audio().provider, "mistral");

    let mut local = Config::defaults(false);
    local.switch_audio_provider("orukeet");
    local.switch_meeting_provider("same");
    assert!(local.resolve_same_meeting_provider());
    assert_eq!(local.meetings_audio_provider, "orukeet");
    assert_eq!(local.meetings_audio_endpoint, "");
    assert_eq!(local.meetings_audio_model, "orukeet-onnx-int8");
}

#[test]
fn live_speaker_labels_are_on_unless_turned_off() {
    assert!(Config::defaults(false).meetings_live_speakers);
    // A config from before the setting existed, and Ownkey's own config, load with labels on.
    assert!(load(json!({})).meetings_live_speakers);
    let off = load(json!({"meetings_live_speakers": false}));
    assert!(!off.meetings_live_speakers);
    assert!(load(json!({"meetings_live_speakers": 1})).meetings_live_speakers);
    assert!(!load(json!({"meetings_live_speakers": ""})).meetings_live_speakers);
    // Ownkey's "label after Stop" is another setting: it never turns live labels on or off.
    let after_stop = load(json!({"meetings_auto_speakers": true}));
    assert!(after_stop.meetings_auto_speakers);
    assert!(after_stop.meetings_live_speakers);
    let both_off = load(json!({"meetings_auto_speakers": false, "meetings_live_speakers": false}));
    assert!(!both_off.meetings_live_speakers);

    let dir = TempDir::new("config-live-speakers");
    off.save(dir.path()).unwrap();
    assert!(
        !Config::load_as(dir.path(), false)
            .unwrap()
            .meetings_live_speakers
    );
}

#[test]
fn the_ownkey_import_leaves_live_speaker_labels_alone() {
    let dir = TempDir::new("config-import-speakers");
    let path = dir.join("ownkey.json");
    let write = |value: Value| std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

    write(json!({"meetings_auto_speakers": true}));
    let mut cfg = Config::defaults(false);
    cfg.meetings_live_speakers = false;
    voice_core::ownkey::import(&path)
        .unwrap()
        .apply(&mut cfg, false);
    assert!(
        cfg.meetings_auto_speakers,
        "the import keeps Ownkey's value"
    );
    assert!(
        !cfg.meetings_live_speakers,
        "and doesn't read it as live labels"
    );

    write(json!({"meetings_auto_speakers": false}));
    let mut on = Config::defaults(false);
    voice_core::ownkey::import(&path)
        .unwrap()
        .apply(&mut on, false);
    assert!(on.meetings_live_speakers);
}

#[test]
fn consent_mode_round_trips_and_old_agreements_mean_windows() {
    let stored = |consent: Value| load(json!({ "meetings_transcription_consent": consent }));
    let stream = stored(json!({"provider": "mistral", "host": "api.mistral.ai", "mode": "stream"}));
    assert_eq!(
        stream.meetings_transcription_consent,
        Some(Consent {
            provider: "mistral".into(),
            host: "api.mistral.ai".into(),
            mode: ConsentMode::Stream,
        })
    );
    assert_eq!(
        stored(json!({"provider": "mistral", "host": "api.mistral.ai", "mode": " STREAM "}))
            .meetings_transcription_consent
            .unwrap()
            .mode,
        ConsentMode::Stream
    );
    // Stored before streaming existed, or anything else: an agreement for recorded windows.
    for mode in [Value::Null, json!("windows"), json!("websocket"), json!(1)] {
        let mut consent = json!({"provider": "mistral", "host": "api.mistral.ai"});
        if !mode.is_null() {
            consent["mode"] = mode;
        }
        assert_eq!(
            stored(consent).meetings_transcription_consent.unwrap().mode,
            ConsentMode::Windows
        );
    }

    let dir = TempDir::new("config-consent-mode");
    stream.save(dir.path()).unwrap();
    let saved: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(
        saved["meetings_transcription_consent"],
        json!({"provider": "mistral", "host": "api.mistral.ai", "mode": "stream"})
    );
    assert_eq!(Config::load_as(dir.path(), false).unwrap(), stream);
}

/// A meeting provider on Mistral's realtime model, at `host`.
fn realtime(host: &str) -> Config {
    let mut cfg = Config::defaults(false);
    cfg.switch_meeting_provider("mistral");
    cfg.meetings_audio_endpoint = format!("https://{host}/v1/audio/transcriptions");
    cfg.meetings_audio_model = "voxtral-mini-transcribe-realtime-3".into();
    cfg
}

#[test]
fn streaming_needs_its_own_agreement() {
    // A realtime model with only an agreement for recorded windows asks again, for streaming.
    let mut cfg = realtime("api.mistral.ai");
    cfg.meetings_transcription_consent = Some(Consent {
        provider: "mistral".into(),
        host: "api.mistral.ai".into(),
        mode: ConsentMode::Windows,
    });
    assert_eq!(
        cfg.meeting_consent(),
        ConsentState::Required {
            provider: "mistral".into(),
            label: "Mistral".into(),
            host: "api.mistral.ai".into(),
            mode: ConsentMode::Stream,
        }
    );
    assert!(cfg.grant_meeting_consent());
    assert_eq!(cfg.meeting_consent(), ConsentState::Granted);
    assert_eq!(
        cfg.meetings_transcription_consent.as_ref().unwrap().mode,
        ConsentMode::Stream
    );

    // A stream agreement for one host doesn't cover another.
    let mut eu = realtime("api.eu.mistral.ai");
    eu.meetings_transcription_consent = cfg.meetings_transcription_consent.clone();
    assert!(matches!(
        eu.meeting_consent(),
        ConsentState::Required { ref host, mode: ConsentMode::Stream, .. } if host == "api.eu.mistral.ai"
    ));

    // The batch model with a stream agreement asks again too, for recorded windows.
    let mut batch = realtime("api.mistral.ai");
    batch.meetings_audio_model = "voxtral-mini-latest".into();
    batch.meetings_transcription_consent = cfg.meetings_transcription_consent.clone();
    assert!(matches!(
        batch.meeting_consent(),
        ConsentState::Required {
            mode: ConsentMode::Windows,
            ..
        }
    ));

    // Audio that stays on this computer needs no agreement, whatever is stored.
    let mut local = Config::defaults(false);
    local.meetings_transcription_consent = cfg.meetings_transcription_consent.clone();
    assert_eq!(local.meeting_consent(), ConsentState::NotNeeded);
    assert_eq!(local.meeting_disclosure(), None);

    // A realtime model name on another provider doesn't stream.
    let mut other = realtime("api.openai.com");
    other.meetings_audio_provider = "openai".into();
    assert!(matches!(
        other.meeting_consent(),
        ConsentState::Required {
            mode: ConsentMode::Windows,
            ..
        }
    ));
}

#[test]
fn the_stream_disclosure_says_what_streaming_sends() {
    let cfg = realtime("api.mistral.ai");
    let disclosure = cfg.meeting_disclosure().unwrap();
    assert_eq!(disclosure.mode, ConsentMode::Stream);
    assert_eq!(
        disclosure.title,
        "Voice will stream meeting audio to Mistral"
    );
    assert_eq!(
        disclosure.agree,
        "Stream meeting audio to Mistral at api.mistral.ai"
    );
    assert_eq!(
        disclosure.sent,
        vec!["Microphone and call audio as a continuous stream, and clips of up to 15 seconds per voice after a reconnect"]
    );
    assert!(disclosure
        .intro
        .contains("continuously while the meeting records"));
    assert!(disclosure
        .intro
        .contains("clips of up to 15 seconds of each voice heard so far"));
    assert!(!disclosure.intro.contains("28 seconds"));
    assert_eq!(
        disclosure.not_sent,
        vec!["Screen recordings", "Your notes and agent chats in Rukoo"]
    );
    assert_eq!(
        disclosure.retention,
        "What the provider keeps is set by its data policy; Voice has no control after upload."
    );

    let mut batch = cfg.clone();
    batch.meetings_audio_model = "voxtral-mini-latest".into();
    let windows = batch.meeting_disclosure().unwrap();
    assert_eq!(windows.mode, ConsentMode::Windows);
    assert!(windows.intro.contains("windows of up to 28 seconds"));
}
