//! Posting keys and text with `CGEventPost`, the counterpart of `win.rs`'s
//! `SendInput`.
//!
//! Posting into another app needs the Accessibility permission; the caller
//! checks it first (`mac::ax`). The events come from a private event source,
//! whose key state starts empty instead of mirroring the session's, and carry
//! their own modifier flags. An event made without a source takes the
//! session's current flags, and a user who still holds Right Option or Shift
//! would turn Cmd+V into Cmd+Option+V. Whether a modifier the user physically
//! holds still reaches the app next to these flags hasn't been seen at a real
//! keyboard, so the wait `BeforeKeys` asks for is still there, as on Windows,
//! and the copy at a press only happens for keys that add no modifier
//! (`Key::can_copy_while_held`).
//!
//! The events are built here as plain data (`KeyEvent`) so the key codes and
//! flags can be checked without posting anything; `Keys` is what puts them on
//! the session.

use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGKeyCode,
};

use crate::clipboard::{Clock, Keyboard, TYPE_PACE};
use crate::strokes::{self, Stroke};
use crate::InputError;

/// The virtual key code a Unicode event carries. The text rides in the
/// event's Unicode string instead, so the code only has to be a harmless one.
const UNICODE_KEY: u16 = 0;

/// How many UTF-16 units one Unicode event carries.
/// `CGEventKeyboardSetUnicodeString` keeps at most 20, and short events are
/// more reliable across apps (some insert only the first few units of a long
/// one); 8 keeps the events rare without splitting words. A surrogate pair is
/// never split between two events.
pub(crate) const UNICODE_CHUNK: usize = 8;

/// One keyboard event Voice posts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    /// `CGKeyCode`, or [`UNICODE_KEY`] for an event that carries text.
    pub code: u16,
    pub down: bool,
    /// The modifier flags the event carries, `.MaskCommand` for Cmd+V.
    pub flags: CGEventFlags,
    /// The text the event carries, for a Unicode event.
    pub text: Option<String>,
}

/// The flags a modifier key sets while it is down.
fn modifier_mask(code: u16) -> Option<CGEventFlags> {
    Some(match code {
        strokes::mac_key::COMMAND => CGEventFlags::MaskCommand,
        strokes::mac_key::SHIFT => CGEventFlags::MaskShift,
        strokes::mac_key::CONTROL => CGEventFlags::MaskControl,
        strokes::mac_key::OPTION => CGEventFlags::MaskAlternate,
        _ => return None,
    })
}

/// The events that post `batch`. Every event carries the modifiers that are
/// down at that moment, from the batch alone: a modifier's own down event
/// carries its flag and its up event drops it again, as macOS' own
/// `flagsChanged` events do. Nothing of what the user physically holds leaks
/// in, so Cmd+V stays Cmd+V.
pub(crate) fn events(batch: &[Stroke]) -> Vec<KeyEvent> {
    let mut held = CGEventFlags::empty();
    let mut events = Vec::with_capacity(batch.len());
    for stroke in batch {
        match *stroke {
            Stroke::MacKey { code, up } => {
                let mask = modifier_mask(code).unwrap_or_else(CGEventFlags::empty);
                if up {
                    held.remove(mask);
                } else {
                    held.insert(mask);
                }
                events.push(KeyEvent {
                    code,
                    down: !up,
                    flags: held,
                    text: None,
                });
            }
            // macOS types text with `send_text`; a lone code unit can still
            // arrive here from the Windows-shaped typing strokes, and an app
            // gets it as text either way.
            Stroke::Unicode { unit, up } => events.push(KeyEvent {
                code: UNICODE_KEY,
                down: !up,
                flags: held,
                text: Some(String::from_utf16_lossy(&[unit])),
            }),
            // A Windows virtual key means nothing to macOS, and nothing here
            // builds one: `strokes::typing` is the Windows typing path.
            Stroke::Key { .. } => {}
        }
    }
    events
}

/// What one batch of typing posts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Typed {
    /// Text for one Unicode keyboard event.
    Text(String),
    /// Shift+Return.
    Newline,
}

/// The chunks one Unicode event can carry, never splitting a surrogate pair.
pub(crate) fn chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut units = 0;
    for c in text.chars() {
        let width = c.len_utf16();
        if units > 0 && units + width > UNICODE_CHUNK {
            chunks.push(std::mem::take(&mut current));
            units = 0;
        }
        current.push(c);
        units += width;
    }
    if units > 0 {
        chunks.push(current);
    }
    chunks
}

/// The batches that type `text`, with a newline as Shift+Return so a chat app
/// doesn't send the message halfway through a dictation, as on Windows, and
/// other control characters dropped as Windows drops them.
pub(crate) fn typing(text: &str) -> Vec<Typed> {
    let mut batches = Vec::new();
    let mut run = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                batches.extend(chunks(&run).into_iter().map(Typed::Text));
                run.clear();
                batches.push(Typed::Newline);
            }
            // Other control characters aren't text a text box should receive.
            c if c.is_control() && c != '\t' => {}
            c => run.push(c),
        }
    }
    batches.extend(chunks(&run).into_iter().map(Typed::Text));
    batches
}

/// Types `text` one batch at a time, [`TYPE_PACE`] apart, asking the caller's
/// key check before each batch.
pub(crate) fn type_text(
    text: &str,
    keyboard: &mut dyn Keyboard,
    clock: &dyn Clock,
) -> Result<(), InputError> {
    for batch in typing(text) {
        match batch {
            Typed::Text(chunk) => keyboard.send_text(&chunk)?,
            Typed::Newline => keyboard.send(&strokes::newline())?,
        }
        clock.sleep(TYPE_PACE);
    }
    Ok(())
}

/// The keyboard: one `CGEventPost` per event.
pub(crate) struct Keys;

impl Keyboard for Keys {
    fn send(&mut self, batch: &[Stroke]) -> Result<(), InputError> {
        let source = private_source()?;
        for event in events(batch) {
            post(&source, &event)?;
        }
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<(), InputError> {
        let source = private_source()?;
        // A down and an up event both carry the text; apps read it from the
        // down event, and the up keeps their key state honest.
        for down in [true, false] {
            post(
                &source,
                &KeyEvent {
                    code: UNICODE_KEY,
                    down,
                    flags: CGEventFlags::empty(),
                    text: Some(text.to_string()),
                },
            )?;
        }
        Ok(())
    }
}

/// A source whose key state is Voice's own, empty at the start, rather than the session's.
fn private_source() -> Result<CFRetained<CGEventSource>, InputError> {
    CGEventSource::new(CGEventSourceStateID::Private).ok_or_else(|| {
        InputError::System("macOS refused to create a keyboard event source.".into())
    })
}

/// Posts one event at the HID level, where the session's own keys arrive.
fn post(source: &CGEventSource, event: &KeyEvent) -> Result<(), InputError> {
    let Some(cg) = CGEvent::new_keyboard_event(Some(source), event.code as CGKeyCode, event.down)
    else {
        return Err(InputError::System(
            "macOS refused to create a keyboard event.".into(),
        ));
    };
    CGEvent::set_flags(Some(&cg), event.flags);
    if let Some(text) = &event.text {
        let units: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: the units live for the call, which is all macOS copies.
        unsafe {
            CGEvent::keyboard_set_unicode_string(Some(&cg), units.len() as _, units.as_ptr())
        };
    }
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&cg));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use objc2_core_graphics::CGEventFlags as Flags;

    use super::*;

    #[test]
    fn cmd_v_is_command_v_with_the_command_flag() {
        assert_eq!(
            events(&strokes::paste()),
            [
                KeyEvent {
                    code: 0x37,
                    down: true,
                    flags: Flags::MaskCommand,
                    text: None,
                },
                KeyEvent {
                    code: 0x09,
                    down: true,
                    flags: Flags::MaskCommand,
                    text: None,
                },
                KeyEvent {
                    code: 0x09,
                    down: false,
                    flags: Flags::MaskCommand,
                    text: None,
                },
                // The Command key's release carries no flag any more, as a
                // real `flagsChanged` event does.
                KeyEvent {
                    code: 0x37,
                    down: false,
                    flags: Flags::empty(),
                    text: None,
                },
            ]
        );
    }

    #[test]
    fn cmd_c_is_command_c() {
        let events = events(&strokes::copy());
        let codes: Vec<(u16, bool)> = events.iter().map(|e| (e.code, e.down)).collect();
        assert_eq!(
            codes,
            [(0x37, true), (0x08, true), (0x08, false), (0x37, false)]
        );
        assert!(events[..3]
            .iter()
            .all(|e| e.flags == Flags::MaskCommand && e.text.is_none()));
    }

    #[test]
    fn a_newline_is_shift_return() {
        let events = events(&strokes::newline());
        let flags: Vec<Flags> = events.iter().map(|e| e.flags).collect();
        assert_eq!(
            flags,
            [
                Flags::MaskShift,
                Flags::MaskShift,
                Flags::MaskShift,
                Flags::empty(),
            ]
        );
        assert_eq!(
            events.iter().map(|e| e.code).collect::<Vec<u16>>(),
            [0x38, 0x24, 0x24, 0x38]
        );
    }

    /// The point of carrying the flags: a chord is what Voice posted, whatever
    /// the user holds. The events never read the session's flags, so a held
    /// Option can't turn Cmd+V into Cmd+Option+V.
    #[test]
    fn a_chord_carries_only_its_own_modifiers() {
        for event in events(&strokes::paste()) {
            assert!(!event.flags.contains(Flags::MaskAlternate));
            assert!(!event.flags.contains(Flags::MaskControl));
            assert!(!event.flags.contains(Flags::MaskShift));
        }
    }

    #[test]
    fn chunks_stay_within_the_limit() {
        let split = chunks("abcdefghijklmnopqrstuvwxyz");
        assert_eq!(split, ["abcdefgh", "ijklmnop", "qrstuvwx", "yz"]);
        assert!(split
            .iter()
            .all(|c| c.encode_utf16().count() <= UNICODE_CHUNK));
        assert_eq!(chunks("").len(), 0);
    }

    #[test]
    fn a_surrogate_pair_is_never_split() {
        // U+1F600 is two UTF-16 units, so a chunk of 8 holds 7 of them and
        // never a lone high surrogate at its end.
        let emoji = "😀".repeat(9);
        let split = chunks(&emoji);
        assert_eq!(split.len(), 3);
        for chunk in &split {
            assert_eq!(chunk.chars().count() * 2, chunk.encode_utf16().count());
            for c in chunk.chars() {
                assert_eq!(c, '😀');
            }
        }
        assert_eq!(split.concat(), emoji);
        // Six units and a pair fill a chunk exactly.
        assert_eq!(chunks("abcdef😀"), ["abcdef😀"]);
    }

    #[test]
    fn the_batches_of_a_dictation_keep_its_text_and_newlines() {
        assert_eq!(
            typing("hé\nx"),
            [
                Typed::Text("hé".into()),
                Typed::Newline,
                Typed::Text("x".into())
            ]
        );
        assert_eq!(typing("\r\n"), [Typed::Newline]);
        assert_eq!(typing("\r"), [Typed::Newline]);
        assert_eq!(typing("\n\n"), [Typed::Newline, Typed::Newline]);
        assert_eq!(typing(""), []);
        // Long text arrives in chunks, in order.
        let long = "a".repeat(UNICODE_CHUNK * 2 + 1);
        let batches = typing(&long);
        assert_eq!(batches.len(), 3);
        let text: String = batches
            .into_iter()
            .map(|batch| match batch {
                Typed::Text(text) => text,
                Typed::Newline => unreachable!(),
            })
            .collect();
        assert_eq!(text, long);
    }

    #[test]
    fn drops_other_control_characters_and_keeps_tabs() {
        assert_eq!(typing("\u{0}\u{8}\u{1b}\u{7f}"), []);
        assert_eq!(typing("\t"), [Typed::Text("\t".into())]);
    }

    /// A keyboard that records what it was asked to post, and a clock that
    /// doesn't wait.
    #[derive(Default)]
    struct Desk {
        posted: Vec<String>,
        sleeps: u32,
    }

    struct Recording<'a>(&'a RefCell<Desk>);

    impl Keyboard for Recording<'_> {
        fn send(&mut self, batch: &[Stroke]) -> Result<(), InputError> {
            let events = events(batch);
            let codes: Vec<String> = events
                .iter()
                .map(|e| format!("{:x}{}", e.code, if e.down { "d" } else { "u" }))
                .collect();
            self.0.borrow_mut().posted.push(codes.join(" "));
            Ok(())
        }

        fn send_text(&mut self, text: &str) -> Result<(), InputError> {
            self.0.borrow_mut().posted.push(text.to_string());
            Ok(())
        }
    }

    struct NoWait<'a>(&'a RefCell<Desk>);

    impl Clock for NoWait<'_> {
        fn now(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }

        fn sleep(&self, _duration: std::time::Duration) {
            self.0.borrow_mut().sleeps += 1;
        }
    }

    #[test]
    fn the_key_check_runs_before_every_batch() {
        let desk = RefCell::new(Desk::default());
        let checks = RefCell::new(Vec::new());
        let mut check = || {
            checks.borrow_mut().push(desk.borrow().posted.len());
            Ok(())
        };
        let mut keys = Recording(&desk);
        let mut guarded = crate::clipboard::Guarded::new(&mut keys, &mut check);
        type_text("ab\nc", &mut guarded, &NoWait(&desk)).unwrap();
        assert_eq!(checks.into_inner(), [0, 1, 2], "one check per batch");
        let desk = desk.into_inner();
        assert_eq!(desk.posted, ["ab", "38d 24d 24u 38u", "c"]);
        assert_eq!(desk.sleeps, 3, "one {TYPE_PACE:?} per batch");
    }

    #[test]
    fn a_cancelled_check_posts_nothing_more() {
        let desk = RefCell::new(Desk::default());
        let mut sent = 0;
        let mut cancel_after_one = || {
            sent += 1;
            if sent > 1 {
                Err(InputError::System("Voice is quitting.".into()))
            } else {
                Ok(())
            }
        };
        let mut keys = Recording(&desk);
        let mut guarded = crate::clipboard::Guarded::new(&mut keys, &mut cancel_after_one);
        assert_eq!(
            type_text("a\nb", &mut guarded, &NoWait(&desk)),
            Err(InputError::System("Voice is quitting.".into()))
        );
        assert_eq!(desk.borrow().posted, ["a"], "the newline never went out");
    }
}
