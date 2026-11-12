//! The calls this crate hasn't implemented on a platform yet. Every one says
//! so instead of failing silently.
//!
//! Linux uses all of them. macOS implements the pasteboard itself and shares
//! the rest for step 2's key posting, typed text, selection capture and
//! foreground window.

use std::time::Duration;

use crate::clipboard::BeforeKeys;
use crate::target::WindowInfo;
use crate::{InputError, Key};

pub(crate) fn key_state() -> Result<fn(Key) -> bool, InputError> {
    Err(InputError::Unsupported)
}

pub(crate) fn is_key_down(_key: Key) -> bool {
    false
}

pub(crate) fn type_text(_text: &str, _before_keys: BeforeKeys) -> Result<(), InputError> {
    Err(InputError::Unsupported)
}

pub(crate) fn paste_text(_text: &str, _before_keys: BeforeKeys) -> Result<(), InputError> {
    Err(InputError::Unsupported)
}

pub(crate) fn capture_selection(
    _timeout: Duration,
    _before_keys: BeforeKeys,
) -> Result<Option<String>, InputError> {
    Err(InputError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn wait_for_clipboard_restore() {}

#[cfg(not(target_os = "macos"))]
pub(crate) fn copy_hidden(_text: &str) -> Result<(), InputError> {
    Err(InputError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn clear_if(_text: &str) -> Result<bool, InputError> {
    Err(InputError::Unsupported)
}

pub(crate) fn foreground_window() -> Option<WindowInfo> {
    None
}
