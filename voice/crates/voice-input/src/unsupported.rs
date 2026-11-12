//! The calls this crate hasn't implemented on a platform yet. Every one says
//! so instead of failing silently.
//!
//! Linux uses all of them. macOS implements the pasteboard, Secure Event
//! Input, posted keys, the selection and the foreground app itself, and
//! shares only [`key_state`] and [`is_key_down`], which read the physical
//! keyboard and aren't implemented there yet.

use crate::{InputError, Key};

#[cfg(not(target_os = "macos"))]
use std::time::Duration;

#[cfg(not(target_os = "macos"))]
use crate::clipboard::BeforeKeys;
#[cfg(not(target_os = "macos"))]
use crate::target::WindowInfo;

pub(crate) fn key_state() -> Result<fn(Key) -> bool, InputError> {
    Err(InputError::Unsupported)
}

pub(crate) fn is_key_down(_key: Key) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn type_text(_text: &str, _before_keys: BeforeKeys) -> Result<(), InputError> {
    Err(InputError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn paste_text(_text: &str, _before_keys: BeforeKeys) -> Result<(), InputError> {
    Err(InputError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
pub(crate) fn foreground_window() -> Option<WindowInfo> {
    None
}
