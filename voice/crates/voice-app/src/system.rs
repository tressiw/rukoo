//! What the app reads from the system before the engine answers: the theme and language to start
//! in, and a way to say why it can't start.

use voice_engine::Locale;
#[cfg(target_os = "macos")]
use voice_engine::Theme;

/// Windows' "app mode" setting. Missing means light, Windows' default.
#[cfg(windows)]
pub fn prefers_light_theme() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_err() || value != 0
}

/// What System Settings > Appearance shows now, read from the user defaults the way the engine
/// does. Not from `NSApp.effectiveAppearance`: that only answers on the main thread, and an app
/// that was given an appearance of its own reports that one instead of the system's.
#[cfg(target_os = "macos")]
pub fn prefers_light_theme() -> bool {
    use voice_engine::services::System;
    voice_engine::SystemInfo.theme() == Theme::Light
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn prefers_light_theme() -> bool {
    true
}

/// Dutch when Windows' display language is Dutch, else English.
#[cfg(windows)]
pub fn locale() -> Locale {
    const LANG_DUTCH: u16 = 0x13;
    let language = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    if language & 0x3ff == LANG_DUTCH {
        Locale::Nl
    } else {
        Locale::En
    }
}

/// The first of the user's preferred languages, as System Settings > General > Language & Region
/// lists them. An app started from Finder or at login has no `LANG` to read.
#[cfg(target_os = "macos")]
pub fn locale() -> Locale {
    locale_from_language(mac::preferred_language().as_deref())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn locale() -> Locale {
    locale_from_language(std::env::var("LANG").ok().as_deref())
}

/// The language Voice speaks, from a language tag such as `nl-NL` or `nl_NL.UTF-8`: Dutch for
/// anything that starts with `nl`, else English.
#[cfg(any(not(windows), test))]
fn locale_from_language(language: Option<&str>) -> Locale {
    match language {
        Some(language) if language.to_ascii_lowercase().starts_with("nl") => Locale::Nl,
        _ => Locale::En,
    }
}

/// Tells the user why Voice stops. A release build has no console, so it shows a dialog; a debug build
/// writes to stderr, so tests that start it never wait on a dialog.
pub fn fail(text: &str) {
    // Once logging runs, a debug build's log already echoes to stderr.
    if log::max_level() == log::LevelFilter::Off {
        eprintln!("{text}");
    }
    log::error!("{text}");
    #[cfg(all(windows, not(debug_assertions)))]
    unsafe {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
        };
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("Rukoo Voice"),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
    #[cfg(all(target_os = "macos", not(debug_assertions)))]
    mac::show_alert(text);
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2_foundation::NSLocale;

    /// System Settings' first preferred language, such as `nl-NL`.
    pub fn preferred_language() -> Option<String> {
        NSLocale::preferredLanguages()
            .firstObject()
            .map(|language| language.to_string())
    }

    /// The release build's dialog: the same text as Windows' message box, and a process that
    /// waits for OK, because without its data folder Voice has nothing to show.
    #[cfg(not(debug_assertions))]
    pub fn show_alert(text: &str) {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSAlert, NSAlertStyle, NSApplication, NSApplicationActivationPolicy};
        use objc2_foundation::NSString;

        let Some(mtm) = MainThreadMarker::new() else {
            log::error!("can't show a start error outside the main thread");
            return;
        };
        // Before Tauri builds its windows there may be no application object; AppKit makes one,
        // and an accessory never takes the Dock icon on the way out.
        let app = NSApplication::sharedApplication(mtm);
        let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.activate();
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Rukoo Voice"));
        alert.setInformativeText(&NSString::from_str(text));
        alert.setAlertStyle(NSAlertStyle::Critical);
        alert.addButtonWithTitle(&NSString::from_str("OK"));
        alert.runModal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dutch_preferences_are_dutch() {
        assert_eq!(locale_from_language(Some("nl-NL")), Locale::Nl);
        assert_eq!(locale_from_language(Some("nl")), Locale::Nl);
        assert_eq!(locale_from_language(Some("en-NL")), Locale::En);
        assert_eq!(locale_from_language(Some("en-US")), Locale::En);
        assert_eq!(locale_from_language(None), Locale::En);
    }
}
