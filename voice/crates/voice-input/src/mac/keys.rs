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
//! (Ownkey #11). A release that happens while Voice itself is suspended shows
//! on the next read. One the HID state might never report, around a session
//! switch, the lock screen or sleep, is why the app also treats those as
//! releases (see `voice-app`'s session observer), and why dictation's watchdog
//! stops a recording whose key reads up.

use std::time::Duration;

use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};

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

/// How long ago any key went down on the HID system, modifiers aside: macOS reports those as
/// flag changes, not key-downs. `CGEventSourceSecondsSinceLastEventType` is meant to answer
/// without a grant, which nobody has checked at a real keyboard yet. It also counts keys an app
/// posted at the HID level.
pub(crate) fn since_key_down() -> Option<Duration> {
    duration(CGEventSource::seconds_since_last_event_type(
        CGEventSourceStateID::HIDSystemState,
        CGEventType::KeyDown,
    ))
}

/// The answer as a duration, or `None` for one that can't be a time ago.
fn duration(seconds: f64) -> Option<Duration> {
    Duration::try_from_secs_f64(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_time_ago_reads_as_one() {
        assert_eq!(duration(1.5), Some(Duration::from_millis(1500)));
        assert_eq!(duration(0.0), Some(Duration::ZERO));
        for nonsense in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(duration(nonsense), None, "{nonsense}");
        }
    }

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
