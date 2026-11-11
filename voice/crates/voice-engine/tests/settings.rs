//! Settings through the engine API (voice/ui/API.md), on fakes and temp folders.

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{json, Value};
use support::{lock, sync_state, wait_for, Harness, TempDir, BUCKET, BUCKET_SECRET};
use voice_core::config::{Consent, ConsentMode};
use voice_core::{ErrorKind, ProviderError};
use voice_engine::Folder;

const SECRET: &str = "sk-test-0123456789";

fn code(result: Result<impl std::fmt::Debug, voice_engine::ApiError>) -> String {
    result.unwrap_err().code
}

#[test]
fn the_state_has_every_part_api_md_lists() {
    let h = Harness::new("state");
    h.config(json!({"audio_api_key": SECRET}));
    let engine = h.start();
    let state = engine.settings_state();
    for field in [
        "seq",
        "version",
        "devMode",
        "platform",
        "features",
        "theme",
        "locale",
        "config",
        "providers",
        "dictation",
        "rukoo",
        "models",
        "speakers",
        "storage",
        "screen",
        "updates",
        "ownkey",
        "sync",
    ] {
        assert!(state.get(field).is_some(), "{field} is missing");
    }
    // Every feature is in this Voice; bucket sync shows only where this platform can keep its keys
    // safe, which Settings report as `sync.available` (voice-sync's protector decides both). That
    // is Windows (DPAPI) and macOS (the login keychain), and not Linux.
    assert_eq!(
        state["features"],
        json!({"screen": true, "speakers": true, "sync": state["sync"]["available"]})
    );
    let keeps_keys = cfg!(any(windows, target_os = "macos"));
    assert_eq!(state["features"]["sync"], keeps_keys);
    assert_eq!(state["sync"]["available"], keeps_keys);
    assert_eq!(state["features"]["sync"], voice_sync::protect::SUPPORTED);
    // No screen program here, so no probe answered.
    assert_eq!(state["screen"], Value::Null);
    assert_eq!(state["config"]["meetings"]["screen"], "off");
    assert_eq!(state["devMode"], true);
    assert_eq!(state["version"], "0.1.0-test");
    assert_eq!(state["theme"], "dark");
    assert_eq!(state["config"]["dictation"]["hotkey"], "f13");
    assert_eq!(state["config"]["dictation"]["startAtLogin"], false);
    assert_eq!(state["config"]["transcription"]["provider"], "orukeet");
    let audio: Vec<&str> = state["providers"]["audio"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(audio, ["openai", "google", "mistral", "custom", "orukeet"]);
    assert_eq!(state["providers"]["rewrite"].as_array().unwrap().len(), 7);
    assert_eq!(state["models"]["orukeet"]["stage"], "missing");
    assert_eq!(state["models"]["orukeet"]["removable"], false);
    assert_eq!(state["models"]["orukeet"]["active"], "unloaded");
    assert_eq!(state["models"]["nemotron"]["active"], Value::Null);
    // The fake speech can't run Nemotron: it says why, and nothing labels yet.
    assert_eq!(state["config"]["meetings"]["liveSpeakers"], true);
    assert_eq!(state["speakers"]["engine"], "nemotron");
    assert_eq!(state["speakers"]["ready"], false);
    assert_eq!(state["speakers"]["reason"], "platform");
    assert_eq!(state["speakers"]["runtime"], Value::Null);
    assert_eq!(state["speakers"]["live"], json!([]));
    for feature in ["avx2", "fma", "f16c", "bmi2"] {
        assert!(state["speakers"]["cpu"][feature].is_boolean(), "{feature}");
    }
    let mistral = state["providers"]["audio"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "mistral")
        .unwrap();
    assert_eq!(
        mistral["streamingPrefix"],
        "voxtral-mini-transcribe-realtime"
    );
    assert_eq!(
        state["providers"]["audio"][0]["streamingPrefix"],
        Value::Null
    );
    assert_eq!(state["dictation"]["status"], "modelMissing");
    assert_eq!(state["rukoo"]["state"], "notRunning");
    assert!(
        state["rukoo"]["port"].as_u64().is_some(),
        "the bridge listens"
    );
    assert_eq!(
        state["rukoo"]["discoveryPath"],
        h.dir.join("bridge.json").display().to_string()
    );
    assert_eq!(state["updates"], json!({"state": "idle", "version": null}));
    assert_eq!(state["ownkey"]["configFound"], false);
    assert_eq!(state["ownkey"]["report"], Value::Null);
    assert!(
        !state.to_string().contains(SECRET),
        "a key never reaches the page"
    );
    assert_eq!(state["config"]["rewriting"]["keySaved"], false);
    // Bucket sync is off; its part of Settings shows when its store opened.
    assert_eq!(state["sync"]["state"], "off");
    assert_eq!(state["features"]["sync"], state["sync"]["available"]);
    assert_eq!(state["config"]["storage"]["sync"]["enabled"], false);
    engine.shutdown();
}

/// The newest `status` event's `screenDefault`.
fn screen_default_sent(h: &Harness) -> Value {
    let log = voice_bridge::EventLog::open(&h.data()).unwrap();
    log.after(0)
        .unwrap()
        .into_iter()
        .rev()
        .find(|e| e.event == "status")
        .unwrap()
        .data["screenDefault"]
        .clone()
}

#[test]
fn the_screen_default_is_saved_and_reaches_rukoo() {
    let mut h = Harness::new("screen-default");
    // A screen program is there; its probe can't run, so capture isn't known to be unsupported.
    h.screen_exe = h.dir.join("rukoo-voice-screen.exe");
    std::fs::write(&h.screen_exe, b"not a program").unwrap();
    h.config(json!({}));
    let engine = h.start();
    engine
        .settings_set("meetings.screen", json!("video"))
        .unwrap();
    assert_eq!(
        h.saved_config().meetings_screen,
        voice_core::config::ScreenMode::Video
    );
    assert_eq!(
        h.sink.last_settings()["config"]["meetings"]["screen"],
        "video"
    );
    // Rukoo hears the new default in a status event.
    assert_eq!(screen_default_sent(&h), "video");
    assert_eq!(
        code(engine.settings_set("meetings.screen", json!("4k"))),
        "invalid_value"
    );
    engine.shutdown();

    // Without a screen program a meeting records no screen, and Rukoo hears that as the default
    // while Settings keeps the choice.
    let h = Harness::new("screen-default-none");
    h.config(json!({}));
    let engine = h.start();
    engine
        .settings_set("meetings.screen", json!("video"))
        .unwrap();
    assert_eq!(
        h.saved_config().meetings_screen,
        voice_core::config::ScreenMode::Video
    );
    assert_eq!(screen_default_sent(&h), "off");
    engine.shutdown();
}

#[test]
fn a_setting_is_saved_applied_and_sent() {
    let h = Harness::new("set");
    h.config(json!({}));
    let engine = h.start();
    let before = h.sink.last_settings()["seq"].as_u64().unwrap();
    engine
        .settings_set("dictation.pasteMode", json!(false))
        .unwrap();
    engine
        .settings_set(
            "dictionary.vocabulary",
            json!(["Rukoo", " rukoo ", "Hermes"]),
        )
        .unwrap();
    engine.settings_set("fillers.languages", json!([])).unwrap();
    engine.settings_set("ui.theme", json!("light")).unwrap();
    engine.settings_set("ui.language", json!("nl")).unwrap();
    let state = h.sink.last_settings();
    assert!(state["seq"].as_u64().unwrap() > before);
    assert_eq!(state["config"]["dictation"]["pasteMode"], false);
    assert_eq!(
        state["config"]["dictionary"]["vocabulary"],
        json!(["Rukoo", "Hermes"])
    );
    assert_eq!(state["config"]["fillers"]["languages"], json!([]));
    assert_eq!(
        (state["theme"].as_str(), state["locale"].as_str()),
        (Some("light"), Some("nl"))
    );
    let ui = engine.ui();
    assert_eq!(
        (ui.theme, ui.locale),
        (voice_engine::Theme::Light, voice_engine::Locale::Nl)
    );
    assert!(h.sink.ui_changes.load(Ordering::SeqCst) >= 1);
    // The pill follows at once.
    wait_for("a light pill", || {
        engine.pill_snapshot().theme == voice_engine::Theme::Light
    });
    let saved = h.saved_config();
    assert!(!saved.paste_mode);
    assert_eq!(saved.vocabulary, ["Rukoo", "Hermes"]);
    assert!(saved.filler_languages.is_empty());
    engine.shutdown();
}

#[test]
fn bad_values_same_keys_and_dev_mode_are_refused() {
    let h = Harness::new("refuse");
    h.config(json!({}));
    let engine = h.start();
    assert_eq!(
        code(engine.settings_set("dictation.hotkey", json!("f14"))),
        "same_key"
    );
    assert_eq!(
        code(engine.settings_set("meetings.stopKey", json!("f13"))),
        "same_key"
    );
    assert_eq!(
        code(engine.settings_set("local.idleMinutes", json!(5000))),
        "invalid_value"
    );
    assert_eq!(
        code(engine.settings_set("transcription.provider", json!("openai"))),
        "invalid_value"
    );
    assert_eq!(
        code(engine.settings_set("dictation.startAtLogin", json!(true))),
        "dev_mode"
    );
    assert_eq!(
        h.autostart.writes.load(Ordering::SeqCst),
        0,
        "never touched in dev mode"
    );
    assert_eq!(code(engine.settings_import_ownkey()), "dev_mode");
    assert_eq!(
        code(engine.settings_set_folder("recordings", h.dir.join("x"))),
        "invalid_value"
    );
    assert_eq!(h.saved_config().hotkey, "f13", "nothing was saved");
    engine.shutdown();
}

#[test]
fn the_speaker_switch_is_voice_s_own_setting() {
    let h = Harness::new("live-speakers");
    // An Ownkey import fills Ownkey's "label after Stop"; it doesn't turn labels off.
    h.config(json!({"meetings_auto_speakers": true}));
    let engine = h.start();
    assert_eq!(
        engine.settings_state()["config"]["meetings"]["liveSpeakers"],
        true
    );
    engine
        .settings_set("meetings.liveSpeakers", json!(false))
        .unwrap();
    let cfg = h.saved_config();
    assert!(!cfg.meetings_live_speakers);
    assert!(cfg.meetings_auto_speakers, "Ownkey's value stays as it was");
    let state = h.sink.last_settings();
    assert_eq!(state["config"]["meetings"]["liveSpeakers"], false);
    assert_eq!(state["speakers"]["reason"], "off");
    assert_eq!(state["speakers"]["ready"], false);
    engine.shutdown();
}

#[test]
fn start_at_login_writes_the_run_value_only_for_an_installed_copy() {
    let mut h = Harness::new("autostart");
    h.dev_mode = false;
    let engine = h.start();
    // Decision 6: an installed copy is registered on its first start.
    assert!(h.autostart.on.load(Ordering::SeqCst));
    assert_eq!(
        engine.settings_state()["config"]["dictation"]["startAtLogin"],
        true
    );
    engine
        .settings_set("dictation.startAtLogin", json!(false))
        .unwrap();
    assert!(!h.autostart.on.load(Ordering::SeqCst));
    engine.shutdown();
    // A later start never turns it back on.
    let again = h.start();
    assert!(!h.autostart.on.load(Ordering::SeqCst));
    assert_eq!(h.autostart.writes.load(Ordering::SeqCst), 2);
    // The first run picked Orukeet, as Settings promises.
    assert_eq!(
        again.settings_state()["config"]["transcription"]["provider"],
        "orukeet"
    );
    again.shutdown();
}

fn save(engine: &voice_engine::Engine, save: Value) -> Result<(), voice_engine::ApiError> {
    engine.settings_save_provider(save)
}

#[test]
fn a_provider_form_is_checked_and_saved_as_a_whole() {
    let h = Harness::new("provider");
    h.config(json!({}));
    let engine = h.start();
    assert_eq!(
        code(save(
            &engine,
            json!({"target": "transcription", "provider": "openai", "endpoint": "", "model": "", "apiKey": null})
        )),
        "need_endpoint_model"
    );
    assert_eq!(
        code(save(
            &engine,
            json!({"target": "rewriting", "provider": "orukeet", "endpoint": "", "model": "", "apiKey": null})
        )),
        "invalid_value"
    );
    save(
        &engine,
        json!({
            "target": "transcription", "provider": "openai",
            "endpoint": " https://api.openai.com/v1/audio/transcriptions ", "model": "whisper-1",
            "apiKey": SECRET, "language": "nl"
        }),
    )
    .unwrap();
    let cfg = h.saved_config();
    assert_eq!(
        (
            cfg.audio_provider.as_str(),
            cfg.audio_endpoint.as_str(),
            cfg.audio_model.as_str(),
            cfg.language.as_str()
        ),
        (
            "openai",
            "https://api.openai.com/v1/audio/transcriptions",
            "whisper-1",
            "nl"
        )
    );
    assert_eq!(cfg.audio_api_key, SECRET);
    // null keeps the key for the same provider ...
    save(&engine, json!({"target": "transcription", "provider": "openai", "endpoint": "https://api.openai.com/v1/audio/transcriptions", "model": "gpt-4o-mini-transcribe", "apiKey": null})).unwrap();
    assert_eq!(h.saved_config().audio_api_key, SECRET);
    // ... and never carries it to another one.
    save(&engine, json!({"target": "transcription", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": "voxtral-mini-latest", "apiKey": null})).unwrap();
    assert_eq!(h.saved_config().audio_api_key, "");
    assert_eq!(engine.settings_state()["dictation"]["status"], "noKey");
    save(&engine, json!({"target": "transcription", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": "voxtral-mini-latest", "apiKey": SECRET})).unwrap();
    engine.settings_remove_key("transcription").unwrap();
    assert_eq!(h.saved_config().audio_api_key, "");
    engine.shutdown();
}

#[test]
fn a_realtime_model_is_refused_outside_meetings() {
    let h = Harness::new("realtime-form");
    h.config(json!({}));
    let engine = h.start();
    let realtime = "voxtral-mini-transcribe-realtime-3";
    for (target, endpoint) in [
        (
            "transcription",
            "https://api.mistral.ai/v1/audio/transcriptions",
        ),
        ("rewriting", "https://api.mistral.ai/v1/chat/completions"),
    ] {
        let error = save(&engine, json!({"target": target, "provider": "mistral", "endpoint": endpoint, "model": realtime, "apiKey": SECRET})).unwrap_err();
        assert_eq!(error.code, "invalid_value", "{target}");
        assert_eq!(
            error.message,
            "Realtime models work for meeting transcription only."
        );
    }
    // The name decides, whatever the provider: a proxy in front of Mistral lists the same ids.
    let error = save(&engine, json!({"target": "transcription", "provider": "custom", "endpoint": "http://localhost:1234/v1/audio/transcriptions", "model": realtime, "apiKey": null})).unwrap_err();
    assert_eq!(error.code, "invalid_value", "custom");
    let cfg = h.saved_config();
    assert_eq!(cfg.audio_provider, "orukeet", "nothing was saved");
    assert_ne!(cfg.rewrite_model, realtime);
    // Meetings take it.
    save(&engine, json!({"target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": realtime, "apiKey": SECRET, "consent": false})).unwrap();
    assert_eq!(h.saved_config().meetings_audio_model, realtime);
    let state = engine.settings_state();
    assert_eq!(state["speakers"]["engine"], "mistral");
    assert_eq!(state["speakers"]["reason"], "consent");
    engine.shutdown();
}

#[test]
fn mistral_labels_are_not_ready_without_a_meeting_key() {
    let h = Harness::new("realtime-no-key");
    h.config(json!({}));
    let engine = h.start();
    let mistral = |key: Value, consent: bool| json!({"target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": "voxtral-mini-transcribe-realtime-3", "apiKey": key, "consent": consent});
    // Settings calls labels ready only when the meeting's stream can be built.
    let stream_builds =
        || voice_core::providers::streaming_speech(&h.saved_config().meeting_audio()).is_ok();
    save(&engine, mistral(json!(SECRET), true)).unwrap();
    let state = engine.settings_state();
    assert_eq!(state["speakers"]["engine"], "mistral");
    assert_eq!(state["speakers"]["ready"], true);
    assert!(stream_builds());
    // Removing the key keeps the agreement, but the stream can't start without a key.
    engine.settings_remove_key("meetings").unwrap();
    let cfg = h.saved_config();
    assert_eq!(cfg.meetings_audio_api_key, "");
    assert!(
        cfg.meetings_transcription_consent.is_some(),
        "consent stays"
    );
    assert!(!stream_builds());
    let state = h.sink.last_settings();
    assert_eq!(state["config"]["meetings"]["keySaved"], false);
    assert_eq!(state["speakers"]["engine"], "mistral");
    assert_eq!(state["speakers"]["ready"], false);
    assert_eq!(state["speakers"]["reason"], "key");
    // Without a key and without an agreement, the key comes first: the form that takes it
    // also asks for the agreement.
    save(&engine, mistral(json!(""), false)).unwrap();
    assert_eq!(engine.settings_state()["speakers"]["reason"], "key");
    save(&engine, mistral(json!(SECRET), false)).unwrap();
    assert_eq!(engine.settings_state()["speakers"]["reason"], "consent");
    save(&engine, mistral(Value::Null, true)).unwrap();
    let state = engine.settings_state();
    assert_eq!(state["speakers"]["ready"], true);
    assert_eq!(state["speakers"]["reason"], Value::Null);
    assert!(stream_builds());
    engine.shutdown();
}

#[test]
fn orukeet_is_loaded_before_it_becomes_the_dictation_provider() {
    let h = Harness::new("orukeet-save");
    h.config(json!({
        "audio_provider": "mistral", "audio_api_key": SECRET,
        "audio_endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "audio_model": "voxtral-mini-latest"
    }));
    *lock(&h.speech.local_load) = Some(ProviderError::new(
        ErrorKind::LocalMissingModel,
        "Model missing. Download it in Settings > Transcription.",
    ));
    let engine = h.start();
    let orukeet = json!({"target": "transcription", "provider": "orukeet", "endpoint": "x", "model": "y", "apiKey": null});
    let error = save(&engine, orukeet.clone()).unwrap_err();
    assert_eq!(error.code, "failed");
    assert_eq!(
        error.message,
        "Model missing. Download it in Settings > Transcription."
    );
    // A failed load keeps the old settings.
    assert_eq!(h.saved_config().audio_provider, "mistral");
    assert_eq!(h.saved_config().audio_api_key, SECRET);
    *lock(&h.speech.local_load) = None;
    save(&engine, orukeet).unwrap();
    let cfg = h.saved_config();
    assert_eq!(
        (
            cfg.audio_provider.as_str(),
            cfg.audio_endpoint.as_str(),
            cfg.audio_model.as_str()
        ),
        ("orukeet", "", "orukeet-onnx-int8")
    );
    assert_eq!(cfg.audio_api_key, "", "Orukeet keeps no key");
    assert_eq!(h.speech.local_loads.load(Ordering::SeqCst), 2);
    engine.shutdown();
}

#[test]
fn a_change_saved_while_orukeet_loads_is_kept() {
    let h = Harness::new("save-during-load");
    h.config(json!({
        "audio_provider": "mistral", "audio_api_key": SECRET,
        "audio_endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "audio_model": "voxtral-mini-latest"
    }));
    // The first load checks the 487 MB model, which takes a while.
    *lock(&h.speech.local_load_delay) = Duration::from_millis(800);
    let engine = h.start();
    let saving = std::thread::spawn({
        let engine = engine.clone();
        move || {
            save(
                &engine,
                json!({"target": "transcription", "provider": "orukeet", "apiKey": null}),
            )
        }
    });
    wait_for("Orukeet to load", || {
        h.speech.local_loads.load(Ordering::SeqCst) == 1
    });
    // Meanwhile the user saves another page and flips a switch.
    let mistral = json!({"target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": "voxtral-mini-latest", "apiKey": SECRET, "consent": true});
    save(&engine, mistral).unwrap();
    engine
        .settings_set("dictation.pasteMode", json!(false))
        .unwrap();
    saving.join().unwrap().unwrap();
    let cfg = h.saved_config();
    assert_eq!(cfg.audio_provider, "orukeet");
    assert!(
        !cfg.paste_mode,
        "the switch flipped during the load stays off"
    );
    assert_eq!(cfg.meetings_audio_provider, "mistral");
    assert_eq!(cfg.meetings_audio_api_key, SECRET);
    assert!(cfg.meetings_transcription_consent.is_some());
    assert_eq!(
        engine.settings_state()["config"]["dictation"]["pasteMode"],
        false
    );
    engine.shutdown();
}

#[test]
fn every_settings_state_goes_out_with_the_next_seq() {
    let h = Harness::new("seq-order");
    h.config(json!({}));
    let engine = h.start();
    lock(&h.sink.settings).clear();
    // Many threads change Settings' state at once, as a download, a save and Rukoo's watcher do.
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let engine = engine.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    engine.settings_check_updates();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let seqs: Vec<u64> = lock(&h.sink.settings)
        .iter()
        .map(|state| state["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(seqs.len(), 8 * 25 * 2);
    for pair in seqs.windows(2) {
        assert_eq!(
            pair[1],
            pair[0] + 1,
            "a state went out out of order or with a seq already used"
        );
    }
    engine.shutdown();
}

#[test]
fn meeting_consent_names_the_provider_and_host() {
    let h = Harness::new("consent");
    h.config(json!({}));
    let engine = h.start();
    let mistral = |consent: bool, endpoint: &str| json!({"target": "meetings", "provider": "mistral", "endpoint": endpoint, "model": "voxtral-mini-latest", "apiKey": SECRET, "consent": consent});
    save(
        &engine,
        mistral(false, "https://api.mistral.ai/v1/audio/transcriptions"),
    )
    .unwrap();
    assert_eq!(
        engine.settings_state()["config"]["meetings"]["consent"],
        Value::Null
    );
    save(
        &engine,
        mistral(true, "https://api.mistral.ai/v1/audio/transcriptions"),
    )
    .unwrap();
    // The batch model sends recorded windows; that agreement doesn't cover a stream.
    assert_eq!(
        engine.settings_state()["config"]["meetings"]["consent"],
        json!({"provider": "mistral", "host": "api.mistral.ai", "mode": "windows"})
    );
    assert_eq!(
        h.saved_config().meetings_transcription_consent,
        Some(Consent {
            provider: "mistral".into(),
            host: "api.mistral.ai".into(),
            mode: ConsentMode::Windows,
        })
    );
    // Another host needs consent again; the form carries it only when the box is ticked.
    save(
        &engine,
        mistral(false, "https://eu.example.com/v1/audio/transcriptions"),
    )
    .unwrap();
    assert_eq!(h.saved_config().meetings_transcription_consent, None);
    // A server on this computer needs none, even with the box ticked.
    save(&engine, json!({"target": "meetings", "provider": "custom", "endpoint": "http://localhost:1234/v1/audio/transcriptions", "model": "whisper", "apiKey": "", "consent": true})).unwrap();
    assert_eq!(h.saved_config().meetings_transcription_consent, None);
    engine.shutdown();
}

#[test]
fn model_lists_use_the_saved_key_only_for_the_saved_provider() {
    let h = Harness::new("models");
    h.config(json!({"rewrite_api_key": SECRET}));
    *lock(&h.speech.models) = vec!["b-model".into(), "A-model".into(), "b-model".into()];
    let engine = h.start();
    let list = engine
        .settings_list_models(json!({"target": "rewriting", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/chat/completions", "apiKey": null}))
        .unwrap();
    assert_eq!(list, ["A-model", "b-model"], "sorted, without duplicates");
    engine
        .settings_list_models(json!({"target": "rewriting", "provider": "openai", "endpoint": "https://api.openai.com/v1/responses", "apiKey": null}))
        .unwrap();
    let requests = lock(&h.speech.model_requests).clone();
    assert_eq!(requests[0].api_key, SECRET);
    assert_eq!(
        requests[1].api_key, "",
        "the Mistral key never goes to OpenAI"
    );
    assert_eq!(
        code(engine.settings_list_models(
            json!({"target": "rewriting", "provider": "openai", "endpoint": "", "apiKey": "x"})
        )),
        "config"
    );
    assert_eq!(
        engine
            .settings_list_models(json!({"target": "transcription", "provider": "orukeet", "endpoint": "", "apiKey": null}))
            .unwrap(),
        ["orukeet-onnx-int8"]
    );
    engine.shutdown();
}

#[test]
fn model_downloads_report_progress_and_failures() {
    let h = Harness::new("download");
    h.config(json!({}));
    let engine = h.start();
    assert_eq!(
        code(engine.settings_model("whisper", "download")),
        "not_found"
    );
    assert_eq!(
        code(engine.settings_model("orukeet", "remove")),
        "not_removable"
    );
    // Both models download the same way; Nemotron is no longer refused.
    for model in ["orukeet", "nemotron"] {
        engine.settings_model(model, "download").unwrap();
        wait_for("the failed download", || {
            engine.settings_state()["models"][model]["stage"] == "failed"
        });
        let state = engine.settings_state()["models"][model].clone();
        assert_eq!(state["error"], "Checksum mismatch for encoder.int8.onnx.");
        let stages: Vec<Value> = lock(&h.sink.settings)
            .iter()
            .map(|s| s["models"][model]["stage"].clone())
            .collect();
        assert!(
            stages.contains(&json!("downloading")),
            "{model}: {stages:?}"
        );
        assert!(stages.contains(&json!("verifying")), "{model}: {stages:?}");
    }
    engine.shutdown();
}

#[test]
fn the_filler_preview_runs_the_real_cleanup() {
    let h = Harness::new("preview");
    h.config(json!({}));
    let engine = h.start();
    let sample = "Um, I think, uh, we should ehm ship it on Tuesday. Er is nog één ding.";
    let cleaned = engine
        .settings_clean_preview(
            json!({"text": sample, "enabled": true, "languages": ["en", "nl"], "custom": ""}),
        )
        .unwrap();
    assert_eq!(
        cleaned,
        voice_core::cleanup::clean(
            sample,
            &voice_core::cleanup::CleanupConfig {
                remove_fillers: true,
                filler_languages: vec!["en".into(), "nl".into()],
                custom_fillers: String::new(),
                corrections: Vec::new(),
            }
        )
    );
    assert!(cleaned.starts_with("I think"), "{cleaned}");
    let untouched = engine
        .settings_clean_preview(
            json!({"text": sample, "enabled": false, "languages": [], "custom": ""}),
        )
        .unwrap();
    assert_eq!(untouched, sample);
    assert_eq!(
        h.saved_config().filler_languages,
        ["en", "nl"],
        "a preview saves nothing"
    );
    engine.shutdown();
}

#[test]
fn the_update_check_reports_each_outcome() {
    let h = Harness::new("updates");
    h.config(json!({}));
    {
        let mut answers = lock(&h.updates.0);
        answers.push_back(Ok(Some("0.2.0".into())));
        answers.push_back(Ok(None));
        answers.push_back(Err("offline".into()));
    }
    let engine = h.start();
    assert_eq!(
        engine.settings_check_updates(),
        json!({"state": "available", "version": "0.2.0"})
    );
    assert_eq!(
        engine.settings_check_updates(),
        json!({"state": "upToDate", "version": null})
    );
    assert_eq!(
        engine.settings_check_updates(),
        json!({"state": "failed", "version": null})
    );
    assert_eq!(engine.settings_state()["updates"]["state"], "failed");
    engine.shutdown();
}

#[test]
fn a_new_token_rewrites_the_discovery_file() {
    let h = Harness::new("token");
    h.config(json!({}));
    let engine = h.start();
    let read = || -> Value {
        serde_json::from_slice(&std::fs::read(h.dir.join("bridge.json")).unwrap()).unwrap()
    };
    let before = read();
    assert_eq!(before["voiceVersion"], "0.1.0-test");
    engine.settings_new_token().unwrap();
    let after = read();
    assert_ne!(after["token"], before["token"]);
    assert_eq!(
        after["instance"], before["instance"],
        "the event log is the same"
    );
    assert_eq!(engine.settings_state()["rukoo"]["port"], after["port"]);
    assert_eq!(
        engine.folder_path(Folder::Bridge),
        Some(h.dir.path().to_path_buf())
    );
    engine.shutdown();
    assert!(
        !h.dir.join("bridge.json").exists(),
        "quitting removes the file"
    );
}

fn ownkey_copy(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.join("ownkey-config.json");
    std::fs::write(
        &path,
        json!({
            "hotkey": "right alt", "rewrite_hotkey": "right ctrl",
            "audio_provider": "mistral", "audio_api_key": SECRET,
            "audio_endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "audio_model": "voxtral-mini-latest",
            "vocabulary": ["Rukoo", "Bart"], "corrections": [{"from": "own key", "to": "Ownkey"}],
            "rewrite_provider": "mistral", "rewrite_api_key": SECRET,
            "meetings_live_transcription_policy": "allow",
            "meetings_retention": "keep"
        })
        .to_string(),
    )
    .unwrap();
    path
}

#[test]
fn the_first_start_of_an_installed_copy_imports_ownkey() {
    let mut h = Harness::new("import");
    h.dev_mode = false;
    let source = ownkey_copy(&h.dir);
    h.ownkey_config = Some(source.clone());
    let before = std::fs::read(&source).unwrap();
    let engine = h.start();
    assert_eq!(
        std::fs::read(&source).unwrap(),
        before,
        "Ownkey's file is only read"
    );
    assert_eq!(engine.settings_start_page().as_deref(), Some("import"));
    assert_eq!(engine.settings_start_page(), None, "only once");
    let cfg = h.saved_config();
    assert_eq!(cfg.hotkey, "right alt");
    assert_eq!(cfg.audio_provider, "mistral");
    assert_eq!(cfg.vocabulary, ["Rukoo", "Bart"]);
    // Ownkey's "same as dictation" became the provider it pointed at, consent included.
    assert_eq!(cfg.meetings_audio_provider, "mistral");
    assert_eq!(
        cfg.meetings_transcription_consent,
        Some(Consent {
            provider: "mistral".into(),
            host: "api.mistral.ai".into(),
            mode: ConsentMode::Windows,
        })
    );
    let state = engine.settings_state();
    let report = &state["ownkey"]["report"];
    assert_eq!(
        report["hotkeys"],
        json!({"dictation": "right alt", "rewrite": "right ctrl"})
    );
    assert_eq!(report["transcription"]["keyImported"], true);
    assert_eq!(report["dictionary"], json!({"words": 2, "corrections": 1}));
    assert_eq!(report["meetings"]["provider"], "mistral");
    assert_eq!(report["meetings"]["consent"], true);
    assert_eq!(report["meetings"]["retention"], "keep");
    assert_eq!(
        report["fillers"],
        Value::Null,
        "Ownkey's file had no filler settings"
    );
    assert_eq!(report["skipped"], json!(["summaries"]));
    assert_eq!(state["ownkey"]["configFound"], true);
    assert_eq!(state["ownkey"]["keys"], json!(["right alt", "right ctrl"]));
    assert!(state["ownkey"]["lastImport"].as_u64().is_some());
    assert!(!state.to_string().contains(SECRET));
    // Importing again from Settings works the same way.
    let again = engine.settings_import_ownkey().unwrap();
    assert_eq!(again["dictionary"]["words"], 2);
    engine.shutdown();
}

#[test]
fn a_development_copy_imports_only_on_request_and_keeps_its_keys() {
    let mut h = Harness::new("import-dev");
    let source = ownkey_copy(&h.dir);
    h.ownkey_config = Some(source.clone());
    let engine = h.start();
    assert_eq!(
        h.saved_config().audio_provider,
        "orukeet",
        "no automatic import in dev mode"
    );
    assert_eq!(engine.settings_start_page(), None);
    engine.shutdown();

    h.import_from = Some(source);
    let engine = h.start();
    let cfg = h.saved_config();
    assert_eq!(cfg.audio_provider, "mistral");
    assert_eq!(
        (cfg.hotkey.as_str(), cfg.rewrite_hotkey.as_str()),
        ("f13", "f14")
    );
    assert_eq!(
        engine.settings_state()["ownkey"]["report"]["hotkeys"],
        Value::Null
    );
    engine.shutdown();
}

#[test]
fn a_missing_ownkey_config_is_not_found() {
    let mut h = Harness::new("import-missing");
    h.dev_mode = false;
    h.ownkey_config = Some(h.dir.join("nope.json"));
    let engine = h.start();
    assert_eq!(code(engine.settings_import_ownkey()), "not_found");
    engine.shutdown();
}

#[test]
fn a_damaged_config_is_kept_aside_and_voice_starts_with_defaults() {
    let h = Harness::new("damaged");
    std::fs::create_dir_all(h.data()).unwrap();
    std::fs::write(h.data().join("config.json"), b"{ not json").unwrap();
    let engine = h.start();
    let note = h.sink.wait_note("app.notify.settings");
    assert!(note.body.contains("damaged"), "{}", note.body);
    assert_eq!(
        engine.settings_state()["config"]["dictation"]["hotkey"],
        "f13"
    );
    let kept = std::fs::read_dir(h.data())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("config.damaged-")
        });
    assert!(kept);
    engine.shutdown();
}

#[test]
fn folders_point_into_the_data_folder() {
    let h = Harness::new("folders");
    h.config(json!({}));
    let engine = h.start();
    assert_eq!(
        engine.folder_path(Folder::Logs),
        Some(h.data().join("logs"))
    );
    assert_eq!(
        engine.folder_path(Folder::Recordings),
        Some(h.data().join("meetings"))
    );
    assert_eq!(
        engine.folder_path(Folder::Models),
        Some(h.data().join("models"))
    );
    engine
        .settings_set_folder("models", h.dir.join("chosen"))
        .unwrap();
    assert_eq!(
        engine.folder_path(Folder::Models),
        Some(h.dir.join("chosen"))
    );
    assert_eq!(
        engine.settings_state()["config"]["local"]["modelFolder"],
        h.dir.join("chosen").display().to_string()
    );
    engine.shutdown();
}

// Bucket sync (voice/ui/API.md, settings_sync_*) ---------------------------------------------

fn sync_config(engine: &voice_engine::Engine) -> Value {
    engine.settings_state()["config"]["storage"]["sync"].clone()
}

fn saved_sync(h: &Harness) -> Value {
    h.saved_config()
        .extra
        .get("storage_sync")
        .cloned()
        .unwrap_or(Value::Null)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn bucket_sync_turns_on_only_for_a_bucket_that_passed_its_test() {
    let h = Harness::new("sync-settings");
    h.config(json!({}));
    let fake = support::fake_s3();
    let engine = h.start();
    assert_eq!(sync_config(&engine)["enabled"], false, "off by default");
    assert_eq!(sync_state(&engine)["state"], "off");
    assert_eq!(
        code(engine.settings_set("storage.sync.enabled", json!(true))),
        "needs_test"
    );
    for (key, value) in [
        ("storage.sync.service", "other"),
        ("storage.sync.endpoint", fake.endpoint().as_str()),
        ("storage.sync.region", "us-east-1"),
        ("storage.sync.bucket", BUCKET),
        ("storage.sync.accessKeyId", support::ACCESS_KEY),
    ] {
        engine.settings_set(key, json!(value)).unwrap();
    }
    assert_eq!(code(engine.settings_sync_test()), "no_secret");
    assert_eq!(
        code(engine.settings_set("storage.sync.secret", json!(BUCKET_SECRET))),
        "invalid_value"
    );
    assert_eq!(
        code(engine.settings_sync_save_secret(String::new())),
        "invalid_value"
    );
    assert_eq!(
        code(engine.settings_sync_save_secret("x".repeat(1025))),
        "invalid_value"
    );

    engine
        .settings_sync_save_secret(BUCKET_SECRET.into())
        .unwrap();
    assert_eq!(sync_config(&engine)["secretSaved"], true);
    assert_eq!(sync_config(&engine)["secret"], "");
    let config_json = std::fs::read_to_string(h.data().join("config.json")).unwrap();
    assert!(
        !config_json.contains(BUCKET_SECRET),
        "no secret in config.json"
    );
    assert!(h.data().join("sync").join("bucket-secret.bin").is_file());
    assert!(!engine.settings_state().to_string().contains(BUCKET_SECRET));

    // Test connection against the fake bucket marks exactly these fields as tested.
    assert_eq!(
        engine.settings_sync_test().unwrap(),
        json!({"otherKeys": 0})
    );
    let saved = saved_sync(&h);
    assert_eq!(saved["tested"].as_str().map(str::len), Some(64), "{saved}");
    assert_eq!(saved["urlStyle"], "path");
    assert_eq!(sync_config(&engine)["tested"], true);
    engine
        .settings_set("storage.sync.enabled", json!(true))
        .unwrap();
    assert_eq!(saved_sync(&h)["enabled"], true);

    // Another bucket hasn't passed a test: uploads pause until it does. The switch stays on, so
    // retention keeps holding what hasn't reached the bucket.
    engine
        .settings_set("storage.sync.bucket", json!("elsewhere"))
        .unwrap();
    assert_eq!(sync_config(&engine)["enabled"], true);
    assert_eq!(sync_config(&engine)["tested"], false);
    assert_eq!(sync_state(&engine)["state"], "paused");
    // Off, it turns on again only after a test.
    engine
        .settings_set("storage.sync.enabled", json!(false))
        .unwrap();
    assert_eq!(sync_state(&engine)["state"], "off");
    assert_eq!(
        code(engine.settings_set("storage.sync.enabled", json!(true))),
        "needs_test"
    );
    engine
        .settings_set("storage.sync.bucket", json!(BUCKET))
        .unwrap();
    engine
        .settings_set("storage.sync.enabled", json!(true))
        .unwrap();
    assert_ne!(sync_state(&engine)["state"], "paused");

    // A new secret needs a new test; uploads go on once it passes.
    engine
        .settings_sync_save_secret("another-secret".into())
        .unwrap();
    assert_eq!(sync_config(&engine)["enabled"], true);
    assert_eq!(sync_state(&engine)["state"], "paused");
    assert_eq!(saved_sync(&h).get("tested"), None);
    engine.settings_sync_test().unwrap();
    assert_ne!(sync_state(&engine)["state"], "paused");

    // Removing the secret pauses sync too, and the test needs a secret again.
    engine.settings_remove_key("sync").unwrap();
    let after = sync_config(&engine);
    assert_eq!(
        (&after["enabled"], &after["secretSaved"], &after["tested"]),
        (&json!(true), &json!(false), &json!(false))
    );
    assert_eq!(sync_state(&engine)["state"], "paused");
    assert!(!h.data().join("sync").join("bucket-secret.bin").exists());
    assert_eq!(code(engine.settings_sync_test()), "no_secret");
    engine.shutdown();
    fake.stop();
}

#[test]
fn a_test_counts_only_for_the_secret_it_read() {
    let h = Harness::new("sync-test-secret");
    h.config(json!({}));
    let fake = support::fake_s3();
    let engine = h.start();
    for (key, value) in [
        ("storage.sync.service", "other"),
        ("storage.sync.endpoint", fake.endpoint().as_str()),
        ("storage.sync.region", "us-east-1"),
        ("storage.sync.bucket", BUCKET),
        ("storage.sync.accessKeyId", support::ACCESS_KEY),
    ] {
        engine.settings_set(key, json!(value)).unwrap();
    }
    engine
        .settings_sync_save_secret(BUCKET_SECRET.into())
        .unwrap();
    // Every answer of the bucket takes a while, so the test runs while the secret changes.
    fake.faults().stall = Some(std::time::Duration::from_millis(250));
    for change in ["replace", "remove"] {
        let testing = {
            let engine = engine.clone();
            std::thread::spawn(move || engine.settings_sync_test())
        };
        std::thread::sleep(std::time::Duration::from_millis(400));
        match change {
            "replace" => engine
                .settings_sync_save_secret("an-untested-secret".into())
                .unwrap(),
            _ => engine.settings_remove_key("sync").unwrap(),
        }
        let result = testing.join().unwrap();
        assert_eq!(code(result), "failed", "{change}");
        assert_eq!(sync_config(&engine)["tested"], false, "{change}");
        assert_eq!(
            code(engine.settings_set("storage.sync.enabled", json!(true))),
            "needs_test",
            "{change}"
        );
        if change == "replace" {
            engine
                .settings_sync_save_secret(BUCKET_SECRET.into())
                .unwrap();
        }
    }
    engine.shutdown();
    fake.stop();
}

/// A new secret is never trusted with the old secret's test (spec D6). When `config.json` can't be
/// replaced, the save of the secret fails and leaves the tested secret in place, also after a
/// restart; once `config.json` can be written, the new secret pauses uploads until it passes.
#[cfg(windows)]
#[test]
fn a_secret_isn_t_replaced_while_config_json_can_t_be_saved() {
    use std::os::windows::fs::OpenOptionsExt;
    /// `FILE_SHARE_READ` alone: nobody may delete or replace the file while it is open.
    const SHARE_READ: u32 = 0x1;

    let h = Harness::new("sync-secret-unsaved");
    h.config(json!({}));
    let fake = support::fake_s3();
    let engine = h.start();
    support::connect_bucket(&engine, &fake.endpoint());
    let secret_file = h.data().join("sync").join("bucket-secret.bin");
    let tested_secret = std::fs::read(&secret_file).unwrap();
    let tested = saved_sync(&h)["tested"].clone();
    assert!(tested.is_string(), "{tested}");

    // A virus scanner or an editor holds config.json open without delete sharing.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ)
        .open(h.data().join("config.json"))
        .unwrap();
    assert_eq!(
        code(engine.settings_sync_save_secret("an-untested-secret".into())),
        "failed"
    );
    assert_eq!(
        std::fs::read(&secret_file).unwrap(),
        tested_secret,
        "the tested secret stays"
    );
    assert_eq!(sync_config(&engine)["tested"], true);
    assert_ne!(sync_state(&engine)["state"], "paused");
    drop(held);
    engine.shutdown();
    let engine = h.start();
    assert_eq!(saved_sync(&h)["tested"], tested);
    assert_eq!(std::fs::read(&secret_file).unwrap(), tested_secret);
    assert_ne!(sync_state(&engine)["state"], "paused");

    // Saved, the new secret pauses uploads until Test connection passes with it.
    engine
        .settings_sync_save_secret("an-untested-secret".into())
        .unwrap();
    assert_ne!(std::fs::read(&secret_file).unwrap(), tested_secret);
    assert_eq!(sync_config(&engine)["tested"], false);
    assert_eq!(sync_state(&engine)["state"], "paused");
    engine.shutdown();
    let engine = h.start();
    assert_eq!(saved_sync(&h).get("tested"), None);
    assert_eq!(sync_state(&engine)["state"], "paused");
    engine.shutdown();
    fake.stop();
}

#[test]
fn a_failed_test_says_why_and_leaves_sync_off() {
    let h = Harness::new("sync-test-fails");
    h.config(json!({}));
    let fake = support::fake_s3();
    let engine = h.start();
    for (key, value) in [
        ("storage.sync.service", "other"),
        ("storage.sync.endpoint", fake.endpoint().as_str()),
        ("storage.sync.bucket", "no-such-bucket"),
        ("storage.sync.accessKeyId", support::ACCESS_KEY),
    ] {
        engine.settings_set(key, json!(value)).unwrap();
    }
    engine
        .settings_sync_save_secret(BUCKET_SECRET.into())
        .unwrap();
    assert_eq!(code(engine.settings_sync_test()), "bucket");
    engine
        .settings_set("storage.sync.bucket", json!(BUCKET))
        .unwrap();
    engine
        .settings_set("storage.sync.accessKeyId", json!("someone-else"))
        .unwrap();
    assert_eq!(code(engine.settings_sync_test()), "auth");
    assert_eq!(sync_config(&engine)["tested"], false);
    assert_eq!(
        code(engine.settings_set("storage.sync.enabled", json!(true))),
        "needs_test"
    );
    // An endpoint that isn't https, or on this computer, is never saved.
    assert_eq!(
        code(engine.settings_set("storage.sync.endpoint", json!("http://example.com"))),
        "invalid_value"
    );
    engine.shutdown();
    fake.stop();
}

#[test]
fn a_plain_bucket_secret_moves_out_of_config_json_at_start() {
    let h = Harness::new("sync-migrate");
    h.config(json!({"storage_sync": {"bucket": "meetings", "secret": "plain-secret-1234"}}));
    let engine = h.start();
    let config_json = std::fs::read_to_string(h.data().join("config.json")).unwrap();
    assert!(!config_json.contains("plain-secret-1234"), "{config_json}");
    assert_eq!(saved_sync(&h)["bucket"], "meetings", "the rest stays");
    assert_eq!(sync_config(&engine)["secretSaved"], true);
    let blob = std::fs::read(h.data().join("sync").join("bucket-secret.bin")).unwrap();
    assert!(blob.starts_with(b"RKVS"));
    assert!(!contains(&blob, b"plain-secret-1234"));
    engine.shutdown();
    // Started again, nothing moves twice and the secret is still there.
    let engine = h.start();
    assert_eq!(sync_config(&engine)["secretSaved"], true);
    engine.shutdown();
}

#[test]
fn the_key_is_shown_marked_saved_copied_hidden_and_imported() {
    let h = Harness::new("sync-key");
    h.config(json!({}));
    let engine = h.start();
    assert_eq!(sync_state(&engine)["key"]["exists"], false);
    let shown = engine.settings_sync_key("show", None).unwrap();
    let code_text = shown["code"].as_str().unwrap().to_string();
    assert!(code_text.starts_with("RKV1-"), "{code_text}");
    let id = shown["id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 8);
    let key = sync_state(&engine)["key"].clone();
    assert_eq!(
        (&key["exists"], &key["id"], &key["exported"]),
        (&json!(true), &json!(id), &json!(false))
    );
    // The same key every time, and never in Settings' state.
    assert_eq!(
        engine.settings_sync_key("show", None).unwrap()["code"],
        code_text.as_str()
    );
    assert!(!engine.settings_state().to_string().contains(&code_text));
    engine.settings_sync_key("saved", None).unwrap();
    assert_eq!(sync_state(&engine)["key"]["exported"], true);

    // Copy goes through the hidden clipboard path and never to the page.
    engine.settings_sync_key_copy().unwrap();
    assert_eq!(*lock(&h.input.hidden_copies), vec![code_text.clone()]);

    assert_eq!(
        code(engine.settings_sync_key("import", Some("RKV1-not-a-key".into()))),
        "invalid_value"
    );
    assert_eq!(
        code(engine.settings_sync_key("import", None)),
        "invalid_value"
    );
    assert_eq!(
        code(engine.settings_sync_key("forget", None)),
        "invalid_value"
    );
    // Nothing was uploaded yet, so this computer may take a key, in any case and spacing.
    let messy = code_text.to_lowercase().replace('-', " ");
    let imported = engine.settings_sync_key("import", Some(messy)).unwrap();
    assert_eq!(imported["id"], id.as_str());
    engine.shutdown();
}

#[cfg(windows)]
#[test]
fn dpapi_keeps_neither_the_secret_nor_the_key_in_plain_text() {
    let mut h = Harness::new("sync-dpapi");
    h.protector = std::sync::Arc::new(voice_sync::protect::Dpapi);
    h.config(json!({}));
    let engine = h.start();
    engine
        .settings_sync_save_secret(BUCKET_SECRET.into())
        .unwrap();
    let code_text = engine.settings_sync_key("show", None).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_string();
    let key = support::key_bytes(&code_text);
    assert_eq!(key.len(), 32);
    let sync_dir = h.data().join("sync");
    let master = std::fs::read(sync_dir.join("master-key.bin")).unwrap();
    let secret = std::fs::read(sync_dir.join("bucket-secret.bin")).unwrap();
    assert!(master.starts_with(b"RKVK") && secret.starts_with(b"RKVS"));
    for (name, bytes) in [("master-key.bin", &master), ("bucket-secret.bin", &secret)] {
        assert!(!contains(bytes, &key), "{name} holds the key");
        assert!(
            !contains(bytes, BUCKET_SECRET.as_bytes()),
            "{name} holds the secret"
        );
    }
    let key_json = std::fs::read_to_string(sync_dir.join("key.json")).unwrap();
    assert!(!key_json.contains(BUCKET_SECRET) && !key_json.contains(&code_text));
    let config_json = std::fs::read(h.data().join("config.json")).unwrap();
    assert!(!contains(&config_json, &key));
    assert!(!contains(&config_json, BUCKET_SECRET.as_bytes()));
    engine.shutdown();
    // A new start opens both again.
    let engine = h.start();
    assert_eq!(sync_config(&engine)["secretSaved"], true);
    assert_eq!(
        engine.settings_sync_key("show", None).unwrap()["code"],
        code_text.as_str()
    );
    engine.shutdown();
}
