//! The physical keyboard on macOS, read through Core Graphics.
//!
//! `CGEventSourceKeyState` with the HID system state answers whether a key
//! is held right now. It needs no permission: the spike for the port polled it
//! in an app without Input Monitoring and saw every press and release, also
//! while another app had Secure Keyboard Entry on and while a listen-only
//! event tap was stalled. Its worst release lag was about 45 ms, which is why
//! [`crate::HotkeyPoller`] keeps its 40 ms interval.
//!
//! A side-specific modifier also needs `CGEventSourceFlagsState`, because the
//! key-state table folds the right-hand modifiers onto the left-hand codes on
//! some Macs. A hands-on check on macOS 27.2 with an Apple Magic Keyboard read
//! Right Option as 0x3A (`kVK_Option`) and Right Command as 0x37
//! (`kVK_Command`), with 0x3D and 0x36 never down in either the HID state or
//! the combined session state. The device-dependent bits of the flags word,
//! `NX_DEVICERALTKEYMASK` and its neighbours in IOKit's `IOLLEvent.h`, then
//! say which side is held. [`sides_down`] decides both sides of a pair from
//! one read of that and of the pair's two key codes.
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

use crate::strokes::mac_key;
use crate::{InputError, Key};

/// The device-dependent bits that tell a modifier's two sides apart, from
/// IOKit's `IOLLEvent.h` (`NX_DEVICELALTKEYMASK`, `NX_DEVICERALTKEYMASK` and
/// their neighbours). Core Graphics' own `CGEventFlags` names only the
/// generic masks, not these.
mod masks {
    pub(super) const LEFT_CONTROL: u64 = 0x0000_0001;
    pub(super) const LEFT_SHIFT: u64 = 0x0000_0002;
    pub(super) const RIGHT_SHIFT: u64 = 0x0000_0004;
    pub(super) const LEFT_COMMAND: u64 = 0x0000_0008;
    pub(super) const RIGHT_COMMAND: u64 = 0x0000_0010;
    pub(super) const LEFT_OPTION: u64 = 0x0000_0020;
    pub(super) const RIGHT_OPTION: u64 = 0x0000_0040;
    pub(super) const RIGHT_CONTROL: u64 = 0x0000_2000;
}

/// The two device-dependent bits of one modifier pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Masks {
    /// The pair's left-hand bit (`NX_DEVICELALTKEYMASK` for Option), set
    /// while its left key is held.
    pub left: u64,
    /// The pair's right-hand bit (`NX_DEVICERALTKEYMASK` for Option), set
    /// while its right key is held.
    pub right: u64,
}

/// What one read of the HID state says about a modifier pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Readings {
    /// The pair's device bits.
    pub masks: Masks,
    /// `CGEventSourceKeyState` for the pair's right-hand code, which is the
    /// hotkey's own code (`kVK_RightOption` and friends).
    pub right_code: bool,
    /// `CGEventSourceKeyState` for the pair's left-hand code (`kVK_Option`
    /// and friends), the code a folded right-hand key reads down on.
    pub left_code: bool,
    /// `CGEventSourceFlagsState(HIDSystemState)`.
    pub flags: u64,
}

/// Which sides of a modifier pair are down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sides {
    pub left: bool,
    pub right: bool,
}

/// Whether each side of one modifier pair is down, from one read of the HID
/// state. This is the whole decision behind [`is_key_down`], so every
/// modifier and every combination of readings can be tested without a
/// keyboard.
///
/// The right-hand key is down when its own code reads down, or when its
/// device bit is set: a Mac that folds it onto the left code only says so
/// with that bit. A left-hand key is not down just because the left code
/// reads down, which it does while either side is held; its device bit
/// decides whenever the flags carry one of the pair's bits. A source that
/// sets no device bits at all, such as a synthesized event or a remote
/// session, leaves the key codes as the only answer, which is what Voice
/// read before the flags came in.
pub(crate) fn sides_down(readings: Readings) -> Sides {
    let pair = readings.masks.left | readings.masks.right;
    Sides {
        left: if readings.flags & pair != 0 {
            readings.flags & readings.masks.left != 0
        } else {
            readings.left_code
        },
        right: readings.right_code || readings.flags & readings.masks.right != 0,
    }
}

/// The function [`crate::HotkeyPoller`] polls: always available on a Mac.
pub(crate) fn key_state() -> Result<fn(Key) -> bool, InputError> {
    Ok(is_key_down)
}

/// Whether `key` is physically held. A side-specific modifier is read from
/// its own code and its device bit together, because macOS may report the
/// right-hand key under the left-hand code ([`sides_down`]). A key a Mac
/// keyboard doesn't have (Pause, Scroll Lock) is never down.
pub(crate) fn is_key_down(key: Key) -> bool {
    let Some(own_code) = key.mac_code() else {
        return false;
    };
    let Some((left_code, masks)) = pair(key) else {
        return read(own_code);
    };
    let readings = Readings {
        masks,
        right_code: read(own_code),
        left_code: read(left_code),
        flags: flags(),
    };
    sides_down(readings).right
}

/// The pair a side-specific hotkey belongs to: the left-hand code a folded
/// right-hand key reads under, and the two device bits. `None` for a key that
/// isn't a side-specific modifier.
fn pair(key: Key) -> Option<(u16, Masks)> {
    let (left_code, masks) = match key {
        Key::RightAlt => (
            mac_key::OPTION,
            Masks {
                left: masks::LEFT_OPTION,
                right: masks::RIGHT_OPTION,
            },
        ),
        Key::RightCommand => (
            mac_key::COMMAND,
            Masks {
                left: masks::LEFT_COMMAND,
                right: masks::RIGHT_COMMAND,
            },
        ),
        Key::RightCtrl => (
            mac_key::CONTROL,
            Masks {
                left: masks::LEFT_CONTROL,
                right: masks::RIGHT_CONTROL,
            },
        ),
        Key::RightShift => (
            mac_key::SHIFT,
            Masks {
                left: masks::LEFT_SHIFT,
                right: masks::RIGHT_SHIFT,
            },
        ),
        _ => return None,
    };
    Some((left_code, masks))
}

/// One read of the HID system state. Reading a key needs no grant, and the
/// call is safe from any thread.
fn read(code: u16) -> bool {
    CGEventSource::key_state(CGEventSourceStateID::HIDSystemState, code)
}

/// One read of the flags word, which carries the device-dependent bits that
/// tell a modifier's sides apart.
fn flags() -> u64 {
    CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState).bits()
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

    /// One modifier pair as a Mac has it: its left code, its right code and
    /// its two device bits, with the generic flag macOS adds while it's held.
    struct Pair {
        name: &'static str,
        left_code: u16,
        /// The right-hand hotkey, whose own code the pair reads.
        right_code: Key,
        masks: Masks,
        generic: u64,
    }

    const PAIRS: [Pair; 4] = [
        Pair {
            name: "option",
            left_code: mac_key::OPTION, // kVK_Option
            right_code: Key::RightAlt,  // kVK_RightOption
            masks: Masks {
                left: masks::LEFT_OPTION,
                right: masks::RIGHT_OPTION,
            },
            generic: 0x8_0000, // CGEventFlags::MaskAlternate
        },
        Pair {
            name: "command",
            left_code: mac_key::COMMAND,   // kVK_Command
            right_code: Key::RightCommand, // kVK_RightCommand
            masks: Masks {
                left: masks::LEFT_COMMAND,
                right: masks::RIGHT_COMMAND,
            },
            generic: 0x10_0000, // CGEventFlags::MaskCommand
        },
        Pair {
            name: "control",
            left_code: mac_key::CONTROL, // kVK_Control
            right_code: Key::RightCtrl,  // kVK_RightControl
            masks: Masks {
                left: masks::LEFT_CONTROL,
                right: masks::RIGHT_CONTROL,
            },
            generic: 0x4_0000, // CGEventFlags::MaskControl
        },
        Pair {
            name: "shift",
            left_code: mac_key::SHIFT,   // kVK_Shift
            right_code: Key::RightShift, // kVK_RightShift
            masks: Masks {
                left: masks::LEFT_SHIFT,
                right: masks::RIGHT_SHIFT,
            },
            generic: 0x2_0000, // CGEventFlags::MaskShift
        },
    ];

    impl Pair {
        fn sides(&self, right_code: bool, left_code: bool, flags: u64) -> Sides {
            sides_down(Readings {
                masks: self.masks,
                right_code,
                left_code,
                flags,
            })
        }
    }

    fn sides(left: bool, right: bool) -> Sides {
        Sides { left, right }
    }

    /// The four pairs' device bits, as IOKit's `IOLLEvent.h` defines them.
    #[test]
    fn the_device_bits_match_ioll_event_h() {
        assert_eq!(
            (
                masks::LEFT_CONTROL,
                masks::LEFT_SHIFT,
                masks::RIGHT_SHIFT,
                masks::LEFT_COMMAND,
            ),
            (0x0000_0001, 0x0000_0002, 0x0000_0004, 0x0000_0008)
        );
        assert_eq!(
            (
                masks::RIGHT_COMMAND,
                masks::LEFT_OPTION,
                masks::RIGHT_OPTION,
                masks::RIGHT_CONTROL,
            ),
            (0x0000_0010, 0x0000_0020, 0x0000_0040, 0x0000_2000)
        );
    }

    /// Every side-specific hotkey reads its pair: the left code macOS may
    /// fold it onto, and the pair's two device bits.
    #[test]
    fn every_side_specific_hotkey_reads_its_pair() {
        // The codes the keyboard has: kVK_Option, kVK_RightOption, and so on.
        assert_eq!(
            [
                (Key::RightAlt.mac_code(), mac_key::OPTION),
                (Key::RightCommand.mac_code(), mac_key::COMMAND),
                (Key::RightCtrl.mac_code(), mac_key::CONTROL),
                (Key::RightShift.mac_code(), mac_key::SHIFT),
            ],
            [
                (Some(0x3D), 0x3A),
                (Some(0x36), 0x37),
                (Some(0x3E), 0x3B),
                (Some(0x3C), 0x38),
            ]
        );
        for known in &PAIRS {
            assert!(
                Key::MACOS.contains(&known.right_code),
                "{} is a key a Mac offers",
                known.name
            );
            let pair = pair(known.right_code).expect("a side-specific hotkey has a pair");
            assert_eq!(pair.0, known.left_code, "{}", known.name);
            assert_eq!(pair.1, known.masks, "{}", known.name);
        }
        // A key that isn't a side-specific modifier asks about one code only.
        for key in [Key::F13, Key::F14, Key::F15, Key::Pause, Key::ScrollLock] {
            assert_eq!(pair(key), None, "{key}");
        }
    }

    /// Where the system reports a right-hand key under its own code, that
    /// code answers it with no device bits involved, as before.
    #[test]
    fn the_key_codes_answer_a_system_that_sets_no_device_bits() {
        for pair in &PAIRS {
            assert_eq!(
                pair.sides(true, false, pair.generic),
                sides(false, true),
                "{} right only",
                pair.name
            );
            assert_eq!(
                pair.sides(false, true, pair.generic),
                sides(true, false),
                "{} left only",
                pair.name
            );
            assert_eq!(
                pair.sides(true, true, 0),
                sides(true, true),
                "{} both, not even a generic flag",
                pair.name
            );
        }
    }

    /// The Mac that folds a right-hand key onto the left code: only its
    /// device bit says the right key is held, and the left key is not down
    /// because that shared code reads down.
    #[test]
    fn a_folded_right_key_reads_from_its_device_bit() {
        for pair in &PAIRS {
            assert_eq!(
                pair.sides(false, true, pair.generic | pair.masks.right),
                sides(false, true),
                "{} right only",
                pair.name
            );
            assert_eq!(
                pair.sides(false, true, pair.generic | pair.masks.left),
                sides(true, false),
                "{} left only",
                pair.name
            );
        }
    }

    /// Then let go of one of the two keys while the other stays down, for
    /// each order.
    #[test]
    fn the_key_that_is_let_go_is_up_and_the_other_stays_down() {
        for pair in &PAIRS {
            let both = pair.generic | pair.masks.left | pair.masks.right;
            // The right key goes first.
            assert_eq!(
                pair.sides(false, true, both),
                sides(true, true),
                "{} both down",
                pair.name
            );
            assert_eq!(
                pair.sides(false, true, pair.generic | pair.masks.left),
                sides(true, false),
                "{} right let go",
                pair.name
            );
            // The left key goes first, which the folded left code alone would
            // read as the right key still held.
            assert_eq!(
                pair.sides(false, true, pair.generic | pair.masks.right),
                sides(false, true),
                "{} left let go",
                pair.name
            );
            assert_eq!(
                pair.sides(false, false, pair.generic),
                sides(false, false),
                "{} both let go",
                pair.name
            );
            // A system that reports both codes answers the same way, from
            // the bit as well as the code.
            assert_eq!(
                pair.sides(true, true, both),
                sides(true, true),
                "{} both down, both codes reported",
                pair.name
            );
            assert_eq!(
                pair.sides(false, true, pair.generic | pair.masks.left),
                sides(true, false),
                "{} right let go, both codes reported",
                pair.name
            );
            assert_eq!(
                pair.sides(true, false, pair.generic | pair.masks.right),
                sides(false, true),
                "{} left let go, both codes reported",
                pair.name
            );
        }
    }

    /// A bit of another pair says nothing about this one: with none of these
    /// pairs' bits set, the key codes still answer.
    #[test]
    fn another_pairs_device_bits_do_not_decide_this_one() {
        let command_and_shift = 0x10_0000 | masks::RIGHT_COMMAND | masks::LEFT_SHIFT;
        for pair in &PAIRS {
            if pair.name == "command" || pair.name == "shift" {
                continue;
            }
            assert_eq!(
                pair.sides(false, true, command_and_shift),
                sides(true, false),
                "{} with those bits",
                pair.name
            );
            assert_eq!(
                pair.sides(false, false, command_and_shift),
                sides(false, false),
                "{} with those bits and no code down",
                pair.name
            );
        }
    }

    #[test]
    fn only_a_time_ago_reads_as_one() {
        assert_eq!(duration(1.5), Some(Duration::from_millis(1500)));
        assert_eq!(duration(0.0), Some(Duration::ZERO));
        for nonsense in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(duration(nonsense), None, "{nonsense}");
        }
    }

    #[test]
    fn every_mac_key_has_a_pair_or_a_code_of_its_own() {
        for key in Key::MACOS {
            assert!(key.mac_code().is_some(), "{key} has a Mac key code");
        }
        // The codes macOS' keyboard layout uses, and the left codes a Mac may
        // fold the right-hand keys onto.
        assert_eq!(Key::RightAlt.mac_code(), Some(0x3D));
        assert_eq!(Key::RightCommand.mac_code(), Some(0x36));
        assert_eq!(pair(Key::RightAlt).map(|(left, _)| left), Some(0x3A));
        assert_eq!(pair(Key::RightCommand).map(|(left, _)| left), Some(0x37));
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

    /// Reads the physical keyboard and the flags word. Run it on a Mac with
    /// nobody typing, with `cargo test -p voice-input -- --ignored`.
    #[test]
    #[ignore = "reads the real keyboard: run it where nobody is typing"]
    fn reads_the_real_keyboard_without_a_grant() {
        // F15 is on very few keyboards, so it isn't held while this runs.
        assert!(!is_key_down(Key::F15));
        // Reading twice in a row gives the same answer instead of a constant.
        assert_eq!(is_key_down(Key::RightAlt), is_key_down(Key::RightAlt));
        assert_eq!(
            is_key_down(Key::RightCommand),
            is_key_down(Key::RightCommand)
        );
    }
}
