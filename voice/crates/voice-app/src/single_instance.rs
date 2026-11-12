//! One Voice per data folder (decision 5): a second launch asks the running one to open Settings and
//! exits. The names include the data folder, so a development copy runs next to the installed one.

use std::path::Path;

/// The mutex and event names for a data folder. Windows paths ignore case and both slashes.
///
/// Windows only, plus the test that pins the names on every platform.
#[cfg(any(windows, test))]
pub fn names(data_dir: &Path) -> (String, String) {
    let path = data_dir.to_string_lossy().replace('/', "\\").to_lowercase();
    let path = path.trim_end_matches('\\');
    // FNV-1a: stable across builds and platforms, unlike std's hasher.
    let hash = path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    (
        format!("Local\\RukooVoice-{hash:016x}"),
        format!("Local\\RukooVoice-{hash:016x}-settings"),
    )
}

pub enum Instance {
    First(Guard),
    /// Only a Windows launch finds another Voice on the same data folder; macOS waits for its lock
    /// file and socket. `main.rs` matches it on every platform, so the variant stays.
    #[cfg_attr(not(windows), allow(dead_code))]
    Running,
}

/// The first instance's mutex and event. Their handles are never closed, so the mutex stays held until
/// the process ends, however the guard itself goes.
pub struct Guard {
    #[cfg(windows)]
    event: isize,
    #[cfg(windows)]
    _mutex: isize,
}

#[cfg(windows)]
mod imp {
    use std::time::{Duration, Instant};

    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        INFINITE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

    use super::*;

    pub fn acquire(data_dir: &Path) -> Instance {
        let (mutex_name, event_name) = names(data_dir);
        unsafe {
            let mutex = match CreateMutexW(None, true, &HSTRING::from(mutex_name)) {
                Ok(mutex) => mutex,
                Err(error) => {
                    // Without the mutex there's no telling; run rather than refuse to start.
                    log::warn!("single instance: no mutex: {error}");
                    return Instance::First(Guard {
                        event: 0,
                        _mutex: 0,
                    });
                }
            };
            if GetLastError() == ERROR_ALREADY_EXISTS {
                return Instance::Running;
            }
            let event = CreateEventW(None, false, false, &HSTRING::from(event_name))
                .map(|h| h.0 as isize)
                .unwrap_or(0);
            Instance::First(Guard {
                event,
                _mutex: mutex.0 as isize,
            })
        }
    }

    /// Asks the running Voice to open Settings, and lets it come to the foreground.
    pub fn signal(data_dir: &Path) -> bool {
        let (_, event_name) = names(data_dir);
        let deadline = Instant::now() + Duration::from_secs(3);
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            loop {
                // The first instance may still be creating its event.
                if let Ok(event) = OpenEventW(
                    EVENT_MODIFY_STATE,
                    false,
                    &HSTRING::from(event_name.as_str()),
                ) {
                    let ok = SetEvent(event).is_ok();
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                    return ok;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    impl Guard {
        /// Calls `on_signal` on its own thread each time another launch signals.
        pub fn on_signal(&self, on_signal: impl Fn() + Send + 'static) {
            let event = self.event;
            if event == 0 {
                return;
            }
            let _ = std::thread::Builder::new()
                .name("single-instance".into())
                .spawn(move || loop {
                    let result = unsafe { WaitForSingleObject(HANDLE(event as *mut _), INFINITE) };
                    if result.0 != 0 {
                        return;
                    }
                    on_signal();
                });
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn acquire(_data_dir: &Path) -> Instance {
        Instance::First(Guard {})
    }

    pub fn signal(_data_dir: &Path) -> bool {
        false
    }

    impl Guard {
        pub fn on_signal(&self, _on_signal: impl Fn() + Send + 'static) {}
    }
}

pub use imp::{acquire, signal};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_ignore_case_slashes_and_a_trailing_separator() {
        let a = names(Path::new("C:\\Users\\Bart\\AppData\\Local\\Rukoo Voice"));
        assert_eq!(
            a,
            names(Path::new("c:/users/bart/appdata/local/rukoo voice/"))
        );
        assert!(a.0.starts_with("Local\\RukooVoice-") && a.1.ends_with("-settings"));
        assert_ne!(
            a,
            names(Path::new("E:\\Development\\rukoo\\tmp\\voice-dev"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_second_acquire_finds_the_first_and_signals_it() {
        let dir = std::env::temp_dir().join(format!("voice-app-single-{}", std::process::id()));
        let Instance::First(guard) = acquire(&dir) else {
            panic!("the first acquire should win");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        guard.on_signal(move || {
            let _ = tx.send(());
        });
        assert!(matches!(acquire(&dir), Instance::Running));
        assert!(signal(&dir));
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first instance heard the signal");
    }
}
