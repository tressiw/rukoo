//! The real system: the ready chime, the theme and language, Ownkey's process and free disk space.
//! Windows and macOS answer from their own settings; elsewhere these answer quietly with defaults.

use std::path::Path;
use std::time::Duration;

use crate::api::{Locale, Theme};
use crate::services::{Chime, System};

/// Ownkey keeps chimes at least this far apart (`ownkey.py:268`).
pub const CHIME_COOLDOWN: Duration = Duration::from_millis(200);

/// Puts the calling thread in the platform's background mode, so its disk and CPU yield to
/// everything else, the recorder included. Media preparation calls it first. Elsewhere it does
/// nothing.
pub fn background_thread() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        // Fails only for a thread that is in background mode already.
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
    }
    #[cfg(target_os = "macos")]
    {
        // UTILITY rather than BACKGROUND, as the sync worker needs too: BACKGROUND also throttles
        // I/O, which starves a preparation that reads a whole model or meeting from disk.
        // SAFETY: the call changes the quality of service of the calling thread and nothing else.
        let result =
            unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
        if result != 0 {
            log::debug!(
                "background thread: the system refused a lower quality of service: {result}"
            );
        }
    }
}

pub struct SystemChime;

impl Chime for SystemChime {
    #[cfg(windows)]
    fn play(&self) {
        use windows::core::w;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
        use windows::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows::Win32::UI::WindowsAndMessaging::MB_OK;
        // The user's "Asterisk" sound, without the default beep when the alias has none.
        let played = unsafe {
            PlaySoundW(
                w!("SystemAsterisk"),
                None,
                SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
            )
        };
        if !played.as_bool() {
            let _ = unsafe { MessageBeep(MB_OK) };
        }
    }

    #[cfg(target_os = "macos")]
    fn play(&self) {
        mac::play_alert();
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn play(&self) {}
}

pub struct SystemInfo;

impl System for SystemInfo {
    fn theme(&self) -> Theme {
        #[cfg(windows)]
        {
            // 0 means the user chose dark mode for apps.
            match win::read_dword(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                "AppsUseLightTheme",
            ) {
                Some(0) => Theme::Dark,
                Some(_) => Theme::Light,
                None => Theme::Light,
            }
        }
        #[cfg(target_os = "macos")]
        {
            theme_from_style(mac::interface_style().as_deref())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        Theme::Light
    }

    fn locale(&self) -> Locale {
        #[cfg(windows)]
        {
            const LANG_DUTCH: u16 = 0x13;
            let language = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
            if language & 0x3ff == LANG_DUTCH {
                return Locale::Nl;
            }
            Locale::En
        }
        #[cfg(target_os = "macos")]
        {
            locale_from_language(mac::preferred_language().as_deref())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            locale_from_language(std::env::var("LANG").ok().as_deref())
        }
    }

    fn ownkey_running(&self) -> bool {
        #[cfg(windows)]
        {
            win::process_running("ownkey.exe")
        }
        #[cfg(not(windows))]
        false
    }

    fn free_bytes(&self, path: &Path) -> Option<u64> {
        #[cfg(windows)]
        {
            win::free_bytes(path)
        }
        #[cfg(target_os = "macos")]
        {
            mac::free_bytes(path)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = path;
            None
        }
    }
}

/// What macOS' `AppleInterfaceStyle` default says: "Dark" in dark mode, and absent in light mode.
/// With an automatic appearance the system writes the style it shows now, so this follows the
/// schedule too.
#[cfg(any(target_os = "macos", test))]
fn theme_from_style(style: Option<&str>) -> Theme {
    match style {
        Some(style) if style.eq_ignore_ascii_case("dark") => Theme::Dark,
        _ => Theme::Light,
    }
}

/// The language Voice speaks, from a language tag such as the first of the user's preferred
/// languages (`nl-NL`) or `LANG` (`nl_NL.UTF-8`): Dutch for anything that starts with `nl`, else
/// English.
#[cfg(any(not(windows), test))]
fn locale_from_language(language: Option<&str>) -> Locale {
    match language {
        Some(language) if language.to_ascii_lowercase().starts_with("nl") => Locale::Nl,
        _ => Locale::En,
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod mac {
    use std::path::Path;

    use objc2_audio_toolbox::{kSystemSoundID_UserPreferredAlert, AudioServicesPlaySystemSound};
    use objc2_foundation::{NSLocale, NSString, NSUserDefaults};

    /// The alert sound the user chose in System Settings > Sound, played as a sound effect rather
    /// than as an alert, so Accessibility's "flash the screen" stays for real alerts. The system
    /// sound server plays it on its own thread, so the caller never waits for the sound.
    pub fn play_alert() {
        // SAFETY: the preferred alert is a system sound the framework owns; there is nothing to
        // release and the call answers at once.
        unsafe { AudioServicesPlaySystemSound(kSystemSoundID_UserPreferredAlert) };
    }

    /// `AppleInterfaceStyle` in the global defaults, as macOS keeps it.
    pub fn interface_style() -> Option<String> {
        let defaults = NSUserDefaults::standardUserDefaults();
        defaults
            .stringForKey(&NSString::from_str("AppleInterfaceStyle"))
            .map(|style| style.to_string())
    }

    /// The first entry of System Settings > General > Language & Region, such as `nl-NL`. An app
    /// started from Finder or at login has no `LANG`, so this is the only place it can read.
    pub fn preferred_language() -> Option<String> {
        NSLocale::preferredLanguages()
            .firstObject()
            .map(|language| language.to_string())
    }

    /// The free space of the volume `path` is on, through the `statvfs` call voice-core measures
    /// its downloads with. The folder may not exist yet; its volume is what counts.
    pub fn free_bytes(path: &Path) -> Option<u64> {
        let mut existing = path;
        while !existing.exists() {
            existing = existing.parent()?;
        }
        voice_core::local::free_space(existing)
    }
}

#[cfg(windows)]
pub(crate) mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS};
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    pub fn read_dword(key: &str, value: &str) -> Option<u32> {
        let mut data = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                &HSTRING::from(value),
                RRF_RT_REG_DWORD,
                None,
                Some(std::ptr::from_mut(&mut data).cast()),
                Some(&mut size),
            )
        };
        (status == ERROR_SUCCESS).then_some(data)
    }

    /// Reads the process list only; Voice never stops Ownkey.
    pub fn process_running(image: &str) -> bool {
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(image) {
                found = true;
                break;
            }
            more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        let _ = unsafe { CloseHandle(snapshot) };
        found
    }

    pub fn free_bytes(path: &Path) -> Option<u64> {
        // The folder may not exist yet; its drive is what counts.
        let mut existing = path;
        while !existing.exists() {
            existing = existing.parent()?;
        }
        let wide: Vec<u16> = existing
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut available = 0u64;
        unsafe {
            GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None).ok()?;
        }
        Some(available)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_interface_style_is_light() {
        assert_eq!(theme_from_style(None), Theme::Light);
        assert_eq!(theme_from_style(Some("Dark")), Theme::Dark);
        assert_eq!(theme_from_style(Some("dark")), Theme::Dark);
        // Anything else macOS might write is not a request for dark mode.
        assert_eq!(theme_from_style(Some("Light")), Theme::Light);
    }

    #[test]
    fn only_dutch_preferences_are_dutch() {
        assert_eq!(locale_from_language(Some("nl-NL")), Locale::Nl);
        assert_eq!(locale_from_language(Some("nl_BE.UTF-8")), Locale::Nl);
        assert_eq!(locale_from_language(Some("en-GB")), Locale::En);
        assert_eq!(locale_from_language(Some("de-DE")), Locale::En);
        assert_eq!(locale_from_language(Some("")), Locale::En);
        assert_eq!(locale_from_language(None), Locale::En);
    }

    /// The floors the screen process and the sync reserve compare against come from the real
    /// volume, so they have to be the volume's numbers.
    #[cfg(target_os = "macos")]
    #[test]
    fn free_bytes_matches_df_for_the_data_folder() {
        let dir = std::env::temp_dir();
        let free = SystemInfo
            .free_bytes(&dir)
            .expect("macOS reads the free space of a folder that exists");
        let output = std::process::Command::new("df")
            .arg("-k")
            .arg(&dir)
            .output()
            .expect("run df -k");
        assert!(output.status.success(), "df -k failed");
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text
            .lines()
            .rfind(|line| line.split_whitespace().count() >= 4)
            .expect("df -k has a data line");
        let available_kib: u64 = line
            .split_whitespace()
            .nth(3)
            .expect("df -k's fourth column is the space available")
            .parse()
            .expect("df -k reports KiB");
        let from_df = available_kib * 1024;
        let difference = free.abs_diff(from_df);
        assert!(
            difference * 100 <= from_df,
            "free_bytes said {free}, df -k says {from_df}"
        );
    }

    /// A folder that doesn't exist yet is measured on the volume it would be made on.
    #[cfg(target_os = "macos")]
    #[test]
    fn free_bytes_measures_the_volume_of_a_missing_folder() {
        let missing = std::env::temp_dir().join("rukoo-voice-free-bytes-not-here");
        assert!(SystemInfo.free_bytes(&missing).is_some_and(|free| free > 0));
    }

    /// The chime must not hold up the dictation that starts with it. Ignored, like voice-input's
    /// tests that use real devices: it plays the system alert sound on the machine running it.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "plays the system alert sound"]
    fn the_chime_answers_at_once() {
        let started = std::time::Instant::now();
        SystemChime.play();
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    /// Lowering the thread's quality of service is best-effort and never panics.
    #[test]
    fn background_thread_is_harmless() {
        background_thread();
    }

    /// The thread really runs at UTILITY afterwards, so a preparation or an upload yields to a
    /// recording without being starved of disk access the way BACKGROUND would.
    #[cfg(target_os = "macos")]
    #[test]
    fn background_thread_asks_for_utility() {
        let class = std::thread::spawn(|| {
            background_thread();
            let mut class = libc::qos_class_t::QOS_CLASS_DEFAULT;
            let mut priority = 0;
            // SAFETY: `pthread_self` names this thread, and both out-pointers are valid.
            let result = unsafe {
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut priority)
            };
            assert_eq!(result, 0);
            class
        })
        .join()
        .expect("the thread ran");
        assert_eq!(class as u32, libc::qos_class_t::QOS_CLASS_UTILITY as u32);
    }
}
