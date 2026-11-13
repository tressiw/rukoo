//! The dotted keys of voice/ui/API.md and Ownkey's flat keys in `config.json`.

use serde_json::{json, Value};
use voice_core::cleanup::{self, FILLER_LANGUAGES};
use voice_core::config::{
    offered_hotkeys, Config, PillEdge, Retention, RewriteTone, ScreenMode, Theme, UiLanguage,
};

use crate::api::ApiError;

fn invalid(key: &str) -> ApiError {
    ApiError::new("invalid_value", format!("That isn't a value {key} takes."))
}

/// `state.config`: every setting under the key `settings_set` takes for it. Keys never appear;
/// only whether one is saved. The bucket secret lives with DPAPI; `secret_saved` says whether
/// there is one.
pub fn view(cfg: &Config, start_at_login: bool, secret_saved: bool) -> Value {
    let saved = |key: &str| !key.trim().is_empty();
    let enum_text = |value: Value| value.as_str().unwrap_or_default().to_string();
    let sync_value = crate::sync::sync_value(cfg);
    let tested = crate::sync::is_tested(&sync_value);
    let sync = sync_value.as_object().cloned().unwrap_or_default();
    let sync_text = |name: &str| {
        sync.get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    json!({
        "dictation": {
            "hotkey": cfg.hotkey,
            "rewriteHotkey": cfg.rewrite_hotkey,
            "pasteMode": cfg.paste_mode,
            "readyChime": cfg.ready_chime,
            "startAtLogin": start_at_login,
        },
        "transcription": {
            "provider": cfg.audio_provider,
            "endpoint": cfg.audio_endpoint,
            "model": cfg.audio_model,
            "language": cfg.language,
            "keySaved": saved(&cfg.audio_api_key),
        },
        "local": {
            "modelFolder": cfg.local_model_directory,
            "idleMinutes": cfg.local_model_idle_timeout_minutes.round() as u64,
        },
        "dictionary": {
            "vocabulary": cfg.vocabulary,
            "corrections": cfg.corrections.iter().map(|c| json!({"from": c.from, "to": c.to})).collect::<Vec<_>>(),
        },
        "fillers": {
            "enabled": cfg.remove_fillers,
            "languages": cfg.filler_languages,
            "custom": cfg.custom_fillers,
        },
        "rewriting": {
            "provider": cfg.rewrite_provider,
            "endpoint": cfg.rewrite_endpoint,
            "model": cfg.rewrite_model,
            "keySaved": saved(&cfg.rewrite_api_key),
            "auto": cfg.auto_rewrite,
            "tone": enum_text(json!(cfg.rewrite_tone)),
            "formatting": cfg.rewrite_formatting,
            "instructions": cfg.rewrite_custom_instructions,
        },
        "meetings": {
            "provider": cfg.meetings_audio_provider,
            "endpoint": cfg.meetings_audio_endpoint,
            "model": cfg.meetings_audio_model,
            "keySaved": saved(&cfg.meetings_audio_api_key),
            "consent": super::consent_view(cfg.meetings_transcription_consent.as_ref()),
            "retention": enum_text(json!(cfg.meetings_retention)),
            "stopKey": cfg.meetings_stop_key,
            "screen": enum_text(json!(cfg.meetings_screen)),
            "liveSpeakers": cfg.meetings_live_speakers,
        },
        "ui": {
            "language": enum_text(json!(cfg.ui_language)),
            "theme": enum_text(json!(cfg.ui_theme)),
            "pillEdge": enum_text(json!(cfg.pill_edge)),
            "hidePillFromCapture": cfg.pill_hide_from_capture,
        },
        "updates": { "check": cfg.updates_check },
        "storage": {
            "sync": {
                "enabled": sync.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                "service": sync.get("service").and_then(Value::as_str).unwrap_or("upcloud"),
                "endpoint": sync_text("endpoint"),
                "region": sync_text("region"),
                "bucket": sync_text("bucket"),
                "prefix": sync_text("prefix"),
                "accessKeyId": sync_text("accessKeyId"),
                "retention": sync.get("retention").and_then(Value::as_str).unwrap_or("keep"),
                "secretSaved": secret_saved,
                "tested": tested,
                // The secret never reaches the page.
                "secret": "",
            }
        },
    })
}

/// `storage_sync` in `config.json`, made an object when it isn't one.
fn sync_object(cfg: &mut Config) -> &mut serde_json::Map<String, Value> {
    let sync = cfg
        .extra
        .entry(crate::sync::SYNC_KEY)
        .or_insert_with(|| json!({}));
    if !sync.is_object() {
        *sync = json!({});
    }
    sync.as_object_mut().expect("made an object above")
}

/// Saving or removing the bucket secret clears the passed Test connection. The switch stays as the
/// user left it: while it is on, uploads pause until a new test passes, and retention keeps
/// holding media that hasn't reached the bucket (spec D6, D8). Only the user turns it off.
pub fn untest_sync(cfg: &mut Config) {
    sync_object(cfg).remove("tested");
}

/// One `storage.sync.*` key (voice/ui/API.md). The switch turns on only for fields that passed
/// Test connection. A changed connection field no longer matches the tested fingerprint, so
/// uploads pause until the new fields pass; the switch stays where the user put it.
fn set_sync(cfg: &mut Config, key: &str, value: Value) -> Result<(), ApiError> {
    let field = &key["storage.sync.".len()..];
    let text = |value: &Value| value.as_str().map(|s| s.trim().to_string());
    let next = match field {
        "secret" => {
            return Err(ApiError::new(
                "invalid_value",
                "Save the secret with settings_sync_save_secret.",
            ))
        }
        "enabled" => {
            let on = boolean(key, &value)?;
            if on && !crate::sync::is_tested(&Value::Object(sync_object(cfg).clone())) {
                return Err(ApiError::new(
                    "needs_test",
                    "Test the connection before you turn sync on.",
                ));
            }
            sync_object(cfg).insert("enabled".into(), json!(on));
            return Ok(());
        }
        "retention" => {
            let name = text(&value)
                .filter(|n| voice_sync::BucketRetention::parse(n).is_some())
                .ok_or_else(|| invalid(key))?;
            sync_object(cfg).insert("retention".into(), json!(name));
            return Ok(());
        }
        "service" => text(&value)
            .filter(|n| voice_sync::Service::parse(n).is_some())
            .ok_or_else(|| invalid(key))?,
        "endpoint" => {
            let endpoint = text(&value).ok_or_else(|| invalid(key))?;
            if !endpoint.is_empty() {
                voice_sync::SyncConfig::check_endpoint(&endpoint)
                    .map_err(|e| ApiError::new("invalid_value", e.message))?;
            }
            endpoint
        }
        "region" | "bucket" | "prefix" | "accessKeyId" => {
            text(&value).ok_or_else(|| invalid(key))?
        }
        _ => return Err(invalid(key)),
    };
    sync_object(cfg).insert(field.into(), json!(next));
    Ok(())
}

fn boolean(key: &str, value: &Value) -> Result<bool, ApiError> {
    value.as_bool().ok_or_else(|| invalid(key))
}

fn string(key: &str, value: &Value) -> Result<String, ApiError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| invalid(key))
}

fn choice<T: serde::de::DeserializeOwned>(key: &str, value: &Value) -> Result<T, ApiError> {
    serde_json::from_value(value.clone()).map_err(|_| invalid(key))
}

/// A key name, or `off` where `allow_off`.
fn hotkey(key: &str, value: &Value, allow_off: bool) -> Result<String, ApiError> {
    let name = value
        .as_str()
        .map(|s| s.trim().to_lowercase())
        .ok_or_else(|| invalid(key))?;
    if (allow_off && name == "off") || offered_hotkeys().contains(&name.as_str()) {
        Ok(name)
    } else {
        Err(invalid(key))
    }
}

fn same_key(name: &str) -> ApiError {
    ApiError::new(
        "same_key",
        format!("{name} is already the dictation, rewrite or stop key."),
    )
}

/// `fillers.languages`: a subset of Ownkey's five; an empty list stays empty.
pub fn filler_languages(value: &Value) -> Result<Vec<String>, ApiError> {
    let list = value
        .as_array()
        .ok_or_else(|| invalid("fillers.languages"))?;
    let mut languages: Vec<String> = Vec::new();
    for item in list {
        let code = item
            .as_str()
            .map(|s| s.trim().to_lowercase())
            .ok_or_else(|| invalid("fillers.languages"))?;
        if !FILLER_LANGUAGES.iter().any(|(known, _, _)| *known == code) {
            return Err(invalid("fillers.languages"));
        }
        if !languages.contains(&code) {
            languages.push(code);
        }
    }
    Ok(languages)
}

/// Applies one `settings_set` to the config, or refuses it.
pub fn set(cfg: &mut Config, key: &str, value: Value) -> Result<(), ApiError> {
    match key {
        "dictation.hotkey" => {
            let name = hotkey(key, &value, false)?;
            if name == cfg.rewrite_hotkey || name == cfg.meetings_stop_key {
                return Err(same_key(&name));
            }
            cfg.hotkey = name;
        }
        "dictation.rewriteHotkey" => {
            let name = hotkey(key, &value, true)?;
            if name != "off" && (name == cfg.hotkey || name == cfg.meetings_stop_key) {
                return Err(same_key(&name));
            }
            cfg.rewrite_hotkey = name;
        }
        "meetings.stopKey" => {
            let name = hotkey(key, &value, true)?;
            if name != "off" && (name == cfg.hotkey || name == cfg.rewrite_hotkey) {
                return Err(same_key(&name));
            }
            cfg.meetings_stop_key = name;
        }
        "dictation.pasteMode" => cfg.paste_mode = boolean(key, &value)?,
        "dictation.readyChime" => cfg.ready_chime = boolean(key, &value)?,
        "ui.pillEdge" => cfg.pill_edge = choice::<PillEdge>(key, &value)?,
        "ui.hidePillFromCapture" => cfg.pill_hide_from_capture = boolean(key, &value)?,
        "local.modelFolder" => cfg.local_model_directory = string(key, &value)?.trim().to_string(),
        "local.idleMinutes" => {
            let minutes = value
                .as_u64()
                .filter(|m| *m <= 1440)
                .ok_or_else(|| invalid(key))?;
            cfg.local_model_idle_timeout_minutes = minutes as f64;
        }
        "dictionary.vocabulary" => {
            let list = value.as_array().ok_or_else(|| invalid(key))?;
            let words = list
                .iter()
                .map(|w| w.as_str().map(str::to_string).ok_or_else(|| invalid(key)))
                .collect::<Result<Vec<_>, _>>()?;
            cfg.vocabulary = cleanup::normalize_vocabulary_list(&words);
        }
        "dictionary.corrections" => {
            let list = value.as_array().ok_or_else(|| invalid(key))?;
            if !list
                .iter()
                .all(|c| c["from"].is_string() && c["to"].is_string())
            {
                return Err(invalid(key));
            }
            cfg.corrections = cleanup::normalize_corrections(&value);
        }
        "fillers.enabled" => cfg.remove_fillers = boolean(key, &value)?,
        "fillers.languages" => cfg.filler_languages = filler_languages(&value)?,
        "fillers.custom" => {
            let text = string(key, &value)?;
            cfg.custom_fillers = cleanup::normalize_vocabulary_str(&text).join(", ");
        }
        "rewriting.auto" => cfg.auto_rewrite = boolean(key, &value)?,
        "rewriting.tone" => cfg.rewrite_tone = choice::<RewriteTone>(key, &value)?,
        "rewriting.formatting" => cfg.rewrite_formatting = boolean(key, &value)?,
        "rewriting.instructions" => cfg.rewrite_custom_instructions = string(key, &value)?,
        "meetings.retention" => cfg.meetings_retention = choice::<Retention>(key, &value)?,
        "meetings.screen" => cfg.meetings_screen = choice::<ScreenMode>(key, &value)?,
        // Ownkey's `meetings_auto_speakers` meant "label after Stop"; Voice labels while a
        // meeting records, under its own key, and leaves Ownkey's unread.
        "meetings.liveSpeakers" => cfg.meetings_live_speakers = boolean(key, &value)?,
        "ui.language" => cfg.ui_language = choice::<UiLanguage>(key, &value)?,
        "ui.theme" => cfg.ui_theme = choice::<Theme>(key, &value)?,
        "updates.check" => cfg.updates_check = boolean(key, &value)?,
        _ if key.starts_with("storage.sync.") => set_sync(cfg, key, value)?,
        // transcription.*, rewriting.provider and the other provider fields are saved as a
        // whole with settings_save_provider.
        _ => {
            return Err(ApiError::new(
                "invalid_value",
                format!("{key} can't be set on its own."),
            ))
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_key_round_trips_through_the_view() {
        let mut cfg = Config::defaults(true);
        let cases = [
            ("dictation.hotkey", json!("f15")),
            ("dictation.rewriteHotkey", json!("off")),
            ("dictation.pasteMode", json!(false)),
            ("dictation.readyChime", json!(true)),
            ("ui.pillEdge", json!("top")),
            ("ui.hidePillFromCapture", json!(false)),
            ("local.modelFolder", json!("D:\\Models")),
            ("local.idleMinutes", json!(0)),
            ("dictionary.vocabulary", json!(["Rukoo", "Ownkey"])),
            (
                "dictionary.corrections",
                json!([{"from": "rocco", "to": "Rukoo"}]),
            ),
            ("fillers.enabled", json!(false)),
            ("fillers.languages", json!([])),
            ("fillers.custom", json!("basically")),
            ("rewriting.auto", json!(true)),
            ("rewriting.tone", json!("concise")),
            ("rewriting.formatting", json!(false)),
            ("rewriting.instructions", json!("Keep greetings.")),
            ("meetings.retention", json!("after_transcription")),
            ("meetings.stopKey", json!("right shift")),
            ("meetings.screen", json!("off")),
            ("meetings.liveSpeakers", json!(false)),
            ("ui.language", json!("nl")),
            ("ui.theme", json!("dark")),
            ("updates.check", json!(true)),
        ];
        for (key, value) in &cases {
            set(&mut cfg, key, value.clone()).unwrap_or_else(|e| panic!("{key}: {e}"));
        }
        let view = view(&cfg, false, false);
        for (key, value) in cases {
            let (group, field) = key.split_once('.').unwrap();
            assert_eq!(view[group][field], value, "{key}");
        }
        assert_eq!(cfg.hotkey, "f15");
        assert_eq!(cfg.meetings_retention, Retention::AfterTranscription);
        assert!(!cfg.meetings_live_speakers);
        assert!(
            !cfg.meetings_auto_speakers,
            "Ownkey's after-Stop flag isn't the switch"
        );
    }

    #[test]
    fn the_speaker_switch_ignores_ownkey_s_after_stop_flag() {
        let mut cfg = Config::defaults(true);
        cfg.meetings_auto_speakers = true;
        cfg.meetings_live_speakers = false;
        assert_eq!(view(&cfg, false, false)["meetings"]["liveSpeakers"], false);
        set(&mut cfg, "meetings.liveSpeakers", json!(true)).unwrap();
        assert!(cfg.meetings_live_speakers);
        set(&mut cfg, "meetings.liveSpeakers", json!(false)).unwrap();
        assert!(!cfg.meetings_live_speakers);
        assert!(cfg.meetings_auto_speakers, "the import's value stays");
    }

    #[test]
    fn bad_values_and_read_only_keys_are_refused() {
        let mut cfg = Config::defaults(true);
        for (key, value) in [
            ("dictation.hotkey", json!("left alt")),
            ("dictation.hotkey", json!("off")),
            ("dictation.pasteMode", json!("yes")),
            ("local.idleMinutes", json!(1441)),
            ("local.idleMinutes", json!(-1)),
            ("local.idleMinutes", json!(2.5)),
            ("fillers.languages", json!(["en", "xx"])),
            ("rewriting.tone", json!("angry")),
            ("ui.theme", json!("blue")),
            ("meetings.retention", json!("forever")),
            ("dictionary.corrections", json!([{"from": "a"}])),
            ("transcription.provider", json!("openai")),
            ("rewriting.provider", json!("openai")),
            ("nonsense", json!(1)),
        ] {
            let error = set(&mut cfg, key, value).unwrap_err();
            assert_eq!(error.code, "invalid_value", "{key}");
        }
        assert_eq!(cfg, Config::defaults(true), "nothing changed");
    }

    #[test]
    fn only_keys_this_keyboard_has_can_be_chosen() {
        let mut cfg = Config::defaults(true);
        if cfg!(target_os = "macos") {
            for key in ["pause", "scroll lock"] {
                assert_eq!(
                    set(&mut cfg, "dictation.hotkey", json!(key))
                        .unwrap_err()
                        .code,
                    "invalid_value",
                    "{key} isn't on a Mac keyboard"
                );
            }
            set(&mut cfg, "dictation.hotkey", json!("right command")).unwrap();
            assert_eq!(cfg.hotkey, "right command");
        } else {
            assert_eq!(
                set(&mut cfg, "dictation.hotkey", json!("right command"))
                    .unwrap_err()
                    .code,
                "invalid_value",
                "Windows has no Right Command"
            );
            set(&mut cfg, "dictation.hotkey", json!("pause")).unwrap();
            assert_eq!(cfg.hotkey, "pause");
        }
    }

    #[test]
    fn a_key_can_only_have_one_role() {
        let mut cfg = Config::defaults(true);
        assert_eq!(
            set(&mut cfg, "dictation.hotkey", json!("f14"))
                .unwrap_err()
                .code,
            "same_key"
        );
        assert_eq!(
            set(&mut cfg, "dictation.rewriteHotkey", json!("f13"))
                .unwrap_err()
                .code,
            "same_key"
        );
        assert_eq!(
            set(&mut cfg, "meetings.stopKey", json!("F14"))
                .unwrap_err()
                .code,
            "same_key"
        );
        set(&mut cfg, "meetings.stopKey", json!("f15")).unwrap();
        assert_eq!(
            set(&mut cfg, "dictation.hotkey", json!("f15"))
                .unwrap_err()
                .code,
            "same_key"
        );
        set(&mut cfg, "dictation.rewriteHotkey", json!("off")).unwrap();
        set(&mut cfg, "dictation.hotkey", json!("f14")).unwrap();
    }

    #[test]
    fn keys_and_the_bucket_secret_never_reach_the_page() {
        let mut cfg = Config::defaults(true);
        cfg.audio_api_key = "sk-secret-audio".into();
        cfg.rewrite_api_key = "sk-secret-rewrite".into();
        // The secret never goes through settings_set (settings_sync_save_secret keeps it).
        let refused = set(&mut cfg, "storage.sync.secret", json!("bucket-secret")).unwrap_err();
        assert_eq!(refused.code, "invalid_value");
        assert!(refused.message.contains("settings_sync_save_secret"));
        // One from before, still in config.json, never reaches the page either.
        cfg.extra.insert(
            crate::sync::SYNC_KEY.into(),
            json!({"secret": "bucket-secret"}),
        );
        set(&mut cfg, "storage.sync.bucket", json!("meetings")).unwrap();
        let text = view(&cfg, false, true).to_string();
        for secret in ["sk-secret-audio", "sk-secret-rewrite", "bucket-secret"] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
        let view = view(&cfg, false, true);
        assert_eq!(view["transcription"]["keySaved"], true);
        assert_eq!(view["storage"]["sync"]["bucket"], "meetings");
        assert_eq!(view["storage"]["sync"]["secret"], "");
        assert_eq!(view["storage"]["sync"]["secretSaved"], true);
    }

    #[test]
    fn sync_is_off_by_default_and_needs_a_passed_test() {
        let mut cfg = Config::defaults(true);
        let sync = &view(&cfg, false, false)["storage"]["sync"];
        assert_eq!(sync["enabled"], false);
        assert_eq!(sync["service"], "upcloud");
        assert_eq!(sync["retention"], "keep");
        assert_eq!(sync["tested"], false);
        assert_eq!(sync["secretSaved"], false);
        assert_eq!(
            set(&mut cfg, "storage.sync.enabled", json!(true))
                .unwrap_err()
                .code,
            "needs_test"
        );
        // Turning it off always works.
        set(&mut cfg, "storage.sync.enabled", json!(false)).unwrap();
        assert_eq!(cfg.extra[crate::sync::SYNC_KEY]["enabled"], false);
    }

    #[test]
    fn sync_fields_are_checked() {
        let mut cfg = Config::defaults(true);
        for (key, value) in [
            ("storage.sync.service", json!("s3")),
            ("storage.sync.service", json!(1)),
            ("storage.sync.retention", json!("days30")),
            ("storage.sync.enabled", json!("yes")),
            ("storage.sync.bucket", json!(null)),
            ("storage.sync.urlStyle", json!("path")),
            ("storage.sync.tested", json!("0000")),
            ("storage.sync.nonsense", json!("x")),
        ] {
            let error = set(&mut cfg, key, value).unwrap_err();
            assert_eq!(error.code, "invalid_value", "{key}");
        }
        for (key, value) in [
            ("storage.sync.service", json!("r2")),
            ("storage.sync.retention", json!("days90")),
            ("storage.sync.region", json!(" auto ")),
            ("storage.sync.bucket", json!("meetings")),
            ("storage.sync.prefix", json!("/voice/")),
            ("storage.sync.accessKeyId", json!("AKIA")),
            ("storage.sync.endpoint", json!("")),
        ] {
            set(&mut cfg, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
        }
        let sync = &view(&cfg, false, false)["storage"]["sync"];
        assert_eq!(sync["service"], "r2");
        assert_eq!(sync["retention"], "days90");
        assert_eq!(sync["region"], "auto", "trimmed");
        assert_eq!(sync["prefix"], "/voice/");
    }

    /// The fields of a bucket, tested as Test connection would store it.
    fn tested_config() -> Config {
        let mut cfg = Config::defaults(true);
        for (key, value) in [
            ("storage.sync.service", "other"),
            ("storage.sync.endpoint", "http://127.0.0.1:9000"),
            ("storage.sync.region", "us-east-1"),
            ("storage.sync.bucket", "test"),
            ("storage.sync.accessKeyId", "test"),
        ] {
            set(&mut cfg, key, json!(value)).unwrap();
        }
        let sync = crate::sync::sync_value(&cfg);
        let fingerprint = voice_sync::SyncConfig::from_json(&sync)
            .unwrap()
            .fingerprint();
        cfg.extra.get_mut(crate::sync::SYNC_KEY).unwrap()["tested"] = json!(fingerprint);
        cfg
    }

    #[test]
    fn a_tested_bucket_turns_on_and_a_changed_one_pauses() {
        let mut cfg = tested_config();
        assert_eq!(view(&cfg, false, true)["storage"]["sync"]["tested"], true);
        set(&mut cfg, "storage.sync.enabled", json!(true)).unwrap();
        // The same value again changes nothing.
        set(&mut cfg, "storage.sync.bucket", json!("test")).unwrap();
        assert_eq!(cfg.extra[crate::sync::SYNC_KEY]["enabled"], true);
        // The bucket's retention isn't part of the connection.
        set(&mut cfg, "storage.sync.retention", json!("days365")).unwrap();
        assert_eq!(cfg.extra[crate::sync::SYNC_KEY]["enabled"], true);
        // Another bucket hasn't passed a test: nothing goes to it, but the switch stays on, so
        // retention keeps what hasn't reached the bucket.
        set(&mut cfg, "storage.sync.bucket", json!("other")).unwrap();
        let sync = &view(&cfg, false, true)["storage"]["sync"];
        assert_eq!(sync["enabled"], true);
        assert_eq!(sync["tested"], false);
        assert_eq!(crate::sync::sync_config(&cfg), None, "paused");
        // Off, it turns on again only after a test.
        set(&mut cfg, "storage.sync.enabled", json!(false)).unwrap();
        assert_eq!(
            set(&mut cfg, "storage.sync.enabled", json!(true))
                .unwrap_err()
                .code,
            "needs_test"
        );
        // Back to the tested bucket: its fingerprint matches again.
        set(&mut cfg, "storage.sync.bucket", json!("test")).unwrap();
        assert_eq!(cfg.extra[crate::sync::SYNC_KEY]["enabled"], false);
        set(&mut cfg, "storage.sync.enabled", json!(true)).unwrap();
        assert!(crate::sync::sync_config(&cfg).is_some());

        // A new or removed secret needs a new test; the switch stays on meanwhile.
        untest_sync(&mut cfg);
        let sync = &view(&cfg, false, true)["storage"]["sync"];
        assert_eq!(
            (&sync["enabled"], &sync["tested"]),
            (&json!(true), &json!(false))
        );
        assert_eq!(crate::sync::sync_config(&cfg), None);
    }

    #[test]
    fn an_endpoint_is_https_or_on_this_computer() {
        let mut cfg = Config::defaults(true);
        for endpoint in ["http://example.com", "ftp://example.com", "not a url"] {
            let error = set(&mut cfg, "storage.sync.endpoint", json!(endpoint)).unwrap_err();
            assert_eq!(error.code, "invalid_value", "{endpoint}");
        }
        for endpoint in [
            "https://example.upcloudobjects.com",
            "http://127.0.0.1:9000",
            "",
        ] {
            set(&mut cfg, "storage.sync.endpoint", json!(endpoint))
                .unwrap_or_else(|e| panic!("{endpoint}: {e}"));
        }
    }
}
