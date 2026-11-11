//! The privacy permissions Voice needs on macOS, read without asking for them.
//!
//! macOS ties a grant to the app's code signature. An ad-hoc signed Voice (plan D4) is a new app to
//! TCC after every update, so Settings shows what the preflight calls report rather than what System
//! Settings remembers. Nothing here requests a permission: the first microphone open or event tap
//! does that.
//!
//! Windows and Linux have no such permissions and report `None` for every one, so Settings shows no
//! permission rows there.

use serde_json::{Map, Value};

/// A permission Voice needs, named as Settings lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    Microphone,
    /// The other side of a meeting, through a Core Audio process tap.
    SystemAudio,
    InputMonitoring,
    Accessibility,
    ScreenRecording,
}

impl Permission {
    /// Every permission, in Settings' order.
    pub const ALL: [Permission; 5] = [
        Permission::Microphone,
        Permission::SystemAudio,
        Permission::InputMonitoring,
        Permission::Accessibility,
        Permission::ScreenRecording,
    ];

    /// The id in `SettingsState.permissions` (voice/ui/API.md).
    pub fn id(self) -> &'static str {
        match self {
            Permission::Microphone => "microphone",
            Permission::SystemAudio => "systemAudio",
            Permission::InputMonitoring => "inputMonitoring",
            Permission::Accessibility => "accessibility",
            Permission::ScreenRecording => "screenRecording",
        }
    }
}

/// What the preflight calls say about a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    /// Switched off, or never asked and off; the Settings row says what to do.
    Denied,
    /// macOS hasn't asked yet, so there is nothing to fix.
    NotAsked,
    /// No way to tell on this system, such as System Audio Recording before a tap can be checked.
    Unknown,
}

impl PermissionState {
    /// The value in `SettingsState.permissions`.
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionState::Granted => "granted",
            PermissionState::Denied => "denied",
            PermissionState::NotAsked => "notAsked",
            PermissionState::Unknown => "unknown",
        }
    }
}

/// What the engine reads about the system's permissions.
pub trait Permissions: Send + Sync {
    /// `None` where the platform has no such permission.
    fn state(&self, permission: Permission) -> Option<PermissionState>;
}

/// `SettingsState.permissions`: every permission's state, or `None` when this platform has none.
/// A permission the service can't report is `unknown` rather than missing, so the row keeps its
/// place.
pub fn view(permissions: &dyn Permissions) -> Option<Value> {
    let states: Vec<Option<PermissionState>> = Permission::ALL
        .iter()
        .map(|permission| permissions.state(*permission))
        .collect();
    if states.iter().all(Option::is_none) {
        return None;
    }
    let mut map = Map::new();
    for (permission, state) in Permission::ALL.iter().zip(states) {
        let state = state.unwrap_or(PermissionState::Unknown);
        map.insert(permission.id().into(), Value::String(state.as_str().into()));
    }
    Some(Value::Object(map))
}

/// The computer's own permissions.
pub struct SystemPermissions;

#[cfg(target_os = "macos")]
impl Permissions for SystemPermissions {
    fn state(&self, permission: Permission) -> Option<PermissionState> {
        use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
        use objc2_core_graphics::CGPreflightScreenCaptureAccess;
        use objc2_io_kit::{IOHIDCheckAccess, IOHIDRequestType};

        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            /// Whether this app may use the Accessibility API. Unlike
            /// `AXIsProcessTrustedWithOptions`, this never prompts.
            fn AXIsProcessTrusted() -> bool;
        }

        Some(match permission {
            // The same status a first microphone open would read; it never prompts. A Settings
            // read must not panic, so a missing framework constant reads as unknown.
            Permission::Microphone => {
                let media_type = unsafe { AVMediaTypeAudio };
                media_type.map_or(PermissionState::Unknown, |media_type| {
                    let status =
                        unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
                    microphone_state(status.0)
                })
            }
            // No public preflight for a process tap's System Audio Recording grant (spike S2).
            Permission::SystemAudio => PermissionState::Unknown,
            // The one preflight with a third answer: before the first listen request it reads
            // unknown, which is "not asked yet" rather than a denial (spike S1).
            Permission::InputMonitoring => {
                input_monitoring_state(IOHIDCheckAccess(IOHIDRequestType::ListenEvent).0)
            }
            Permission::Accessibility => preflight_state(unsafe { AXIsProcessTrusted() }),
            Permission::ScreenRecording => preflight_state(CGPreflightScreenCaptureAccess()),
        })
    }
}

#[cfg(not(target_os = "macos"))]
impl Permissions for SystemPermissions {
    fn state(&self, _permission: Permission) -> Option<PermissionState> {
        None
    }
}

/// `AVAuthorizationStatus`: 0 not determined, 1 restricted, 2 denied, 3 authorized.
pub fn microphone_state(status: isize) -> PermissionState {
    match status {
        3 => PermissionState::Granted,
        0 => PermissionState::NotAsked,
        // Restricted by policy or denied by the user: either way the switch is off.
        _ => PermissionState::Denied,
    }
}

/// `IOHIDAccessType` (IOHIDCheckAccess): 0 granted, 1 denied, 2 unknown. Unknown means macOS hasn't
/// been asked for Input Monitoring yet, so Settings shows "not asked" instead of an error (spike
/// S1). A value this version doesn't know is unknown rather than a guessed denial.
pub fn input_monitoring_state(access: u32) -> PermissionState {
    match access {
        0 => PermissionState::Granted,
        1 => PermissionState::Denied,
        2 => PermissionState::NotAsked,
        _ => PermissionState::Unknown,
    }
}

/// `CGPreflightScreenCaptureAccess` and `AXIsProcessTrusted` only report allowed or not; a Mac that
/// never asked reports not allowed, so the row says what to switch on rather than guessing that the
/// user refused.
pub fn preflight_state(allowed: bool) -> PermissionState {
    if allowed {
        PermissionState::Granted
    } else {
        PermissionState::Denied
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Every permission reports what the test set.
    struct Fake(HashMap<Permission, PermissionState>);

    impl Permissions for Fake {
        fn state(&self, permission: Permission) -> Option<PermissionState> {
            self.0.get(&permission).copied()
        }
    }

    /// A platform without permissions, as Windows and Linux are.
    struct None;

    impl Permissions for None {
        fn state(&self, _permission: Permission) -> Option<PermissionState> {
            Option::None
        }
    }

    #[test]
    fn the_microphone_status_maps_to_a_state() {
        assert_eq!(microphone_state(0), PermissionState::NotAsked);
        assert_eq!(microphone_state(1), PermissionState::Denied);
        assert_eq!(microphone_state(2), PermissionState::Denied);
        assert_eq!(microphone_state(3), PermissionState::Granted);
        // A status this version doesn't know is never mistaken for granted.
        assert_eq!(microphone_state(9), PermissionState::Denied);
    }

    #[test]
    fn the_input_monitoring_access_maps_to_a_state() {
        assert_eq!(input_monitoring_state(0), PermissionState::Granted);
        assert_eq!(input_monitoring_state(1), PermissionState::Denied);
        // Unknown until the first listen request, which is not a denial (spike S1).
        assert_eq!(input_monitoring_state(2), PermissionState::NotAsked);
        assert_eq!(input_monitoring_state(9), PermissionState::Unknown);
    }

    #[test]
    fn a_failed_preflight_is_denied_and_never_unknown() {
        assert_eq!(preflight_state(true), PermissionState::Granted);
        assert_eq!(preflight_state(false), PermissionState::Denied);
    }

    #[test]
    fn the_view_lists_all_five_states() {
        let fake = Fake(HashMap::from([
            (Permission::Microphone, PermissionState::Granted),
            (Permission::SystemAudio, PermissionState::Unknown),
            (Permission::InputMonitoring, PermissionState::NotAsked),
            (Permission::Accessibility, PermissionState::Denied),
            (Permission::ScreenRecording, PermissionState::Granted),
        ]));
        assert_eq!(
            view(&fake),
            Some(serde_json::json!({
                "microphone": "granted",
                "systemAudio": "unknown",
                "inputMonitoring": "notAsked",
                "accessibility": "denied",
                "screenRecording": "granted",
            }))
        );
    }

    #[test]
    fn a_permission_the_service_cannot_read_is_unknown() {
        // A platform that reports only some permissions keeps the others' rows, as unknown.
        struct Partial;
        impl Permissions for Partial {
            fn state(&self, permission: Permission) -> Option<PermissionState> {
                (permission == Permission::Microphone).then_some(PermissionState::Granted)
            }
        }
        let view = view(&Partial).expect("the microphone is a permission");
        assert_eq!(view[Permission::Microphone.id()], "granted");
        for permission in Permission::ALL {
            if permission != Permission::Microphone {
                assert_eq!(view[permission.id()], "unknown");
            }
        }
    }

    #[test]
    fn a_platform_without_permissions_has_no_view() {
        assert_eq!(view(&None), Option::None);
    }
}
