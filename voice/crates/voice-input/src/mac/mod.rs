//! The macOS backend: the pasteboard, Secure Event Input and the stubs that
//! step 2 fills in after spike S1.
//!
//! The general pasteboard needs no permission, so it works now. Reading
//! physical keys, posting Cmd+V or typed keys, capturing a selection and
//! reading the foreground app all need Input Monitoring or Accessibility and
//! wait for step 2.

use std::time::Duration;

use crate::clipboard::BeforeKeys;
use crate::target::WindowInfo;
use crate::{InputError, Key};

mod pasteboard;
mod secure;

pub(crate) use pasteboard::{clear_if, copy_hidden, wait_for_clipboard_restore};

/// Comes with step 2 after spike S1: reads the physical keyboard.
pub(crate) fn key_state() -> Result<fn(Key) -> bool, InputError> {
    crate::unsupported::key_state()
}

/// Comes with step 2 after spike S1: reads the physical keyboard.
pub(crate) fn is_key_down(key: Key) -> bool {
    crate::unsupported::is_key_down(key)
}

/// Comes with step 2 after spike S1: types the text with Cmd+V.
pub(crate) fn paste_text(text: &str, before_keys: BeforeKeys) -> Result<(), InputError> {
    // A secure field already refuses keys, which is why the user needs to see
    // that reason rather than "not supported".
    secure::check()?;
    crate::unsupported::paste_text(text, before_keys)
}

/// Comes with step 2 after spike S1: types the text one key at a time.
pub(crate) fn type_text(text: &str, before_keys: BeforeKeys) -> Result<(), InputError> {
    crate::unsupported::type_text(text, before_keys)
}

/// Comes with step 2 after spike S1: captures the selection through the
/// Accessibility API or Cmd+C.
pub(crate) fn capture_selection(
    timeout: Duration,
    before_keys: BeforeKeys,
) -> Result<Option<String>, InputError> {
    crate::unsupported::capture_selection(timeout, before_keys)
}

/// Comes with step 2 after spike S1: reads the frontmost app and its focus.
pub(crate) fn foreground_window() -> Option<WindowInfo> {
    crate::unsupported::foreground_window()
}
