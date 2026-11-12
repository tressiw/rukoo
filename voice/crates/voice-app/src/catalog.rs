//! The app's own text, read from the pages' catalogs (`voice/ui/i18n`), so the tray, the stop window and
//! the Settings title are translated in one place. voice/ui/API.md lists the keys under "Strings the app
//! reads from the catalogs".

use std::collections::HashMap;

use serde_json::Value;
use voice_engine::Locale;

/// Every key the app reads. A test checks that both catalogs have them.
#[cfg(test)]
pub const APP_KEYS: &[&str] = &[
    keys::STOP_BUTTON,
    keys::SCREEN_BUTTON,
    keys::SCREEN_OFF,
    keys::SCREEN_FRAMES,
    keys::SCREEN_VIDEO,
    keys::SCREEN_MONITOR,
    keys::SETTINGS_TITLE,
    keys::FOLDER_MODELS,
    keys::FOLDER_RECORDINGS,
    keys::TRAY_TOOLTIP,
    keys::TRAY_TOOLTIP_RECORDING,
    keys::TRAY_SETTINGS,
    keys::TRAY_START,
    keys::TRAY_PAUSE,
    keys::TRAY_RESUME,
    keys::TRAY_STOP,
    keys::TRAY_QUIT,
];

pub mod keys {
    pub const STOP_BUTTON: &str = "pill.stopButton";
    /// The title of the native window over the pill's screen button.
    pub const SCREEN_BUTTON: &str = "pill.screenButton";
    pub const SCREEN_OFF: &str = "app.pill.screenOff";
    pub const SCREEN_FRAMES: &str = "app.pill.screenFrames";
    pub const SCREEN_VIDEO: &str = "app.pill.screenVideo";
    pub const SCREEN_MONITOR: &str = "app.pill.screenMonitor";
    pub const SETTINGS_TITLE: &str = "settings.title";
    pub const FOLDER_MODELS: &str = "settings.model.folder";
    pub const FOLDER_RECORDINGS: &str = "settings.storage.folder";
    pub const TRAY_TOOLTIP: &str = "app.tray.tooltip";
    pub const TRAY_TOOLTIP_RECORDING: &str = "app.tray.tooltipRecording";
    pub const TRAY_SETTINGS: &str = "app.tray.settings";
    pub const TRAY_START: &str = "app.tray.startMeeting";
    pub const TRAY_PAUSE: &str = "app.tray.pauseMeeting";
    pub const TRAY_RESUME: &str = "app.tray.resumeMeeting";
    pub const TRAY_STOP: &str = "app.tray.stopMeeting";
    pub const TRAY_QUIT: &str = "app.tray.quit";
}

/// English and Dutch strings by key. Plural messages keep their `other` form; the app has no counts.
#[derive(Debug, Default)]
pub struct Catalog {
    en: HashMap<String, String>,
    nl: HashMap<String, String>,
}

impl Catalog {
    /// The catalogs built into the binary, the same files the pages load.
    pub fn embedded() -> Catalog {
        Catalog::parse(
            include_str!("../../../ui/i18n/en.json"),
            include_str!("../../../ui/i18n/nl.json"),
        )
        .expect("voice/ui/i18n catalogs are valid JSON")
    }

    /// `en` has `{ key: { message, description } }`, `nl` has `{ key: message }`; a message is a string or
    /// an object of plural forms.
    pub fn parse(en: &str, nl: &str) -> Result<Catalog, String> {
        let en: Value = serde_json::from_str(en).map_err(|e| format!("en.json: {e}"))?;
        let nl: Value = serde_json::from_str(nl).map_err(|e| format!("nl.json: {e}"))?;
        let mut catalog = Catalog::default();
        for (key, entry) in en.as_object().ok_or("en.json is not an object")? {
            if let Some(text) = entry.get("message").and_then(message_text) {
                catalog.en.insert(key.clone(), text);
            }
        }
        for (key, entry) in nl.as_object().ok_or("nl.json is not an object")? {
            if let Some(text) = message_text(entry) {
                catalog.nl.insert(key.clone(), text);
            }
        }
        Ok(catalog)
    }

    /// The string in `locale`, falling back to English.
    pub fn get(&self, locale: Locale, key: &str) -> Option<&str> {
        let local = match locale {
            Locale::Nl => self.nl.get(key),
            Locale::En => None,
        };
        local.or_else(|| self.en.get(key)).map(String::as_str)
    }

    /// Like [`Catalog::get`], but never empty: a missing key shows as itself, which a test prevents.
    pub fn text(&self, locale: Locale, key: &str) -> String {
        self.get(locale, key).unwrap_or(key).to_string()
    }
}

fn message_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(forms) => forms
            .get("other")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_the_app_reads_is_in_both_catalogs() {
        let catalog = Catalog::embedded();
        for key in APP_KEYS {
            let en = catalog
                .en
                .get(*key)
                .unwrap_or_else(|| panic!("en.json lacks {key}"));
            let nl = catalog
                .nl
                .get(*key)
                .unwrap_or_else(|| panic!("nl.json lacks {key}"));
            assert!(
                !en.trim().is_empty() && !nl.trim().is_empty(),
                "{key} is empty"
            );
        }
    }

    #[test]
    fn every_notification_title_the_engine_sends_is_in_both_catalogs() {
        let catalog = Catalog::embedded();
        let api = include_str!("../../../ui/API.md");
        for key in voice_engine::NOTIFY_TITLES {
            assert!(catalog.en.contains_key(*key), "en.json lacks {key}");
            assert!(catalog.nl.contains_key(*key), "nl.json lacks {key}");
            assert!(
                api.contains(&format!("`{key}`")),
                "API.md doesn't list {key}"
            );
        }
    }

    #[test]
    fn the_notification_bodies_with_a_catalog_string_are_in_both_catalogs() {
        let catalog = Catalog::embedded();
        let api = include_str!("../../../ui/API.md");
        let key = voice_engine::NOTIFY_SCREEN_PERMISSION;
        for text in [catalog.en.get(key), catalog.nl.get(key)] {
            let text = text.unwrap_or_else(|| panic!("a catalog lacks {key}"));
            // The message names the setting, so the user knows where to switch it on.
            assert!(text.contains("Screen") || text.contains("Scherm"), "{text}");
        }
        assert!(
            api.contains(&format!("`{key}`")),
            "API.md doesn't list {key}"
        );
    }

    #[test]
    fn api_md_lists_every_key_the_app_reads() {
        let api = include_str!("../../../ui/API.md");
        let start = api
            .find("### Strings the app reads")
            .expect("API.md section");
        let section = &api[start
            ..api[start..]
                .find("## Settings")
                .map_or(api.len(), |end| start + end)];
        for key in APP_KEYS {
            assert!(
                section.contains(&format!("`{key}`")),
                "API.md doesn't list {key}"
            );
        }
    }

    #[test]
    fn dutch_falls_back_to_english_and_plurals_use_other() {
        let catalog = Catalog::parse(
            r#"{ "a": { "message": "Apple", "description": "x" },
                 "b": { "message": "Banana", "description": "x" },
                 "n": { "message": { "one": "1 word", "other": "{count} words" }, "description": "x" } }"#,
            r#"{ "a": "Appel", "n": { "one": "1 woord", "other": "{count} woorden" } }"#,
        )
        .unwrap();
        assert_eq!(catalog.text(Locale::Nl, "a"), "Appel");
        assert_eq!(catalog.text(Locale::En, "a"), "Apple");
        assert_eq!(catalog.text(Locale::Nl, "b"), "Banana");
        assert_eq!(catalog.text(Locale::Nl, "n"), "{count} woorden");
        assert_eq!(catalog.get(Locale::En, "missing"), None);
        assert_eq!(catalog.text(Locale::En, "missing"), "missing");
    }

    #[test]
    fn the_app_strings_read_as_expected() {
        let catalog = Catalog::embedded();
        assert_eq!(
            catalog.text(Locale::En, keys::STOP_BUTTON),
            "Stop recording"
        );
        assert_eq!(
            catalog.text(Locale::Nl, keys::STOP_BUTTON),
            "Opname stoppen"
        );
        assert_eq!(
            catalog.text(Locale::En, keys::SETTINGS_TITLE),
            "Rukoo Voice Settings"
        );
        assert_eq!(
            catalog.text(Locale::Nl, keys::TRAY_QUIT),
            "Rukoo Voice afsluiten"
        );
    }

    #[test]
    fn bad_catalogs_are_refused() {
        assert!(Catalog::parse("[]", "{}").is_err());
        assert!(Catalog::parse("{}", "not json").is_err());
    }
}
