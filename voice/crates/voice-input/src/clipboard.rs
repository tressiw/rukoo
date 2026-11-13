//! Clipboard save and restore around a paste or a selection capture, and the
//! keys every insertion sends.
//!
//! Ownkey overwrote the clipboard on every dictation and rewrite. Voice takes
//! a snapshot of every format it can copy, puts its own text on the clipboard
//! hidden from clipboard history, and puts the snapshot back once the target
//! app has had time to read the text, unless someone copied something in the
//! meantime. This differs from Ownkey on purpose.
//!
//! Windows doesn't say when an app has read the clipboard, so the restore
//! waits [`RESTORE_AFTER`] and then asks the window the keys went to for an
//! answer. A window that doesn't answer may still paste later; Voice then
//! leaves the dictation on the clipboard, as Ownkey always did, rather than
//! risk pasting the user's old clipboard into it.
//!
//! The logic is written against small traits so it can be tested with a fake
//! clipboard; the Windows and macOS backends implement them.

use std::sync::{Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::strokes::{self, Stroke};
use crate::InputError;

/// Ownkey's "small delay so clipboard is ready" before Ctrl+V.
pub(crate) const PASTE_SETTLE: Duration = Duration::from_millis(50);
/// Apps read the clipboard after they handle Ctrl+V, which a busy app or a
/// remote desktop does late. Restoring sooner pastes the user's old content
/// instead of ours. Voice first used 600 ms, which nobody measured; this is
/// the floor, and [`TARGET_ANSWER`] covers a target that is still busy.
pub(crate) const RESTORE_AFTER: Duration = Duration::from_millis(2000);
/// How long the window the keys went to may take to answer before Voice
/// leaves the dictation on the clipboard instead of restoring.
pub(crate) const TARGET_ANSWER: Duration = Duration::from_millis(1000);
/// Ownkey's pause between typed characters.
pub(crate) const TYPE_PACE: Duration = Duration::from_millis(5);
/// Ownkey lets the foreground app settle before Ctrl+C, longer on the retry.
pub(crate) const COPY_SETTLE: [Duration; 2] =
    [Duration::from_millis(50), Duration::from_millis(150)];
/// How often a capture checks whether the app has copied.
pub(crate) const COPY_POLL: Duration = Duration::from_millis(20);

/// Standard clipboard format ids.
pub(crate) mod format {
    pub const BITMAP: u32 = 2;
    pub const METAFILEPICT: u32 = 3;
    pub const PALETTE: u32 = 9;
    pub const UNICODETEXT: u32 = 13;
    pub const ENHMETAFILE: u32 = 14;
    pub const OWNERDISPLAY: u32 = 0x80;
    pub const DSPBITMAP: u32 = 0x82;
    pub const DSPMETAFILEPICT: u32 = 0x83;
    pub const DSPENHMETAFILE: u32 = 0x8E;
    pub const PRIVATE_FIRST: u32 = 0x200;
    pub const GDIOBJ_LAST: u32 = 0x3FF;
}

/// Whether a format's data is a global memory block whose bytes can be copied
/// and put back. GDI handles, metafile pictures (which hold a handle) and
/// private formats are left out. An image survives anyway: Windows offers
/// CF_DIB next to CF_BITMAP.
pub(crate) fn holds_hglobal(id: u32) -> bool {
    use format::*;
    !matches!(
        id,
        BITMAP
            | METAFILEPICT
            | PALETTE
            | ENHMETAFILE
            | OWNERDISPLAY
            | DSPBITMAP
            | DSPMETAFILEPICT
            | DSPENHMETAFILE
    ) && !(PRIVATE_FIRST..=GDIOBJ_LAST).contains(&id)
}

/// Registered formats that OLE ties to the process that copied. Put back
/// after that process's clipboard object is gone, they would point OLE at a
/// window that no longer serves the data; without them OLE reads the plain
/// formats.
pub(crate) fn is_process_bound(name: &str) -> bool {
    matches!(name, "DataObject" | "Ole Private Data")
}

/// One clipboard format's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Format {
    pub id: u32,
    pub data: Vec<u8>,
}

/// The clipboard's formats in the order the owner offered them. Empty for an
/// empty clipboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub formats: Vec<Format>,
}

pub(crate) trait Clipboard {
    /// The user's clipboard contents in the shape this platform keeps them:
    /// Windows' formats, or the general pasteboard's items. Only the backend
    /// that took the snapshot reads it again.
    type Snapshot;

    /// `GetClipboardSequenceNumber` or `NSPasteboard.changeCount`: changes
    /// whenever the content does.
    fn sequence(&self) -> u32;
    /// Copies everything the platform can put back: Windows' formats, or the
    /// pasteboard's items and their types.
    fn snapshot(&mut self) -> Result<Self::Snapshot, InputError>;
    /// Replaces the content with `text`, hidden from clipboard history, and
    /// returns the sequence number read before the clipboard closed, which
    /// belongs to exactly that text. Read after the close, it could already
    /// belong to someone else's copy.
    fn set_text(&mut self, text: &str) -> Result<u32, InputError>;
    /// The Unicode text on the clipboard, if any, and the sequence number
    /// read while the clipboard was open, which belongs to exactly that text.
    fn text(&mut self) -> Result<(Option<String>, u32), InputError>;
    /// Replaces the content with the snapshot. On Windows that is hidden from
    /// clipboard history; on macOS the items go back exactly as they were.
    /// With `expected`, it first compares the sequence number while it holds
    /// the clipboard open (or, on macOS, as late as the pasteboard allows),
    /// so a copy made in between wins, and leaves the clipboard alone when it
    /// differs. Returns whether it restored.
    fn restore(
        &mut self,
        snapshot: &Self::Snapshot,
        expected: Option<u32>,
    ) -> Result<bool, InputError>;
}

pub(crate) trait Keyboard {
    /// Sends one batch. [`InputError::Blocked`] when Windows inserted fewer
    /// events than it was given.
    fn send(&mut self, strokes: &[Stroke]) -> Result<(), InputError>;
}

/// Runs before every batch of keys. The caller waits there until the user
/// has let go of keys that would change them, such as Right Alt turning
/// Ctrl+V into AltGr+V, or returns an error to send nothing more.
pub type BeforeKeys<'a> = &'a mut dyn FnMut() -> Result<(), InputError>;

/// A keyboard that calls [`BeforeKeys`] right before each batch, so the check
/// runs after any wait for the clipboard and the paste settle, not before.
pub(crate) struct Guarded<'a> {
    keyboard: &'a mut dyn Keyboard,
    before: BeforeKeys<'a>,
}

impl<'a> Guarded<'a> {
    pub(crate) fn new(keyboard: &'a mut dyn Keyboard, before: BeforeKeys<'a>) -> Guarded<'a> {
        Guarded { keyboard, before }
    }
}

impl Keyboard for Guarded<'_> {
    fn send(&mut self, strokes: &[Stroke]) -> Result<(), InputError> {
        (self.before)()?;
        self.keyboard.send(strokes)
    }
}

/// Types `text` one character per batch, [`TYPE_PACE`] apart.
pub(crate) fn type_text(
    text: &str,
    keyboard: &mut dyn Keyboard,
    clock: &dyn Clock,
) -> Result<(), InputError> {
    for batch in strokes::typing(text) {
        keyboard.send(&batch)?;
        clock.sleep(TYPE_PACE);
    }
    Ok(())
}

pub(crate) trait Clock {
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

/// The window a paste went to.
pub(crate) trait Target {
    /// Whether it handles messages again: it answers within `timeout`, or it
    /// is gone. A window that is busy or hung may still read the clipboard.
    fn answers(&self, timeout: Duration) -> bool;
}

pub(crate) struct SystemClock(Instant);

impl SystemClock {
    pub(crate) fn new() -> SystemClock {
        SystemClock(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// The user's clipboard, waiting to be put back after a paste.
#[derive(Debug)]
pub(crate) struct PendingRestore<J> {
    snapshot: J,
    /// The sequence number our text got, read while the clipboard was open.
    ours: u32,
}

/// Puts `text` on the clipboard and presses Ctrl+V. The caller then calls
/// [`finish_paste`].
pub(crate) fn paste<J>(
    text: &str,
    clipboard: &mut dyn Clipboard<Snapshot = J>,
    keyboard: &mut dyn Keyboard,
    clock: &dyn Clock,
) -> Result<PendingRestore<J>, InputError> {
    let snapshot = clipboard.snapshot()?;
    let before = clipboard.sequence();
    let ours = match clipboard.set_text(text) {
        Ok(ours) => ours,
        Err(err) => {
            let after = clipboard.sequence();
            if after != before {
                // It may have emptied the clipboard before it failed.
                let _ = clipboard.restore(&snapshot, Some(after));
            }
            return Err(err);
        }
    };
    let pending = PendingRestore { snapshot, ours };
    clock.sleep(PASTE_SETTLE);
    if let Err(err) = keyboard.send(&strokes::paste()) {
        // Nothing will read our text, so put the user's clipboard back now.
        let _ = restore_if_unchanged(clipboard, &pending);
        return Err(err);
    }
    Ok(pending)
}

/// After a paste: waits [`RESTORE_AFTER`], then puts the user's clipboard
/// back once the window the keys went to answers. When it doesn't answer
/// within [`TARGET_ANSWER`], it may still paste later, so the dictation stays
/// on the clipboard. Returns whether the snapshot went back.
pub(crate) fn finish_paste<J>(
    clipboard: &mut dyn Clipboard<Snapshot = J>,
    clock: &dyn Clock,
    target: &dyn Target,
    pending: &PendingRestore<J>,
) -> Result<bool, InputError> {
    clock.sleep(RESTORE_AFTER);
    if !target.answers(TARGET_ANSWER) {
        return Ok(false);
    }
    restore_if_unchanged(clipboard, pending)
}

/// Puts the snapshot back unless the clipboard changed after our paste.
/// Returns whether it did.
pub(crate) fn restore_if_unchanged<J>(
    clipboard: &mut dyn Clipboard<Snapshot = J>,
    pending: &PendingRestore<J>,
) -> Result<bool, InputError> {
    if clipboard.sequence() != pending.ours {
        // Someone copied something since; theirs wins.
        return Ok(false);
    }
    // Checked again with the clipboard open: a copy made in between wins too.
    clipboard.restore(&pending.snapshot, Some(pending.ours))
}

/// Copies the selection with Ctrl+C and returns its text, then puts the
/// user's clipboard back. Makes two attempts, as Ownkey did, each waiting up
/// to `timeout` for the app to copy. `None` when nothing with text was copied.
pub(crate) fn capture<J>(
    clipboard: &mut dyn Clipboard<Snapshot = J>,
    keyboard: &mut dyn Keyboard,
    clock: &dyn Clock,
    timeout: Duration,
) -> Result<Option<String>, InputError> {
    let snapshot = clipboard.snapshot()?;
    let original = clipboard.sequence();
    let copied = copy_selection(clipboard, keyboard, clock, timeout);
    // The selection's copy is what goes; a copy the user makes after it stays.
    let selection = match &copied {
        Ok(Some((_, seen))) => *seen,
        _ => clipboard.sequence(),
    };
    if selection != original {
        // A failed restore leaves the selection on the clipboard, which is
        // what Ownkey always did; the captured text is still good.
        let _ = clipboard.restore(&snapshot, Some(selection));
    }
    copied.map(|copied| copied.map(|(text, _)| text))
}

/// The selection's text and the sequence number it was read at.
fn copy_selection<J>(
    clipboard: &mut dyn Clipboard<Snapshot = J>,
    keyboard: &mut dyn Keyboard,
    clock: &dyn Clock,
    timeout: Duration,
) -> Result<Option<(String, u32)>, InputError> {
    for settle in COPY_SETTLE {
        clock.sleep(settle);
        let before = clipboard.sequence();
        keyboard.send(&strokes::copy())?;
        let deadline = clock.now() + timeout;
        loop {
            if clipboard.sequence() != before {
                match clipboard.text() {
                    Ok((Some(text), seen)) if !text.is_empty() => return Ok(Some((text, seen))),
                    // Still being written, or no text (yet).
                    Ok(_) | Err(InputError::ClipboardBusy) => {}
                    Err(err) => return Err(err),
                }
            }
            if clock.now() >= deadline {
                break;
            }
            clock.sleep(COPY_POLL);
        }
    }
    Ok(None)
}

/// Serializes clipboard work in this process. A paste's delayed restore must
/// finish before the next paste or capture takes its snapshot, or that
/// snapshot would hold our own text instead of the user's.
#[derive(Debug)]
pub(crate) struct Gate {
    busy: Mutex<bool>,
    freed: Condvar,
}

impl Gate {
    pub(crate) const fn new() -> Gate {
        Gate {
            busy: Mutex::new(false),
            freed: Condvar::new(),
        }
    }

    /// Waits until no other clipboard work runs, then holds the gate until
    /// the returned turn is dropped, possibly on another thread.
    pub(crate) fn enter(&'static self) -> Turn {
        let mut busy = self.busy.lock().unwrap_or_else(PoisonError::into_inner);
        while *busy {
            busy = self
                .freed
                .wait(busy)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *busy = true;
        Turn { gate: self }
    }
}

#[derive(Debug)]
pub(crate) struct Turn {
    gate: &'static Gate,
}

impl Drop for Turn {
    fn drop(&mut self) {
        *self
            .gate
            .busy
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = false;
        self.gate.freed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    const HTML: u32 = 0xC0F0;
    const PNG: u32 = 0xC0F1;
    const TIMEOUT: Duration = Duration::from_millis(500);

    fn text_format(text: &str) -> Format {
        let data = text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        Format {
            id: format::UNICODETEXT,
            data,
        }
    }

    fn decode(formats: &[Format]) -> Option<String> {
        let data = &formats.iter().find(|f| f.id == format::UNICODETEXT)?.data;
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        Some(String::from_utf16_lossy(&units))
    }

    /// A clipboard, the focused app and a clock, all fake.
    #[derive(Default)]
    struct Desk {
        formats: Vec<Format>,
        sequence: u32,
        hidden: bool,
        /// Opening the clipboard fails while this is set.
        locked: bool,
        /// The next text reads fail as busy, as while the app is still writing.
        busy_reads: u32,
        /// What the focused app puts on the clipboard for Ctrl+C, and how long it takes.
        selection: Option<Vec<Format>>,
        copy_delay: Duration,
        copy_at: Option<Duration>,
        /// SendInput is blocked, as for an elevated window.
        blocked: bool,
        /// Another app copies this after a restore checked the sequence
        /// number and before the restore gets the clipboard open.
        copied_while_opening: Option<Vec<Format>>,
        /// Another app copies this right after our text went on the
        /// clipboard and the clipboard closed.
        copied_after_write: Option<Vec<Format>>,
        /// The window the paste went to doesn't answer, as while it is busy.
        target_busy: bool,
        /// How long each question to the target waited.
        asked: Vec<Duration>,
        sent: Vec<Vec<Stroke>>,
        pasted: Vec<Option<String>>,
        restores: u32,
        time: Duration,
        sleeps: Vec<Duration>,
    }

    impl Desk {
        fn with(formats: Vec<Format>) -> Desk {
            Desk {
                formats,
                sequence: 100,
                ..Desk::default()
            }
        }

        fn replace(&mut self, formats: Vec<Format>, hidden: bool) {
            self.formats = formats;
            self.hidden = hidden;
            self.sequence += 1;
        }

        /// The app finishes its copy once enough time has passed.
        fn settle_copy(&mut self) {
            if self.copy_at.is_some_and(|at| self.time >= at) {
                self.copy_at = None;
                let selection = self.selection.clone().unwrap_or_default();
                self.replace(selection, false);
            }
        }

        fn open(&self) -> Result<(), InputError> {
            if self.locked {
                Err(InputError::ClipboardBusy)
            } else {
                Ok(())
            }
        }
    }

    #[derive(Clone)]
    struct Fake(Rc<RefCell<Desk>>);

    impl Fake {
        fn new(desk: Desk) -> Fake {
            Fake(Rc::new(RefCell::new(desk)))
        }

        fn desk(&self) -> std::cell::RefMut<'_, Desk> {
            self.0.borrow_mut()
        }

        fn paste(&self, text: &str) -> Result<PendingRestore<Snapshot>, InputError> {
            paste(text, &mut self.clone(), &mut self.clone(), &self.clone())
        }

        fn restore(&self, pending: &PendingRestore<Snapshot>) -> Result<bool, InputError> {
            restore_if_unchanged(&mut self.clone(), pending)
        }

        fn finish(&self, pending: &PendingRestore<Snapshot>) -> Result<bool, InputError> {
            finish_paste(&mut self.clone(), &self.clone(), &self.clone(), pending)
        }

        fn capture(&self) -> Result<Option<String>, InputError> {
            capture(&mut self.clone(), &mut self.clone(), &self.clone(), TIMEOUT)
        }
    }

    impl Target for Fake {
        fn answers(&self, timeout: Duration) -> bool {
            let mut desk = self.desk();
            desk.asked.push(timeout);
            if desk.target_busy {
                desk.time += timeout;
            }
            !desk.target_busy
        }
    }

    impl Clipboard for Fake {
        type Snapshot = Snapshot;

        fn sequence(&self) -> u32 {
            let mut desk = self.desk();
            desk.settle_copy();
            desk.sequence
        }

        fn snapshot(&mut self) -> Result<Snapshot, InputError> {
            let desk = self.desk();
            desk.open()?;
            let formats = desk
                .formats
                .iter()
                .filter(|f| holds_hglobal(f.id))
                .cloned()
                .collect();
            Ok(Snapshot { formats })
        }

        fn set_text(&mut self, text: &str) -> Result<u32, InputError> {
            let mut desk = self.desk();
            desk.open()?;
            desk.replace(vec![text_format(text)], true);
            let ours = desk.sequence;
            if let Some(theirs) = desk.copied_after_write.take() {
                desk.replace(theirs, false);
            }
            Ok(ours)
        }

        fn text(&mut self) -> Result<(Option<String>, u32), InputError> {
            let mut desk = self.desk();
            desk.settle_copy();
            if desk.busy_reads > 0 {
                desk.busy_reads -= 1;
                return Err(InputError::ClipboardBusy);
            }
            desk.open()?;
            Ok((decode(&desk.formats), desk.sequence))
        }

        fn restore(
            &mut self,
            snapshot: &Snapshot,
            expected: Option<u32>,
        ) -> Result<bool, InputError> {
            let mut desk = self.desk();
            if let Some(theirs) = desk.copied_while_opening.take() {
                desk.replace(theirs, false);
            }
            desk.open()?;
            if expected.is_some_and(|expected| expected != desk.sequence) {
                return Ok(false);
            }
            desk.restores += 1;
            let hidden = !snapshot.formats.is_empty();
            desk.replace(snapshot.formats.clone(), hidden);
            Ok(true)
        }
    }

    impl Keyboard for Fake {
        fn send(&mut self, batch: &[Stroke]) -> Result<(), InputError> {
            let mut desk = self.desk();
            desk.sent.push(batch.to_vec());
            if desk.blocked {
                return Err(InputError::Blocked);
            }
            if batch == strokes::paste() {
                let text = decode(&desk.formats);
                desk.pasted.push(text);
            } else if batch == strokes::copy() && desk.selection.is_some() {
                desk.copy_at = Some(desk.time + desk.copy_delay);
            }
            Ok(())
        }
    }

    impl Clock for Fake {
        fn now(&self) -> Duration {
            self.desk().time
        }

        fn sleep(&self, duration: Duration) {
            let mut desk = self.desk();
            desk.time += duration;
            desk.sleeps.push(duration);
        }
    }

    fn rich_clipboard() -> Vec<Format> {
        vec![
            text_format("the user's text"),
            Format {
                id: HTML,
                data: b"<b>the user's text</b>".to_vec(),
            },
        ]
    }

    #[test]
    fn paste_puts_hidden_text_on_the_clipboard_and_presses_ctrl_v() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let pending = fake.paste("dictated").unwrap();
        {
            let desk = fake.desk();
            assert_eq!(desk.pasted, [Some("dictated".to_string())]);
            assert!(desk.hidden, "dictation stays out of clipboard history");
            assert_eq!(desk.sleeps, [PASTE_SETTLE]);
            assert_eq!(desk.sent, [strokes::paste()]);
            assert_eq!(desk.restores, 0, "the restore waits for the app to read");
        }
        assert_eq!(fake.restore(&pending), Ok(true));
        assert_eq!(fake.desk().formats, rich_clipboard());
    }

    #[test]
    fn restore_is_skipped_when_someone_copied_after_the_paste() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let pending = fake.paste("dictated").unwrap();
        let theirs = vec![text_format("copied meanwhile")];
        fake.desk().replace(theirs.clone(), false);
        assert_eq!(fake.restore(&pending), Ok(false));
        assert_eq!(fake.desk().formats, theirs);
        assert_eq!(fake.desk().restores, 0);
    }

    #[test]
    fn a_copy_made_right_after_our_write_wins() {
        let theirs = vec![text_format("copied right after")];
        let mut desk = Desk::with(rich_clipboard());
        desk.copied_after_write = Some(theirs.clone());
        let fake = Fake::new(desk);
        // The copy lands after our text went on the clipboard and before the
        // paste looks at the sequence number: it is the user's, not ours.
        let pending = fake.paste("dictated").unwrap();
        assert_eq!(fake.restore(&pending), Ok(false));
        let desk = fake.desk();
        assert_eq!(desk.formats, theirs);
        assert_eq!(desk.restores, 0);
    }

    #[test]
    fn a_copy_made_while_the_restore_opens_the_clipboard_wins() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let pending = fake.paste("dictated").unwrap();
        // The sequence number still matches when the restore looks; the copy
        // lands while it waits for OpenClipboard.
        let theirs = vec![text_format("copied right then")];
        fake.desk().copied_while_opening = Some(theirs.clone());
        assert_eq!(fake.restore(&pending), Ok(false));
        assert_eq!(fake.desk().formats, theirs);
        assert_eq!(fake.desk().restores, 0);
    }

    #[test]
    fn the_restore_waits_and_asks_the_target_first() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let pending = fake.paste("dictated").unwrap();
        assert_eq!(fake.finish(&pending), Ok(true));
        let desk = fake.desk();
        assert_eq!(desk.sleeps, [PASTE_SETTLE, RESTORE_AFTER]);
        assert_eq!(desk.asked, [TARGET_ANSWER]);
        assert_eq!(desk.formats, rich_clipboard());
        assert!(
            RESTORE_AFTER >= Duration::from_secs(2),
            "600 ms was too short"
        );
    }

    #[test]
    fn a_busy_target_keeps_the_dictation_on_the_clipboard() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let pending = fake.paste("dictated").unwrap();
        // The app hasn't handled Ctrl+V yet and may read the clipboard any
        // moment: the user's old clipboard must not be what it pastes.
        fake.desk().target_busy = true;
        assert_eq!(fake.finish(&pending), Ok(false));
        let desk = fake.desk();
        assert_eq!(desk.formats, [text_format("dictated")]);
        assert_eq!(desk.restores, 0);
    }

    #[test]
    fn the_key_check_runs_right_before_ctrl_v() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let seen = RefCell::new(Vec::new());
        let mut check = || {
            let desk = fake.desk();
            seen.borrow_mut()
                .push((desk.sent.len(), decode(&desk.formats), desk.sleeps.clone()));
            Ok(())
        };
        let mut keyboard = fake.clone();
        let mut guarded = Guarded::new(&mut keyboard, &mut check);
        paste("dictated", &mut fake.clone(), &mut guarded, &fake.clone()).unwrap();
        // After our text went on the clipboard and settled, before any key.
        assert_eq!(
            seen.into_inner(),
            [(0, Some("dictated".to_string()), vec![PASTE_SETTLE])]
        );
        assert_eq!(fake.desk().sent, [strokes::paste()]);
    }

    #[test]
    fn a_cancelled_key_check_sends_nothing_and_restores_at_once() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let mut cancel = || Err(InputError::System("Voice is quitting.".into()));
        let mut keyboard = fake.clone();
        let mut guarded = Guarded::new(&mut keyboard, &mut cancel);
        let err = paste("dictated", &mut fake.clone(), &mut guarded, &fake.clone()).unwrap_err();
        assert_eq!(err, InputError::System("Voice is quitting.".into()));
        let desk = fake.desk();
        assert!(desk.sent.is_empty());
        assert_eq!(desk.formats, rich_clipboard());
        assert_eq!(desk.restores, 1);
    }

    #[test]
    fn typing_checks_the_keys_before_every_character() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let checks = RefCell::new(Vec::new());
        let mut check = || {
            checks.borrow_mut().push(fake.desk().sent.len());
            Ok(())
        };
        let mut keyboard = fake.clone();
        let mut guarded = Guarded::new(&mut keyboard, &mut check);
        type_text("hé\nx", &mut guarded, &fake.clone()).unwrap();
        assert_eq!(checks.into_inner(), [0, 1, 2, 3], "one check per batch");
        let desk = fake.desk();
        assert_eq!(desk.sent.len(), 4);
        assert_eq!(desk.sleeps, [TYPE_PACE; 4]);
        assert_eq!(
            desk.formats,
            rich_clipboard(),
            "typing leaves the clipboard alone"
        );
    }

    #[test]
    fn a_capture_checks_the_keys_before_each_ctrl_c() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        let mut checks = 0;
        let mut check = || {
            checks += 1;
            Ok(())
        };
        let mut keyboard = fake.clone();
        let mut guarded = Guarded::new(&mut keyboard, &mut check);
        let copied = capture(&mut fake.clone(), &mut guarded, &fake.clone(), TIMEOUT);
        assert_eq!(copied, Ok(None));
        assert_eq!(checks, 2, "both attempts");
    }

    #[test]
    fn a_copy_made_while_a_capture_restores_wins() {
        let mut desk = Desk::with(rich_clipboard());
        desk.selection = Some(vec![text_format("selected")]);
        let theirs = vec![text_format("copied right then")];
        desk.copied_while_opening = Some(theirs.clone());
        let fake = Fake::new(desk);
        assert_eq!(fake.capture(), Ok(Some("selected".to_string())));
        let desk = fake.desk();
        assert_eq!(desk.formats, theirs);
        assert_eq!(desk.restores, 0);
    }

    #[test]
    fn an_empty_clipboard_is_put_back_empty() {
        let fake = Fake::new(Desk::with(Vec::new()));
        let pending = fake.paste("dictated").unwrap();
        assert_eq!(fake.restore(&pending), Ok(true));
        assert!(fake.desk().formats.is_empty());
    }

    #[test]
    fn formats_that_cannot_be_copied_are_left_out() {
        let mut formats = rich_clipboard();
        formats.push(Format {
            id: format::BITMAP,
            data: vec![1, 2, 3],
        });
        let fake = Fake::new(Desk::with(formats));
        let pending = fake.paste("dictated").unwrap();
        fake.restore(&pending).unwrap();
        assert_eq!(fake.desk().formats, rich_clipboard());
    }

    #[test]
    fn a_blocked_paste_puts_the_clipboard_back_at_once() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        fake.desk().blocked = true;
        let err = fake.paste("dictated").unwrap_err();
        assert_eq!(err, InputError::Blocked);
        #[cfg(windows)]
        assert_eq!(
            err.to_string(),
            "Windows blocked typing into an app that runs as administrator."
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            err.to_string(),
            "macOS blocks typing while a password field or Terminal's Secure Keyboard Entry is active."
        );
        let desk = fake.desk();
        assert_eq!(desk.formats, rich_clipboard());
        assert_eq!(desk.restores, 1);
    }

    #[test]
    fn a_locked_clipboard_cancels_the_paste() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        fake.desk().locked = true;
        assert_eq!(
            fake.paste("dictated").unwrap_err(),
            InputError::ClipboardBusy
        );
        let desk = fake.desk();
        assert!(
            desk.sent.is_empty(),
            "no Ctrl+V without our text on the clipboard"
        );
        assert_eq!(desk.formats, rich_clipboard());
        assert_eq!(desk.sequence, 100);
    }

    #[test]
    fn capture_returns_the_selection_and_restores_the_clipboard() {
        let mut desk = Desk::with(rich_clipboard());
        desk.selection = Some(vec![text_format("selected words")]);
        let fake = Fake::new(desk);
        assert_eq!(fake.capture(), Ok(Some("selected words".to_string())));
        let desk = fake.desk();
        assert_eq!(desk.sent, [strokes::copy()]);
        assert_eq!(desk.sleeps[0], COPY_SETTLE[0]);
        assert_eq!(desk.formats, rich_clipboard());
        assert_eq!(desk.restores, 1);
    }

    #[test]
    fn capture_waits_for_a_slow_app() {
        let mut desk = Desk::with(rich_clipboard());
        desk.selection = Some(vec![text_format("slow")]);
        desk.copy_delay = Duration::from_millis(300);
        let fake = Fake::new(desk);
        assert_eq!(fake.capture(), Ok(Some("slow".to_string())));
        assert_eq!(fake.desk().sent.len(), 1);
    }

    #[test]
    fn capture_keeps_polling_while_the_app_still_holds_the_clipboard() {
        let mut desk = Desk::with(rich_clipboard());
        desk.selection = Some(vec![text_format("selected")]);
        desk.busy_reads = 3;
        let fake = Fake::new(desk);
        assert_eq!(fake.capture(), Ok(Some("selected".to_string())));
        assert_eq!(fake.desk().formats, rich_clipboard());
    }

    #[test]
    fn nothing_selected_tries_twice_and_leaves_the_clipboard_alone() {
        let fake = Fake::new(Desk::with(rich_clipboard()));
        assert_eq!(fake.capture(), Ok(None));
        let desk = fake.desk();
        assert_eq!(desk.sent, [strokes::copy(), strokes::copy()]);
        assert_eq!(desk.restores, 0);
        assert_eq!(desk.sequence, 100);
        assert!(desk.sleeps.contains(&COPY_SETTLE[0]) && desk.sleeps.contains(&COPY_SETTLE[1]));
        // Each attempt waits out its timeout before giving up.
        assert!(desk.time >= COPY_SETTLE[0] + COPY_SETTLE[1] + 2 * TIMEOUT);
        assert!(desk.time < COPY_SETTLE[0] + COPY_SETTLE[1] + 2 * (TIMEOUT + COPY_POLL));
    }

    #[test]
    fn a_copy_without_text_is_undone() {
        let mut desk = Desk::with(rich_clipboard());
        desk.selection = Some(vec![Format {
            id: PNG,
            data: vec![0x89, b'P', b'N', b'G'],
        }]);
        let fake = Fake::new(desk);
        assert_eq!(fake.capture(), Ok(None));
        let desk = fake.desk();
        assert_eq!(desk.formats, rich_clipboard());
        assert_eq!(desk.restores, 1);
    }

    #[test]
    fn a_blocked_copy_leaves_the_clipboard_alone() {
        let mut desk = Desk::with(rich_clipboard());
        desk.selection = Some(vec![text_format("selected")]);
        desk.blocked = true;
        let fake = Fake::new(desk);
        assert_eq!(fake.capture(), Err(InputError::Blocked));
        let desk = fake.desk();
        assert_eq!(desk.formats, rich_clipboard());
        assert_eq!(desk.restores, 0);
    }

    #[test]
    fn copies_only_global_memory_formats() {
        for id in [1, 7, 8, 13, 15, 16, 17, 0x81, 0xC000, 0xFFFF] {
            assert!(holds_hglobal(id), "format {id:#x}");
        }
        for id in [
            2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E, 0x200, 0x2FF, 0x300, 0x3FF,
        ] {
            assert!(!holds_hglobal(id), "format {id:#x}");
        }
    }

    #[test]
    fn skips_ole_formats_tied_to_the_copying_process() {
        assert!(is_process_bound("DataObject"));
        assert!(is_process_bound("Ole Private Data"));
        assert!(!is_process_bound("HTML Format"));
        assert!(!is_process_bound(
            "ExcludeClipboardContentFromMonitorProcessing"
        ));
    }

    #[test]
    fn the_gate_holds_the_next_turn_until_the_restore_ends() {
        static GATE: Gate = Gate::new();
        let restored = std::sync::Arc::new(AtomicBool::new(false));
        let (entered, wait) = mpsc::channel();
        let turn = GATE.enter();
        let restore = {
            let restored = std::sync::Arc::clone(&restored);
            thread::spawn(move || {
                entered.send(()).unwrap();
                thread::sleep(Duration::from_millis(50));
                restored.store(true, Ordering::SeqCst);
                drop(turn);
            })
        };
        wait.recv().unwrap();
        let _next = GATE.enter();
        assert!(restored.load(Ordering::SeqCst));
        restore.join().unwrap();
    }
}
