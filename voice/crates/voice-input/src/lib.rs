//! Input for Rukoo Voice dictation: push-to-talk hotkeys, text insertion,
//! selection capture and the focused window.
//!
//! Windows has the full backend. macOS has the pasteboard and Secure Event
//! Input for step 2; its keys, typed text, selection capture and foreground
//! window still return [`InputError::Unsupported`]. Other platforms return
//! [`InputError::Unsupported`] for everything; the platform-neutral parts
//! ([`HotkeyPoller::spawn_with`], [`PushToTalk`], [`target::classify`]) work
//! everywhere.
//!
//! The behavior ports Ownkey for Windows. Where Voice differs on purpose
//! (clipboard restore, Shift+Enter for newlines), the code says so.

use std::fmt;
use std::time::Duration;

#[cfg_attr(not(windows), allow(dead_code))]
mod clipboard;
mod hotkeys;
mod keys;
mod ptt;
#[cfg_attr(not(windows), allow(dead_code))]
mod strokes;
pub mod target;

// The stubs in `unsupported` are shared: macOS uses them for everything step 2
// still fills in, and Linux uses them for every call.
#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
mod win;
#[cfg(windows)]
use win as sys;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac as sys;
#[cfg(not(any(windows, target_os = "macos")))]
use unsupported as sys;

pub use clipboard::BeforeKeys;
pub use hotkeys::{HotkeyEvent, HotkeyPoller, Stopper, POLL_INTERVAL};
pub use keys::{Key, UnknownKey};
pub use ptt::{Action, HotkeyConfig, Mode, PushToTalk};
pub use target::{TextTarget, WindowInfo};

/// How [`insert_text`] gets text into the focused app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InsertMode {
    /// Through the clipboard with Ctrl+V. Ownkey's default.
    Paste,
    /// Key by key, for apps where pasting fails.
    Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputError {
    /// Not available on this platform yet.
    Unsupported,
    /// The app refuses the keys. On Windows, User Interface Privilege
    /// Isolation does that for a window of an app that runs as administrator.
    /// On macOS, Secure Event Input does it while a password field or
    /// Terminal's Secure Keyboard Entry is active.
    Blocked,
    /// Another app kept the clipboard open.
    ClipboardBusy,
    /// A clipboard call failed, with the system's message.
    Clipboard(String),
    /// Something else failed, with a message for the user.
    System(String),
}

/// Why the platform refused keys, in its own terms.
fn blocked_message() -> &'static str {
    #[cfg(windows)]
    {
        "Windows blocked typing into an app that runs as administrator."
    }
    #[cfg(target_os = "macos")]
    {
        "macOS blocks typing while a password field or Terminal's Secure Keyboard Entry is active."
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        "Typing is blocked on this platform."
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::Unsupported => {
                f.write_str("Typing and hotkeys aren't supported on this platform yet.")
            }
            InputError::Blocked => f.write_str(blocked_message()),
            InputError::ClipboardBusy => {
                f.write_str("Another app is using the clipboard. Try again.")
            }
            InputError::Clipboard(message) => write!(f, "Could not use the clipboard: {message}"),
            InputError::System(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for InputError {}

/// Inserts `text` into the focused app. Empty text does nothing.
///
/// [`InsertMode::Paste`] takes a snapshot of the clipboard, puts the text on
/// it hidden from clipboard history and cloud clipboard, and presses Ctrl+V
/// (Cmd+V on macOS, once step 2 posts keys).
/// It returns right away. A background thread waits 2 s and then puts the
/// snapshot back, unless something else was copied by then or the window the
/// keys went to doesn't answer within 1 s: an app that hasn't handled Ctrl+V
/// yet must get the dictation, not the user's old clipboard, so the
/// dictation stays on the clipboard then. The next paste or
/// [`capture_selection`] waits for that restore, and so does
/// [`wait_for_clipboard_restore`], which the app should call before quitting.
///
/// [`InsertMode::Type`] sends each character as a Unicode key event, 5 ms
/// apart, and a newline as Shift+Enter. It doesn't touch the clipboard.
///
/// The keys go to whatever has focus, with any keys the user is holding.
/// Inserting while Right Alt is held for the next recording turns Ctrl+V
/// into AltGr+V, as it did in Ownkey. [`insert_text_with`] lets the caller
/// wait for such keys right before each batch of keys goes out.
pub fn insert_text(text: &str, mode: InsertMode) -> Result<(), InputError> {
    insert_text_with(text, mode, &mut || Ok(()))
}

/// [`insert_text`], calling `before_keys` right before every batch of keys:
/// before Ctrl+V, after the wait for an earlier restore and the paste settle,
/// and before every typed character. The caller waits there while the user
/// holds a key that would change the keys, or returns an error to stop; a
/// paste then puts the user's clipboard back at once.
pub fn insert_text_with(
    text: &str,
    mode: InsertMode,
    before_keys: BeforeKeys,
) -> Result<(), InputError> {
    if text.is_empty() {
        return Ok(());
    }
    match mode {
        InsertMode::Paste => sys::paste_text(text, before_keys),
        InsertMode::Type => sys::type_text(text, before_keys),
    }
}

/// Copies the selection in the focused app with Ctrl+C and returns its text,
/// then puts the user's clipboard back.
///
/// Like Ownkey it makes two attempts, each waiting up to `timeout` for the
/// app to copy (Ownkey waited 500 ms), so the worst case is about twice the
/// timeout plus 0.2 s. `Ok(None)` when nothing with text was copied. Don't
/// call it while a key that changes Ctrl+C is held: see
/// [`Key::can_copy_while_held`] and [`capture_selection_with`].
pub fn capture_selection(timeout: Duration) -> Result<Option<String>, InputError> {
    capture_selection_with(timeout, &mut || Ok(()))
}

/// [`capture_selection`], calling `before_keys` right before each Ctrl+C, as
/// [`insert_text_with`] does.
pub fn capture_selection_with(
    timeout: Duration,
    before_keys: BeforeKeys,
) -> Result<Option<String>, InputError> {
    sys::capture_selection(timeout, before_keys)
}

/// Waits until a pending clipboard restore from [`insert_text`] has finished.
pub fn wait_for_clipboard_restore() {
    sys::wait_for_clipboard_restore();
}

/// Puts `text` on the clipboard for the user to paste somewhere else, hidden from clipboard
/// history and cloud clipboard the way [`insert_text`] hides a paste. Nothing is restored
/// afterwards: the user copied it. Waits for a pending restore of [`insert_text`] first.
///
/// For a secret the user saves, such as bucket sync's recovery code, whose copy a page's
/// `navigator.clipboard` would leave in Win+V history and, with cloud clipboard on, online.
pub fn copy_hidden(text: &str) -> Result<(), InputError> {
    sys::copy_hidden(text)
}

/// Empties the clipboard, but only while it still holds exactly `text`: something the user copied
/// since stays. Returns whether it emptied it. For [`copy_hidden`]'s text after a while.
pub fn clear_if(text: &str) -> Result<bool, InputError> {
    sys::clear_if(text)
}

/// The foreground window, or `None` when there is none (for example on the
/// secure desktop) or on other platforms.
pub fn foreground_window() -> Option<WindowInfo> {
    sys::foreground_window()
}

/// Whether the focused window looks like it takes text.
pub fn text_target() -> TextTarget {
    foreground_window().map_or(TextTarget::Unknown, |window| window.target)
}

/// Whether a key is physically held right now. Always `false` on other
/// platforms.
pub fn is_key_down(key: Key) -> bool {
    sys::is_key_down(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_inserts_nothing_in_either_mode() {
        // Returns before any platform call, so this touches nothing.
        assert_eq!(insert_text("", InsertMode::Paste), Ok(()));
        assert_eq!(insert_text("", InsertMode::Type), Ok(()));
    }

    #[test]
    fn errors_read_as_user_messages() {
        #[cfg(windows)]
        assert_eq!(
            InputError::Blocked.to_string(),
            "Windows blocked typing into an app that runs as administrator."
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            InputError::Blocked.to_string(),
            "macOS blocks typing while a password field or Terminal's Secure Keyboard Entry is active."
        );
        assert_eq!(
            InputError::Clipboard("Access is denied.".into()).to_string(),
            "Could not use the clipboard: Access is denied."
        );
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn other_platforms_say_unsupported() {
        assert_eq!(
            insert_text("x", InsertMode::Paste),
            Err(InputError::Unsupported)
        );
        assert_eq!(
            insert_text("x", InsertMode::Type),
            Err(InputError::Unsupported)
        );
        assert_eq!(
            capture_selection(Duration::from_millis(1)),
            Err(InputError::Unsupported)
        );
        assert_eq!(foreground_window(), None);
        assert_eq!(text_target(), TextTarget::Unknown);
        assert_eq!(copy_hidden("x"), Err(InputError::Unsupported));
        assert_eq!(clear_if("x"), Err(InputError::Unsupported));
        assert!(!is_key_down(Key::F13));
        let (tx, _rx) = std::sync::mpsc::channel();
        assert_eq!(
            HotkeyPoller::spawn(vec![Key::F13], tx).unwrap_err(),
            InputError::Unsupported
        );
    }

    /// macOS has the pasteboard, so `copy_hidden` and `clear_if` work; the
    /// keys and the focus do not yet. This touches no real pasteboard.
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_os_has_no_keys_typing_or_focus_yet() {
        assert_eq!(
            insert_text("x", InsertMode::Paste),
            Err(InputError::Unsupported)
        );
        assert_eq!(
            insert_text("x", InsertMode::Type),
            Err(InputError::Unsupported)
        );
        assert_eq!(
            capture_selection(Duration::from_millis(1)),
            Err(InputError::Unsupported)
        );
        assert_eq!(foreground_window(), None);
        assert_eq!(text_target(), TextTarget::Unknown);
        assert!(!is_key_down(Key::F13));
        let (tx, _rx) = std::sync::mpsc::channel();
        assert_eq!(
            HotkeyPoller::spawn(vec![Key::F13], tx).unwrap_err(),
            InputError::Unsupported
        );
    }
}
