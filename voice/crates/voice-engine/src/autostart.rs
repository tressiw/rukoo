//! Start at login (decision 6).
//!
//! Windows writes the HKCU Run value `Rukoo Voice`. On macOS the app registers itself as a login item
//! through ServiceManagement's `SMAppService.mainAppService`, which only exists for an app bundle.
//!
//! Differs from Ownkey on purpose (ownkey-dictation.md 17.3): Ownkey re-registered itself on every
//! launch, so turning it off never stuck, and a source run overwrote the installed copy's value. Voice
//! writes the setting only while it is on, registers an installed copy once on its first start, and
//! never touches it in development mode. The engine enforces the last two; this module only reads and
//! writes the login item.

use std::sync::Arc;

use crate::services::Autostart;

/// The login item of the real computer, whatever the platform calls it.
pub fn system() -> Arc<dyn Autostart> {
    #[cfg(windows)]
    {
        Arc::new(run_value::RunValue::for_current_exe())
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::LoginItem::for_current_exe())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Arc::new(unsupported::Unsupported)
    }
}

#[cfg(windows)]
mod run_value {
    use super::Autostart;

    pub const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    pub const VALUE_NAME: &str = "Rukoo Voice";

    /// The Run value for one command line.
    pub struct RunValue {
        command: String,
    }

    impl RunValue {
        /// The running app, quoted, as the Run value's command.
        pub fn for_current_exe() -> RunValue {
            let exe = std::env::current_exe().unwrap_or_default();
            RunValue {
                command: quoted(&exe.display().to_string()),
            }
        }

        /// The Run value as written; `set` uses the field directly, so only the test reads this.
        #[cfg(test)]
        pub fn command(&self) -> &str {
            &self.command
        }
    }

    pub fn quoted(path: &str) -> String {
        format!("\"{path}\"")
    }

    impl Autostart for RunValue {
        fn is_on(&self) -> bool {
            use windows::core::HSTRING;
            use windows::Win32::Foundation::ERROR_SUCCESS;
            use windows::Win32::System::Registry::{
                RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ,
            };
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(RUN_KEY),
                    &HSTRING::from(VALUE_NAME),
                    RRF_RT_REG_SZ,
                    None,
                    None,
                    None,
                )
            };
            status == ERROR_SUCCESS
        }

        fn set(&self, on: bool) -> Result<(), String> {
            use windows::core::HSTRING;
            use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
            use windows::Win32::System::Registry::{
                RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
            };
            let status = if on {
                let wide: Vec<u16> = self
                    .command
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                unsafe {
                    RegSetKeyValueW(
                        HKEY_CURRENT_USER,
                        &HSTRING::from(RUN_KEY),
                        &HSTRING::from(VALUE_NAME),
                        REG_SZ.0,
                        Some(wide.as_ptr().cast()),
                        (wide.len() * 2) as u32,
                    )
                }
            } else {
                let status = unsafe {
                    RegDeleteKeyValueW(
                        HKEY_CURRENT_USER,
                        &HSTRING::from(RUN_KEY),
                        &HSTRING::from(VALUE_NAME),
                    )
                };
                // Already off.
                if status == ERROR_FILE_NOT_FOUND {
                    ERROR_SUCCESS
                } else {
                    status
                }
            };
            if status == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(format!(
                    "Windows refused to change Start at login (error {}).",
                    status.0
                ))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_command_is_the_quoted_exe() {
            assert_eq!(
                quoted("C:\\Program Files\\Rukoo Voice\\rukoo-voice.exe"),
                "\"C:\\Program Files\\Rukoo Voice\\rukoo-voice.exe\""
            );
            assert!(RunValue::for_current_exe().command().starts_with('"'));
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::{Path, PathBuf};

    use objc2_foundation::NSError;
    use objc2_service_management::{
        kSMErrorAlreadyRegistered, kSMErrorJobNotFound, kSMErrorLaunchDeniedByUser, SMAppService,
        SMAppServiceStatus,
    };

    use super::Autostart;

    /// Rukoo Voice's main app as a login item, through ServiceManagement.
    pub struct LoginItem {
        /// The `.app` the running program sits in. `None` runs outside a bundle, where macOS has no
        /// login item to register.
        bundle: Option<PathBuf>,
    }

    impl LoginItem {
        pub fn for_current_exe() -> LoginItem {
            LoginItem::in_exe(&std::env::current_exe().unwrap_or_default())
        }

        /// The login item of the app `exe` belongs to; nothing to register outside an `.app`.
        fn in_exe(exe: &Path) -> LoginItem {
            LoginItem {
                bundle: bundle_root(exe),
            }
        }
    }

    /// The `.app` around an executable, for `…/Rukoo Voice.app/Contents/MacOS/rukoo-voice`.
    fn bundle_root(exe: &Path) -> Option<PathBuf> {
        exe.ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .map(Path::to_path_buf)
    }

    impl Autostart for LoginItem {
        fn is_on(&self) -> bool {
            if self.bundle.is_none() {
                return false;
            }
            // `RequiresApproval` means macOS won't start Voice until the user allows it, so it is off here.
            let status = unsafe { SMAppService::mainAppService().status() };
            status == SMAppServiceStatus::Enabled
        }

        fn set(&self, on: bool) -> Result<(), String> {
            let Some(bundle) = &self.bundle else {
                return Err(
                    "Start at login needs Rukoo Voice's app bundle; this copy runs outside one."
                        .to_string(),
                );
            };
            log::info!(
                "start at login: {} {}",
                if on { "registering" } else { "unregistering" },
                bundle.display()
            );
            let service = unsafe { SMAppService::mainAppService() };
            let result = if on {
                unsafe { service.registerAndReturnError() }
            } else {
                unsafe { service.unregisterAndReturnError() }
            };
            match result {
                Ok(()) => Ok(()),
                // Already in the state the setting asks for.
                Err(error) if on && error.code() == kSMErrorAlreadyRegistered as isize => Ok(()),
                Err(error) if !on && error.code() == kSMErrorJobNotFound as isize => Ok(()),
                Err(error) => Err(refusal(on, &error)),
            }
        }
    }

    /// What Settings says when macOS won't take the login item.
    fn refusal(on: bool, error: &NSError) -> String {
        let code = error.code();
        if on && code == kSMErrorLaunchDeniedByUser as isize {
            return "macOS didn't allow Rukoo Voice to start at login. Allow it in System Settings > \
                    General > Login Items & Extensions."
                .to_string();
        }
        let action = if on { "add" } else { "remove" };
        format!(
            "macOS couldn't {action} Rukoo Voice in Login Items (ServiceManagement error {code}: {}).",
            error.localizedDescription()
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_bundle_is_the_app_around_the_executable() {
            assert_eq!(
                bundle_root(Path::new(
                    "/Applications/Rukoo Voice.app/Contents/MacOS/rukoo-voice"
                )),
                Some(PathBuf::from("/Applications/Rukoo Voice.app"))
            );
            assert_eq!(
                bundle_root(Path::new(
                    "/Users/x/tmp/Rukoo Voice Dev.app/Contents/MacOS/rukoo-voice"
                )),
                Some(PathBuf::from("/Users/x/tmp/Rukoo Voice Dev.app"))
            );
        }

        #[test]
        fn an_unbundled_copy_has_no_login_item() {
            let item = LoginItem::in_exe(Path::new("/Users/x/voice/target/debug/rukoo-voice"));
            assert_eq!(item.bundle, None);
            // The state is off and turning it on explains why, without asking macOS.
            assert!(!item.is_on());
            assert!(item
                .set(true)
                .unwrap_err()
                .contains("needs Rukoo Voice's app bundle"));
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod unsupported {
    use super::Autostart;

    pub struct Unsupported;

    impl Autostart for Unsupported {
        fn is_on(&self) -> bool {
            false
        }

        fn set(&self, _on: bool) -> Result<(), String> {
            Err("Start at login isn't supported on this platform yet.".into())
        }
    }
}
