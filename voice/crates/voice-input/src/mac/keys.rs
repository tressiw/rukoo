//! The physical keyboard on macOS, read through Core Graphics.
//!
//! `CGEventSourceKeyState` with the HID system state answers whether a key
//! is held right now. It needs no permission: the spike for the port polled it
//! in an app without Input Monitoring and saw every press and release, also
//! while another app had Secure Keyboard Entry on and while a listen-only
//! event tap was stalled. Its worst release lag was about 45 ms, which is why
//! [`crate::HotkeyPoller`] keeps its 40 ms interval.
//!
//! This is deliberately not an event tap: a tap needs Input Monitoring and
//! can stop delivering, and a missed key-up must never leave recording on
//! (Ownkey #11). What polling can't see is a release that happens while Voice
//! itself is suspended, so the app also treats a resigning session, a locked
//! screen and sleep as releases (see `voice-app`'s session observer).

use objc2_core_graphics::{CGEventSource, CGEventSourceStateID};

use crate::{InputError, Key};

/// The function [`crate::HotkeyPoller`] polls: always available on a Mac.
pub(crate) fn key_state() -> Result<fn(Key) -> bool, InputError> {
    Ok(is_key_down)
}

/// Whether `key` is physically held. A key a Mac keyboard doesn't have
/// (Pause, Scroll Lock) is never down.
pub(crate) fn is_key_down(key: Key) -> bool {
    key.mac_code().is_some_and(read)
}

/// One read of the HID system state. Reading a key needs no grant, and the
/// call is safe from any thread.
fn read(code: u16) -> bool {
    CGEventSource::key_state(CGEventSourceStateID::HIDSystemState, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mac_key_has_a_code_and_a_keyboard_without_it_does_not() {
        for key in Key::MACOS {
            assert!(key.mac_code().is_some(), "{key} has a Mac key code");
        }
        // The codes macOS' keyboard layout uses.
        assert_eq!(Key::RightAlt.mac_code(), Some(0x3D));
        assert_eq!(Key::RightCommand.mac_code(), Some(0x36));
        // A Mac keyboard has no Pause or Scroll Lock.
        for key in [Key::Pause, Key::ScrollLock] {
            assert_eq!(key.mac_code(), None);
        }
    }

    #[test]
    fn a_key_without_a_mac_code_reads_as_not_down_without_asking() {
        for key in [Key::Pause, Key::ScrollLock] {
            assert!(!is_key_down(key));
        }
    }

    /// Reads the physical keyboard. Run it on a Mac with nobody typing, with
    /// `cargo test -p voice-input -- --ignored`.
    #[test]
    #[ignore = "reads the real keyboard: run it where nobody is typing"]
    fn reads_the_real_keyboard_without_a_grant() {
        // F15 is on very few keyboards, so it isn't held while this runs.
        assert!(!is_key_down(Key::F15));
        // Reading twice in a row gives the same answer instead of a constant.
        assert_eq!(is_key_down(Key::RightAlt), is_key_down(Key::RightAlt));
    }
}
