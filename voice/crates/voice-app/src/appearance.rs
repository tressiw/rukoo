//! The appearance Voice asks the system for on its windows' chrome (title bar, scrollbars, form
//! controls), and the way it hears the system's theme change.
//!
//! The pages draw themselves from the theme the engine sends. This is only about what the native
//! parts do, and about not getting in the way of the system's own changes.
//!
//! On macOS tao's `Window::set_theme` and the builder's `.theme(..)` set `NSApp.appearance`: they
//! pin the whole app. A pinned app never sees the system change, because `effectiveAppearance`
//! stays where it was put and tao only emits `ThemeChanged` when that changes. So on macOS Voice
//! never goes through tao for this. It clears `NSApp.appearance` and gives the window its own
//! `NSWindow.appearance` instead: none while Voice's theme is System, so the window follows the
//! system by itself, and Light or Dark when the user picked one.

use voice_engine::{Theme, UiOptions};

/// An appearance to ask for. `None` follows the system.
pub type Choice = Option<Theme>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Other,
}

impl Platform {
    pub const CURRENT: Platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else {
        Platform::Other
    };
}

/// What to set on the app and on a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The whole app's appearance (`NSApp.appearance`). Only macOS has one; elsewhere the app
    /// isn't touched and this stays `None`.
    pub app: Choice,
    pub window: Choice,
}

/// Voice's theme and the system's current appearance (`ui.theme`, which already is the system's
/// when the setting is System) turned into what the platform is told.
pub fn plan(platform: Platform, ui: &UiOptions) -> Plan {
    match platform {
        // The app is never pinned. A window follows the system until the user picks a theme.
        Platform::Mac => Plan {
            app: None,
            window: if ui.theme_follows_system {
                None
            } else {
                Some(ui.theme)
            },
        },
        // Windows gives each window its title bar's mode, and tao's `ThemeChanged` tells Voice
        // when the system's changes, so the window is always told the theme it shows.
        Platform::Other => Plan {
            app: None,
            window: Some(ui.theme),
        },
    }
}

/// The platform calls `apply` makes, so tests can watch them.
pub trait Chrome {
    fn set_app(&self, appearance: Choice);
    fn set_window(&self, appearance: Choice);
}

/// Carries out `plan`: the app first, so the window's own appearance is what's left.
pub fn apply(chrome: &dyn Chrome, plan: Plan) {
    chrome.set_app(plan.app);
    chrome.set_window(plan.window);
}

/// The native chrome of one Tauri window.
pub struct WindowChrome<'a>(pub &'a tauri::WebviewWindow);

#[cfg(target_os = "macos")]
impl Chrome for WindowChrome<'_> {
    fn set_app(&self, appearance: Choice) {
        let _ = self
            .0
            .run_on_main_thread(move || mac::set_app_appearance(appearance));
    }

    fn set_window(&self, appearance: Choice) {
        let _ = self.0.with_webview(move |webview| {
            // SAFETY: Tauri runs this on the main thread with the window it just handed out.
            unsafe { mac::set_window_appearance(webview.ns_window(), appearance) };
        });
    }
}

#[cfg(not(target_os = "macos"))]
impl Chrome for WindowChrome<'_> {
    fn set_app(&self, _appearance: Choice) {}

    fn set_window(&self, appearance: Choice) {
        let _ = self.0.set_theme(appearance.map(|theme| match theme {
            Theme::Dark => tauri::Theme::Dark,
            Theme::Light => tauri::Theme::Light,
        }));
    }
}

/// Gives `window` the appearance `ui` asks for. Needs no particular thread.
pub fn apply_to_window(window: &tauri::WebviewWindow, ui: &UiOptions) {
    apply(&WindowChrome(window), plan(Platform::CURRENT, ui));
}

/// Writes how the window and the app stand to the log: what was asked, and what AppKit says
/// results (`effectiveAppearance`), so a title bar that disagrees with its page shows up there.
pub fn log_state(window: &tauri::WebviewWindow, why: &'static str) {
    #[cfg(target_os = "macos")]
    {
        let _ = window.with_webview(move |webview| {
            // SAFETY: as in `set_window`.
            let line = unsafe { mac::describe(webview.ns_window()) };
            log::info!("appearance ({why}): {line}");
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, why);
}

/// Calls `on_change` when the system's appearance changes, whether Voice's windows follow it or
/// not. On macOS that is the system's `AppleInterfaceThemeChangedNotification`, which also
/// arrives for an automatic Light and Dark switch; tao's `ThemeChanged` can't be relied on for
/// it. Elsewhere tao's `ThemeChanged` is the signal and this does nothing.
///
/// Call it once, from the main thread.
pub fn watch_system(on_change: impl Fn() + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    mac::watch(on_change);
    #[cfg(not(target_os = "macos"))]
    let _ = on_change;
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAppearance, NSAppearanceCustomization, NSApplication, NSWindow};
    use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};
    use voice_engine::Theme;

    use super::Choice;

    const NOTIFICATION: &str = "AppleInterfaceThemeChangedNotification";

    fn appearance(theme: Theme) -> Option<Retained<NSAppearance>> {
        let name = match theme {
            Theme::Dark => "NSAppearanceNameDarkAqua",
            Theme::Light => "NSAppearanceNameAqua",
        };
        NSAppearance::appearanceNamed(&NSString::from_str(name))
    }

    fn name_of(appearance: &NSAppearance) -> String {
        appearance.name().to_string()
    }

    /// Main thread only.
    pub fn set_app_appearance(choice: Choice) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let wanted = choice.and_then(appearance);
        // Setting an appearance, even the same one, makes AppKit redraw every window.
        if app.appearance().is_some() || wanted.is_some() {
            app.setAppearance(wanted.as_deref());
        }
    }

    /// # Safety
    /// `ns_window` is a live `NSWindow`, and this runs on the main thread.
    pub unsafe fn set_window_appearance(ns_window: *mut c_void, choice: Choice) {
        let Some(window) = (unsafe { (ns_window as *const NSWindow).as_ref() }) else {
            return;
        };
        let wanted = choice.and_then(appearance);
        window.setAppearance(wanted.as_deref());
    }

    /// # Safety
    /// As [`set_window_appearance`].
    pub unsafe fn describe(ns_window: *mut c_void) -> String {
        let Some(mtm) = MainThreadMarker::new() else {
            return "not on the main thread".into();
        };
        let app = NSApplication::sharedApplication(mtm);
        let app_set = app.appearance().map(|a| name_of(&a));
        let app_effective = name_of(&app.effectiveAppearance());
        let Some(window) = (unsafe { (ns_window as *const NSWindow).as_ref() }) else {
            return format!("no window; app {app_set:?} -> {app_effective}");
        };
        let window_set = window.appearance().map(|a| name_of(&a));
        let window_effective = name_of(&window.effectiveAppearance());
        format!("window {window_set:?} -> {window_effective}; app {app_set:?} -> {app_effective}")
    }

    pub fn watch(on_change: impl Fn() + Send + Sync + 'static) {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| on_change());
        let center = NSDistributedNotificationCenter::defaultCenter();
        let name = NSString::from_str(NOTIFICATION);
        // SAFETY: the block is `Send + Sync`, takes the notification pointer AppKit gives it and
        // needs no queue: a distributed notification arrives on the main run loop that this
        // runs on.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
        };
        // The center keeps what it was given, and Voice listens until it quits.
        std::mem::forget(observer);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use voice_engine::{Edge, Locale};

    use super::*;

    fn ui(theme: Theme, follows_system: bool) -> UiOptions {
        UiOptions {
            theme,
            theme_follows_system: follows_system,
            locale: Locale::En,
            pill_edge: Edge::Bottom,
            hide_pill_from_capture: true,
            dev: false,
        }
    }

    #[derive(Default)]
    struct Calls(RefCell<Vec<String>>);

    impl Chrome for Calls {
        fn set_app(&self, appearance: Choice) {
            self.0.borrow_mut().push(format!("app {appearance:?}"));
        }
        fn set_window(&self, appearance: Choice) {
            self.0.borrow_mut().push(format!("window {appearance:?}"));
        }
    }

    fn calls(platform: Platform, ui: &UiOptions) -> Vec<String> {
        let chrome = Calls::default();
        apply(&chrome, plan(platform, ui));
        chrome.0.into_inner()
    }

    #[test]
    fn on_mac_the_system_theme_pins_nothing_whatever_the_system_shows() {
        for system in [Theme::Light, Theme::Dark] {
            assert_eq!(
                calls(Platform::Mac, &ui(system, true)),
                ["app None", "window None"],
                "system shows {system:?}"
            );
        }
    }

    #[test]
    fn on_mac_a_chosen_theme_is_the_windows_own_and_never_the_apps() {
        assert_eq!(
            calls(Platform::Mac, &ui(Theme::Light, false)),
            ["app None", "window Some(Light)"]
        );
        assert_eq!(
            calls(Platform::Mac, &ui(Theme::Dark, false)),
            ["app None", "window Some(Dark)"]
        );
    }

    #[test]
    fn the_app_is_cleared_before_the_window_is_set() {
        let steps = calls(Platform::Mac, &ui(Theme::Dark, false));
        assert!(steps[0].starts_with("app") && steps[1].starts_with("window"));
    }

    #[test]
    fn elsewhere_the_window_gets_the_theme_it_shows() {
        assert_eq!(
            calls(Platform::Other, &ui(Theme::Dark, true)),
            ["app None", "window Some(Dark)"]
        );
        assert_eq!(
            calls(Platform::Other, &ui(Theme::Light, false)),
            ["app None", "window Some(Light)"]
        );
    }
}
