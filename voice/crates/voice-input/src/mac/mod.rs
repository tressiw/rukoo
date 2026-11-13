//! The macOS backend: the pasteboard, Secure Event Input, posted keys, the
//! Accessibility API, the foreground app and the physical keyboard.
//!
//! Posting keys into another app and reading its accessibility objects need
//! the Accessibility grant (Privacy & Security › Accessibility). Voice checks
//! it with `AXIsProcessTrusted`, which never prompts, and returns
//! [`InputError::Accessibility`], whose message says what to switch on, rather
//! than failing silently. Reading the physical keyboard needs no grant at all:
//! [`keys`] polls Core Graphics' HID system state.

use std::thread;
use std::time::Duration;

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;

use crate::clipboard::{self, BeforeKeys, Guarded, Keyboard, SystemClock};
use crate::target::{self, WindowInfo};
use crate::{InputError, Key};

mod ax;
mod events;
mod keys;
mod pasteboard;
mod secure;

/// The Accessibility grant, for `voice_input::accessibility_trusted`.
pub(crate) fn is_accessibility_trusted() -> bool {
    ax::is_trusted()
}

pub(crate) use pasteboard::{clear_if, copy_hidden, wait_for_clipboard_restore};

/// Why Voice can't type right now, in the order the user can act on it: a
/// secure field stops even a trusted Voice, and the permission stops an
/// untrusted one from doing anything at all.
fn check() -> Result<(), InputError> {
    secure::check()?;
    ax::check()
}

/// The physical keyboard, through Core Graphics' HID system state.
pub(crate) fn key_state() -> Result<fn(Key) -> bool, InputError> {
    keys::key_state()
}

/// Whether `key` is physically held. A key a Mac keyboard doesn't have is
/// never down.
pub(crate) fn is_key_down(key: Key) -> bool {
    keys::is_key_down(key)
}

/// Types the text with Cmd+V, keeping the clipboard as `win.rs` does: the
/// user's snapshot goes back 2 s later unless someone copied meanwhile.
pub(crate) fn paste_text(text: &str, before_keys: BeforeKeys) -> Result<(), InputError> {
    check()?;
    // Serializes the pasteboard work: the delayed restore must end before the
    // next paste or capture takes its snapshot.
    let turn = pasteboard::enter_gate();
    let pending = {
        let mut keys = events::Keys;
        let mut guarded = Guarded::new(&mut keys, before_keys);
        clipboard::paste(
            text,
            &mut pasteboard::Pasteboard,
            &mut guarded,
            &SystemClock::new(),
        )?
    };
    let target = pasteboard::target();
    // Restoring on another thread lets the caller report the insertion now.
    // The turn moves along, so the next paste or capture waits for it. If the
    // thread can't start or the restore fails, our text stays on the
    // clipboard, which is what Ownkey always did.
    let _ = thread::Builder::new()
        .name("clipboard-restore".into())
        .spawn(move || {
            let clock = SystemClock::new();
            let _ = clipboard::finish_paste(&mut pasteboard::Pasteboard, &clock, &target, &pending);
            drop(turn);
        });
    Ok(())
}

/// Types the text as Unicode keyboard events, one `CGEventKeyboardSetUnicodeString`
/// chunk at a time, with a newline as Shift+Return. It doesn't touch the
/// clipboard.
pub(crate) fn type_text(text: &str, before_keys: BeforeKeys) -> Result<(), InputError> {
    check()?;
    let mut keys = events::Keys;
    let mut guarded = Guarded::new(&mut keys, before_keys);
    events::type_text(text, &mut guarded, &SystemClock::new())
}

/// How Voice reads the focused app's selection.
pub(crate) trait Selection {
    /// The Accessibility API's answer, `None` when the focused element has no
    /// selected text or the app doesn't publish it.
    fn accessibility(&self) -> Option<String>;
    /// Cmd+C, then the pasteboard.
    fn copy(&mut self) -> Result<Option<String>, InputError>;
}

/// Reads the selection, Accessibility first and Cmd+C only when that gave
/// nothing.
///
/// The Accessibility API is first because it costs no copy: the user's
/// clipboard is never touched, and an app that remaps Cmd+C, or whose copy
/// gives more than the selection (browsers adding markup, code editors copying
/// the line), can't be wrong. Not every app publishes `AXSelectedText`,
/// though: Electron apps without the attribute, terminals, and apps whose
/// selection isn't text at all. Cmd+C still works there, as it does on
/// Windows.
fn capture_with(source: &mut dyn Selection) -> Result<Option<String>, InputError> {
    if let Some(text) = source.accessibility().filter(|text| !text.is_empty()) {
        return Ok(Some(text));
    }
    source.copy()
}

/// The focused app on this desktop.
struct Desktop<'a> {
    timeout: Duration,
    keyboard: &'a mut dyn Keyboard,
}

impl Selection for Desktop<'_> {
    fn accessibility(&self) -> Option<String> {
        ax::selected_text()
    }

    fn copy(&mut self) -> Result<Option<String>, InputError> {
        clipboard::capture(
            &mut pasteboard::Pasteboard,
            &mut *self.keyboard,
            &SystemClock::new(),
            self.timeout,
        )
    }
}

/// Captures the selection through the Accessibility API, or with Cmd+C and
/// the clipboard when the app doesn't publish it.
pub(crate) fn capture_selection(
    timeout: Duration,
    before_keys: BeforeKeys,
) -> Result<Option<String>, InputError> {
    check()?;
    let _turn = pasteboard::enter_gate();
    let mut keys = events::Keys;
    let mut guarded = Guarded::new(&mut keys, before_keys);
    let mut desktop = Desktop {
        timeout,
        keyboard: &mut guarded,
    };
    capture_with(&mut desktop)
}

/// The app in front, as AppKit names it.
struct Frontmost {
    bundle_id: Option<String>,
    name: Option<String>,
    pid: i32,
}

impl Frontmost {
    fn read() -> Option<Frontmost> {
        autoreleasepool(|_| {
            let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
            Some(Frontmost {
                bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
                name: app.localizedName().map(|name| name.to_string()),
                pid: app.processIdentifier(),
            })
        })
    }
}

/// Whether the pid an app had is still the app in front and still running.
fn is_still_in_front(pid: i32) -> bool {
    autoreleasepool(|_| {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .is_some_and(|app| app.processIdentifier() == pid && !app.isTerminated())
    })
}

/// The frontmost app and the role of its focused element, classified the way
/// `target::classify_mac` classifies them.
pub(crate) fn foreground_window() -> Option<WindowInfo> {
    let app = Frontmost::read()?;
    let role = ax::focused_role();
    Some(WindowInfo {
        // macOS has no window handle to hand out here; the pid tells apps
        // apart over time, and the bundle id names the app.
        hwnd: 0,
        process_id: app.pid as u32,
        class_name: app
            .bundle_id
            .clone()
            .or_else(|| app.name.clone())
            .unwrap_or_default(),
        focus_class: role.clone(),
        // Only Windows' caret query answers this; the AX role carries the
        // same evidence on macOS.
        has_caret: false,
        target: target::classify_mac(app.bundle_id.as_deref(), role.as_deref()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A selection whose two readers return what the test sets, and record
    /// which one ran.
    struct Fake {
        accessibility: Option<String>,
        copy: Option<&'static str>,
        copy_error: Option<InputError>,
        copied: bool,
    }

    impl Fake {
        fn new(accessibility: Option<&str>) -> Fake {
            Fake {
                accessibility: accessibility.map(str::to_string),
                copy: Some("copied"),
                copy_error: None,
                copied: false,
            }
        }
    }

    impl Selection for Fake {
        fn accessibility(&self) -> Option<String> {
            self.accessibility.clone()
        }

        fn copy(&mut self) -> Result<Option<String>, InputError> {
            self.copied = true;
            match self.copy_error.clone() {
                Some(error) => Err(error),
                None => Ok(self.copy.map(str::to_string)),
            }
        }
    }

    #[test]
    fn the_accessibility_answer_wins_and_leaves_the_clipboard_alone() {
        let mut fake = Fake::new(Some("selected words"));
        assert_eq!(
            capture_with(&mut fake),
            Ok(Some("selected words".to_string()))
        );
        assert!(!fake.copied, "no Cmd+C when the AX API answered");
    }

    #[test]
    fn an_empty_accessibility_answer_falls_back_to_cmd_c() {
        let mut fake = Fake::new(Some(""));
        assert_eq!(capture_with(&mut fake), Ok(Some("copied".to_string())));
        assert!(fake.copied);
    }

    #[test]
    fn an_app_without_the_attribute_falls_back_to_cmd_c() {
        let mut fake = Fake::new(None);
        assert_eq!(capture_with(&mut fake), Ok(Some("copied".to_string())));
        assert!(fake.copied);
    }

    #[test]
    fn a_failed_copy_is_reported() {
        let mut fake = Fake::new(None);
        fake.copy_error = Some(InputError::Blocked);
        assert_eq!(capture_with(&mut fake), Err(InputError::Blocked));
    }

    #[test]
    fn nothing_selected_is_none() {
        let mut fake = Fake::new(None);
        fake.copy = None;
        assert_eq!(capture_with(&mut fake), Ok(None));
    }

    /// These post keys and change the real clipboard, so they need a desktop
    /// nobody is using and an Accessibility grant for the test binary. Run
    /// them there with `cargo test -p voice-input -- --ignored`, with a text
    /// field focused.
    mod desktop {
        use super::super::*;
        use crate::clipboard::Clipboard;
        use crate::InsertMode;

        #[test]
        #[ignore = "needs a dedicated desktop: sends Cmd+V and changes the clipboard"]
        fn a_paste_puts_the_clipboard_back() {
            let before = pasteboard::Pasteboard
                .text()
                .map(|(text, _)| text)
                .unwrap_or_default();
            crate::insert_text("Pasted by voice-input\nsecond line", InsertMode::Paste).unwrap();
            crate::wait_for_clipboard_restore();
            let mut board = pasteboard::Pasteboard;
            assert_eq!(board.text().unwrap().0, before);
        }

        #[test]
        #[ignore = "needs a dedicated desktop: sends keys"]
        fn types_unicode_and_newlines() {
            crate::insert_text("Typed é 😀\nsecond line", InsertMode::Type).unwrap();
        }

        #[test]
        #[ignore = "needs a dedicated desktop with text selected: sends Cmd+C and changes the clipboard"]
        fn captures_the_selection_and_puts_the_clipboard_back() {
            let mut board = pasteboard::Pasteboard;
            let before = board.text().unwrap().0;
            let selection = crate::capture_selection(Duration::from_millis(500)).unwrap();
            assert!(selection.is_some_and(|text| !text.is_empty()));
            assert_eq!(board.text().unwrap().0, before);
        }

        #[test]
        #[ignore = "needs a dedicated desktop: reads the frontmost app"]
        fn reads_the_foreground_app() {
            let window = foreground_window().expect("an app is in front");
            assert_ne!(window.process_id, 0);
            assert!(!window.class_name.is_empty(), "{window:?}");
        }
    }
}
