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
//! That flags word doesn't survive everything Voice itself does. Right Command
//! copies the selection while it is held (it is the rewrite key), so Voice
//! posts Cmd+C at the press; [`crate::mac::events`] posts it at the HID event
//! tap with the chord's own flags, and its last event carries no Command flag.
//! On the same Mac, `CGEventSourceFlagsState` then read `0x20000000` for the
//! rest of a Right Command hold: no Command flag and no device bit, while
//! `CGEventSourceKeyState` still read 0x37 down. The press was followed by a
//! release about 85 ms later, in two holds of 2 to 3 seconds. Right Option,
//! which posts nothing at the press, held fine.
//!
//! So [`sides_down`] remembers that a right-hand key's side came from its
//! device bit, the part a posted chord can wipe. While that is remembered and
//! the flags word carries neither bit of the pair, the pair's shared left-hand
//! code decides: it stayed down through the whole hold. A right-hand key read
//! from its own code needs no memory, since that code kept reading down, and a
//! source that sets no device bits at all keeps the key-code answer Voice read
//! before the flags came in.
//!
//! This is deliberately not an event tap: a tap needs Input Monitoring and
//! can stop delivering, and a missed key-up must never leave recording on
//! (Ownkey #11). A release that happens while Voice itself is suspended shows
//! on the next read. One the HID state might never report, around a session
//! switch, the lock screen or sleep, is why the app also treats those as
//! releases (see `voice-app`'s session observer, whose release calls
//! [`forget_held_modifiers`]), and why dictation's watchdog stops a recording
//! whose key reads up.

use std::sync::atomic::{AtomicU64, Ordering};
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

/// One bit per pair in [`REMEMBERED`].
mod remembered {
    pub(super) const COMMAND: u64 = 1 << 0;
    pub(super) const OPTION: u64 = 1 << 1;
    pub(super) const CONTROL: u64 = 1 << 2;
    pub(super) const SHIFT: u64 = 1 << 3;
}

/// Which pairs' right-hand keys were last seen down through their device bit:
/// the side only the flags word can tell, and the one a chord Voice posts can
/// wipe out from under a hold.
///
/// It is one word for every pair, and each read settles only its own pair's
/// bit, so a read of one pair can't lose another pair's memory. The whole
/// process shares it on purpose: the hotkey poller (every 40 ms), dictation's
/// watchdog and the wait for a released key before a paste or copy all read
/// the same keys from different threads, and a reader that only looks has to
/// get the same answer as the poller. They can only disagree for as long as
/// the HID state itself takes to change between their reads.
static REMEMBERED: AtomicU64 = AtomicU64::new(0);

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
    /// `CGEventSourceFlagsState(HIDSystemState)`, which a key Voice posts at
    /// the HID event tap can overwrite while the key is still held.
    pub flags: u64,
}

/// Which sides of a modifier pair are down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sides {
    pub left: bool,
    pub right: bool,
}

/// One side-specific pair: the left-hand code a folded right-hand key reads
/// under, the two device bits, and this pair's bit in [`REMEMBERED`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SidePair {
    left_code: u16,
    masks: Masks,
    remembered: u64,
}

/// Whether each side of one modifier pair is down, from one read of the HID
/// state, and what the next read is to remember for it. This is the whole
/// decision behind [`is_key_down`], so every modifier and every combination of
/// readings can be tested without a keyboard.
///
/// The right-hand key is down when its own code reads down, or when its
/// device bit is set: a Mac that folds it onto the left code only says so
/// with that bit. A left-hand key is not down just because the left code
/// reads down, which it does while either side is held; its device bit
/// decides whenever the flags carry one of the pair's bits. A source that
/// sets no device bits at all, such as a synthesized event or a remote
/// session, leaves the key codes as the only answer, which is what Voice
/// read before the flags came in.
///
/// `remembered` is what the read before this one left: the right-hand key's
/// side came from its device bit, and the flags word can lose that bit while
/// the key stays down (see this module's documentation). While it is set and
/// the flags word carries neither bit of the pair, the pair's shared left-hand
/// code decides the right key, since that code stayed down through the whole
/// hold. The flags decide whenever they carry one of the pair's bits, so a
/// real key event, which writes them again, ends the hold.
///
/// The known limit: after such a wipe, a physical left key of the pair held
/// past the right key's release keeps the right key down, because the shared
/// code stays down and the flags word has nothing left to say. The left key's
/// own press or release is a real flags event and writes the pair's bits
/// again, which ends it; without one it ends when the left key goes up.
pub(crate) fn sides_down(readings: Readings, remembered: bool) -> (Sides, bool) {
    let pair = readings.masks.left | readings.masks.right;
    let right_bit = readings.flags & readings.masks.right != 0;
    if readings.flags & pair != 0 {
        let right = readings.right_code || right_bit;
        (
            Sides {
                left: readings.flags & readings.masks.left != 0,
                right,
            },
            right_bit,
        )
    } else if remembered {
        let right = readings.right_code || readings.left_code;
        (
            Sides {
                // The shared code is explained by the remembered right-hand
                // key, so it can't say that the left one is down as well.
                left: false,
                right,
            },
            right,
        )
    } else {
        let right = readings.right_code;
        (
            Sides {
                left: readings.left_code,
                right,
            },
            false,
        )
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
    let Some(pair) = pair(key) else {
        return read(own_code);
    };
    let readings = Readings {
        masks: pair.masks,
        right_code: read(own_code),
        left_code: read(pair.left_code),
        flags: flags(),
    };
    // One read, one decision and one settle of this pair's memory, so two
    // readers of the same key can't decide from one memory and store another.
    let bit = pair.remembered;
    let mut word = REMEMBERED.load(Ordering::SeqCst);
    loop {
        let (sides, remember) = sides_down(readings, word & bit != 0);
        let next = if remember { word | bit } else { word & !bit };
        if next == word {
            return sides.right;
        }
        match REMEMBERED.compare_exchange_weak(word, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return sides.right,
            Err(actual) => word = actual,
        }
    }
}

/// Forgets which right-hand keys were seen down through their device bit.
///
/// Call it where the app treats a held hotkey as released because its key-up
/// can be lost: a session switch, the lock screen, sleep. A key that was let go
/// of while the Mac slept then reads up at once, instead of staying down for as
/// long as its pair's shared code still reads down. A key that is still held is
/// read down, and remembered again, on the next read.
pub(crate) fn forget_held_modifiers() {
    REMEMBERED.store(0, Ordering::SeqCst);
}

/// The pair a side-specific hotkey belongs to: the left-hand code a folded
/// right-hand key reads under, and the two device bits. `None` for a key that
/// isn't a side-specific modifier.
fn pair(key: Key) -> Option<SidePair> {
    let (left_code, masks, remembered) = match key {
        Key::RightAlt => (
            mac_key::OPTION,
            Masks {
                left: masks::LEFT_OPTION,
                right: masks::RIGHT_OPTION,
            },
            remembered::OPTION,
        ),
        Key::RightCommand => (
            mac_key::COMMAND,
            Masks {
                left: masks::LEFT_COMMAND,
                right: masks::RIGHT_COMMAND,
            },
            remembered::COMMAND,
        ),
        Key::RightCtrl => (
            mac_key::CONTROL,
            Masks {
                left: masks::LEFT_CONTROL,
                right: masks::RIGHT_CONTROL,
            },
            remembered::CONTROL,
        ),
        Key::RightShift => (
            mac_key::SHIFT,
            Masks {
                left: masks::LEFT_SHIFT,
                right: masks::RIGHT_SHIFT,
            },
            remembered::SHIFT,
        ),
        _ => return None,
    };
    Some(SidePair {
        left_code,
        masks,
        remembered,
    })
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

    /// One modifier pair as a Mac has it: its left code, its right code, its
    /// two device bits and its bit of the memory, with the generic flag macOS
    /// adds while it's held.
    struct Pair {
        name: &'static str,
        left_code: u16,
        /// The right-hand hotkey, whose own code the pair reads.
        right_code: Key,
        masks: Masks,
        remembered: u64,
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
            remembered: remembered::OPTION,
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
            remembered: remembered::COMMAND,
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
            remembered: remembered::CONTROL,
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
            remembered: remembered::SHIFT,
            generic: 0x2_0000, // CGEventFlags::MaskShift
        },
    ];

    impl Pair {
        fn readings(&self, right_code: bool, left_code: bool, flags: u64) -> Readings {
            Readings {
                masks: self.masks,
                right_code,
                left_code,
                flags,
            }
        }

        /// One read with no memory behind it.
        fn sides(&self, right_code: bool, left_code: bool, flags: u64) -> Sides {
            sides_down(self.readings(right_code, left_code, flags), false).0
        }

        /// One read that carries this pair's memory from the read before it,
        /// as `is_key_down` does through the static.
        fn step(
            &self,
            remembered: &mut bool,
            right_code: bool,
            left_code: bool,
            flags: u64,
        ) -> Sides {
            let (sides, remember) =
                sides_down(self.readings(right_code, left_code, flags), *remembered);
            *remembered = remember;
            sides
        }
    }

    fn sides(left: bool, right: bool) -> Sides {
        Sides { left, right }
    }

    /// What `CGEventSourceFlagsState` read for the rest of a Right Command
    /// hold after Voice posted Cmd+C at the press: `CGEventFlags::NonCoalesced`
    /// alone, with no Command flag and none of the pair's device bits.
    const WIPED: u64 = 0x2000_0000;

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
            assert_eq!(pair.left_code, known.left_code, "{}", known.name);
            assert_eq!(pair.masks, known.masks, "{}", known.name);
            assert_eq!(pair.remembered, known.remembered, "{}", known.name);
        }
        // Each pair has a bit of its own in the memory.
        assert_eq!(
            [
                remembered::COMMAND,
                remembered::OPTION,
                remembered::CONTROL,
                remembered::SHIFT,
            ],
            [0b0001, 0b0010, 0b0100, 0b1000]
        );
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

    /// The bug found at a keyboard on macOS 27.2: Right Command copies the
    /// selection with Cmd+C while it is held, Voice posts that chord at the
    /// HID event tap, and the flags word then read `0x20000000` for the rest
    /// of the hold while the shared code stayed down. It must not read as a
    /// release, and the real release, the shared code going up, must.
    #[test]
    fn a_wiped_flags_word_does_not_end_a_held_right_key() {
        for pair in &PAIRS {
            let mut remembered = false;
            // The press, told by the device bit the keyboard sets.
            assert_eq!(
                pair.step(
                    &mut remembered,
                    false,
                    true,
                    pair.generic | pair.masks.right
                ),
                sides(false, true),
                "{} press",
                pair.name
            );
            assert!(remembered, "{} is remembered", pair.name);
            // Voice's own chord wiped the word, and 0x37, the pair's shared
            // code, still reads down.
            assert_eq!(
                pair.step(&mut remembered, false, true, WIPED),
                sides(false, true),
                "{} right after the wipe",
                pair.name
            );
            assert_eq!(
                pair.step(&mut remembered, false, true, WIPED),
                sides(false, true),
                "{} later in the hold",
                pair.name
            );
            // The real release.
            assert_eq!(
                pair.step(&mut remembered, false, false, WIPED),
                sides(false, false),
                "{} released",
                pair.name
            );
            assert!(!remembered, "{} is forgotten", pair.name);
        }
    }

    /// A wipe with nothing held, or one after both codes came up, must not
    /// invent a hold.
    #[test]
    fn a_wiped_flags_word_with_no_key_down_reads_up() {
        for pair in &PAIRS {
            let mut remembered = false;
            assert_eq!(
                pair.step(&mut remembered, false, false, WIPED),
                sides(false, false),
                "{} with no hold",
                pair.name
            );
            assert!(!remembered, "{} with no hold", pair.name);
            pair.step(
                &mut remembered,
                false,
                true,
                pair.generic | pair.masks.right,
            );
            assert_eq!(
                pair.step(&mut remembered, false, false, WIPED),
                sides(false, false),
                "{} after the release",
                pair.name
            );
            assert!(!remembered, "{} after the release", pair.name);
        }
    }

    /// The left key of the pair writes the flags word again with its own bit
    /// only. The bits decide then, so the remembered right key is up even
    /// though it is still under the finger, and a further wipe doesn't bring
    /// it back.
    #[test]
    fn a_left_key_press_after_a_wipe_ends_the_right_hold() {
        for pair in &PAIRS {
            let mut remembered = false;
            pair.step(
                &mut remembered,
                false,
                true,
                pair.generic | pair.masks.right,
            );
            assert_eq!(
                pair.step(&mut remembered, false, true, WIPED),
                sides(false, true),
                "{} wiped",
                pair.name
            );
            assert_eq!(
                pair.step(&mut remembered, false, true, pair.generic | pair.masks.left),
                sides(true, false),
                "{} left pressed",
                pair.name
            );
            assert!(!remembered, "{} forgotten at the left press", pair.name);
            assert_eq!(
                pair.step(&mut remembered, false, true, WIPED),
                sides(true, false),
                "{} left held",
                pair.name
            );
        }
    }

    /// Both keys of the pair held and then the flags word wiped: the shared
    /// code keeps the right key down. The known limit is here too — releasing
    /// only the right key, with the left still down and the word still wiped,
    /// can't be told from the readings — but the left key's own event writes
    /// the pair's bits again and ends the hold.
    #[test]
    fn a_wipe_with_both_sides_held_keeps_the_right_key_down() {
        for pair in &PAIRS {
            let both = pair.generic | pair.masks.left | pair.masks.right;
            let mut remembered = false;
            pair.step(&mut remembered, false, true, both);
            assert_eq!(
                pair.step(&mut remembered, false, true, WIPED),
                sides(false, true),
                "{} wiped with both held",
                pair.name
            );
            assert_eq!(
                pair.step(&mut remembered, false, true, WIPED),
                sides(false, true),
                "{} the known limit",
                pair.name
            );
            assert_eq!(
                pair.step(&mut remembered, false, true, pair.generic | pair.masks.left),
                sides(true, false),
                "{} the left key's own event ends it",
                pair.name
            );
            // The shared code going up ends it just as well.
            let mut remembered = false;
            pair.step(&mut remembered, false, true, both);
            pair.step(&mut remembered, false, true, WIPED);
            assert_eq!(
                pair.step(&mut remembered, false, false, WIPED),
                sides(false, false),
                "{} shared code up",
                pair.name
            );
            assert!(!remembered, "{} forgotten", pair.name);
        }
    }

    /// A source that sets no device bits, such as a synthesized event or a
    /// remote session: its own key code is the whole answer, and the left
    /// code of the pair, which is a different key there, never stands in for
    /// a right key whose own code reads up.
    #[test]
    fn a_source_without_device_bits_keeps_the_key_code_answer() {
        for pair in &PAIRS {
            let mut remembered = false;
            assert_eq!(
                pair.step(&mut remembered, true, false, pair.generic),
                sides(false, true),
                "{} press by its own code",
                pair.name
            );
            assert!(!remembered, "{} needs no memory", pair.name);
            assert_eq!(
                pair.step(&mut remembered, true, true, pair.generic),
                sides(true, true),
                "{} left joins",
                pair.name
            );
            assert_eq!(
                pair.step(&mut remembered, false, true, pair.generic),
                sides(true, false),
                "{} right up, left down",
                pair.name
            );
            assert_eq!(
                pair.step(&mut remembered, false, false, pair.generic),
                sides(false, false),
                "{} both up",
                pair.name
            );
        }
    }

    /// What a session release does to the memory: a remembered key whose
    /// flags word is wiped reads up once nothing puts it back, and a key that
    /// is still held is read down again as soon as an event writes its bit.
    #[test]
    fn forgetting_a_held_key_lets_a_wiped_flags_word_read_up() {
        for pair in &PAIRS {
            let mut remembered = false;
            pair.step(
                &mut remembered,
                false,
                true,
                pair.generic | pair.masks.right,
            );
            assert_eq!(
                pair.step(&mut remembered, false, true, WIPED),
                sides(false, true),
                "{} remembered",
                pair.name
            );
            // `forget_held_modifiers` clears the bit a session release may
            // have left behind from a key that was let go of while the Mac
            // slept.
            let mut forgotten = false;
            assert_eq!(
                pair.step(&mut forgotten, false, true, WIPED),
                sides(true, false),
                "{} after a session release",
                pair.name
            );
            assert!(!forgotten, "{} stays forgotten", pair.name);
            assert_eq!(
                pair.step(&mut forgotten, false, true, pair.generic | pair.masks.right),
                sides(false, true),
                "{} a key still held comes back",
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
        assert_eq!(pair(Key::RightAlt).map(|pair| pair.left_code), Some(0x3A));
        assert_eq!(
            pair(Key::RightCommand).map(|pair| pair.left_code),
            Some(0x37)
        );
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
