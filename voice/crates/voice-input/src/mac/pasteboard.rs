//! The general pasteboard behind [`Clipboard`], the counterpart of
//! `win.rs`'s clipboard.
//!
//! Windows lets one process hold the clipboard open, so its snapshot and its
//! check-then-restore happen without anybody copying in between. AppKit has
//! no such lock: `changeCount` is the only ordering, and the restore looks at
//! it as late as it can before writing.
//!
//! AppKit allows the general pasteboard from any thread. These calls run on
//! the thread that calls them: a paste writes on its caller, and the delayed
//! restore step 2 adds runs on the `clipboard-restore` thread. `changeCount`
//! orders the two.

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use std::sync::{Mutex, PoisonError};

use objc2_app_kit::{
    NSPasteboard, NSPasteboardAccessBehavior, NSPasteboardContentsOptions, NSPasteboardItem,
    NSPasteboardType, NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString};

use crate::clipboard::{Clipboard, Gate};
use crate::InputError;

/// `NSPasteboardTypeString` is an extern static; linking the framework is
/// what makes reading it safe.
fn string_type() -> &'static NSPasteboardType {
    unsafe { NSPasteboardTypeString }
}

/// The types clipboard managers honour as "don't keep this". They replace
/// Windows' `ExcludeClipboardContentFromMonitorProcessing`.
pub(crate) const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
pub(crate) const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";

/// The marker types Voice puts on its own text, in the order it adds them.
pub(crate) fn hidden_types() -> [&'static str; 2] {
    [CONCEALED_TYPE, TRANSIENT_TYPE]
}

/// One type of one pasteboard item: its name and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PasteboardType {
    pub name: String,
    pub data: Vec<u8>,
}

/// One pasteboard item, with every type and its data in the order the app
/// offered them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PasteboardItem {
    pub types: Vec<PasteboardType>,
}

/// Everything on the pasteboard when Voice took it: every item with every
/// type. Empty for an empty pasteboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PasteboardSnapshot {
    pub items: Vec<PasteboardItem>,
}

impl PasteboardSnapshot {
    /// Builds a snapshot from the items a pasteboard read gave, keeping their
    /// order and every type's bytes as they are.
    pub(crate) fn new(items: Vec<Vec<(String, Vec<u8>)>>) -> PasteboardSnapshot {
        PasteboardSnapshot {
            items: items
                .into_iter()
                .map(|types| PasteboardItem {
                    types: types
                        .into_iter()
                        .map(|(name, data)| PasteboardType { name, data })
                        .collect(),
                })
                .collect(),
        }
    }
}

/// What a restore may do with the pasteboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestorePlan {
    /// Someone copied since Voice's text: theirs stays.
    Leave,
    /// The change count is still Voice's, or the caller wants the snapshot
    /// back whatever happened: write it.
    Replace,
}

/// The change-count rule: a restore only writes while the pasteboard still
/// holds what Voice left on it.
pub(crate) fn plan_restore(current: u32, expected: Option<u32>) -> RestorePlan {
    match expected {
        Some(ours) if current != ours => RestorePlan::Leave,
        _ => RestorePlan::Replace,
    }
}

/// The general pasteboard's access setting, `None` before macOS 15.4, which has no such setting.
fn access_behavior() -> Option<NSPasteboardAccessBehavior> {
    if objc2::available!(macos = 15.4) {
        Some(autoreleasepool(|_| {
            NSPasteboard::generalPasteboard().accessBehavior()
        }))
    } else {
        None
    }
}

/// Whether Voice may read what another app copied without macOS asking the user. The general
/// pasteboard's default from macOS 15.4 is to ask at a programmatic read, and Voice's snapshot
/// is one, so only the user's "Always allow" counts, or a system without the setting.
pub(crate) fn reads_allowed(behavior: Option<NSPasteboardAccessBehavior>) -> bool {
    behavior.is_none_or(|behavior| behavior == NSPasteboardAccessBehavior::AlwaysAllow)
}

/// The general pasteboard. Its methods take `&mut self` only to fit
/// [`Clipboard`]; the pasteboard is process-wide.
pub(crate) struct Pasteboard;

impl Clipboard for Pasteboard {
    type Snapshot = PasteboardSnapshot;

    fn sequence(&self) -> u32 {
        autoreleasepool(|_| count(&NSPasteboard::generalPasteboard()))
    }

    fn may_read(&self) -> bool {
        reads_allowed(access_behavior())
    }

    fn snapshot(&mut self) -> Result<PasteboardSnapshot, InputError> {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            let mut items = Vec::new();
            if let Some(contents) = board.pasteboardItems() {
                for item in contents.to_vec() {
                    let mut types = Vec::new();
                    for name in item.types().to_vec() {
                        // A type its owner can't render yet is left out
                        // rather than failing the whole paste.
                        if let Some(data) = item.dataForType(&name) {
                            types.push((name.to_string(), data.to_vec()));
                        }
                    }
                    items.push(types);
                }
            }
            Ok(PasteboardSnapshot::new(items))
        })
    }

    /// Voice's own text stays on this Mac: with Handoff on, Universal Clipboard would offer it to
    /// the user's other devices, where the Concealed and Transient markers mean nothing. The
    /// count `prepareForNewContentsWithOptions` returns is the one this text keeps; the writes
    /// after it don't change it, and a count read at the end could be someone else's copy.
    fn set_text(&mut self, text: &str) -> Result<u32, InputError> {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            let ours = board
                .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly)
                as u32;
            let string = NSString::from_str(text);
            if !board.setString_forType(&string, string_type()) {
                return Err(InputError::Clipboard(
                    "The pasteboard refused the text.".into(),
                ));
            }
            // An empty value is how a private type is marked; clipboard
            // managers only look for the type's name.
            for hidden in hidden_types() {
                let name = NSString::from_str(hidden);
                let empty = NSData::with_bytes(&[]);
                if !board.setData_forType(Some(&empty), &name) {
                    return Err(InputError::Clipboard(format!(
                        "The pasteboard refused {hidden}."
                    )));
                }
            }
            Ok(ours)
        })
    }

    fn text(&mut self) -> Result<(Option<String>, u32), InputError> {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            let text = board
                .stringForType(string_type())
                .map(|text| text.to_string());
            // The count is read right after the text; a copy that lands in
            // between shows up on the next look, as it does on Windows once
            // the clipboard has closed.
            Ok((text, count(&board)))
        })
    }

    fn restore(
        &mut self,
        snapshot: &PasteboardSnapshot,
        expected: Option<u32>,
    ) -> Result<bool, InputError> {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            // Built before the last look: a type that is refused can't leave
            // the pasteboard empty, and the look stays close to the write.
            let items = items(snapshot);
            if items.is_empty() && !snapshot.items.is_empty() {
                // Nothing of the user's would go back: leave what is there.
                return Err(InputError::Clipboard(
                    "The pasteboard refused the copied items.".into(),
                ));
            }
            // The last look before the write. AppKit can't hold the
            // pasteboard closed, so a copy landing after this check and
            // before `clearContents` is the one case Voice can't see.
            if plan_restore(count(&board), expected) == RestorePlan::Leave {
                return Ok(false);
            }
            board.clearContents();
            if items.is_empty() {
                return Ok(true);
            }
            let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = items
                .into_iter()
                .map(ProtocolObject::from_retained)
                .collect();
            if !board.writeObjects(&NSArray::from_retained_slice(&objects)) {
                return Err(InputError::Clipboard(
                    "The pasteboard refused the copied items.".into(),
                ));
            }
            Ok(true)
        })
    }
}

/// The items of `snapshot`, ready to write. A type an item refuses is left out and the rest go
/// back, as on Windows, where a format that fails is skipped; an item left with no type at all
/// is left out too.
fn items(snapshot: &PasteboardSnapshot) -> Vec<Retained<NSPasteboardItem>> {
    let mut items = Vec::with_capacity(snapshot.items.len());
    for item in &snapshot.items {
        let pasteboard_item = NSPasteboardItem::new();
        let mut kept = false;
        for format in &item.types {
            let name = NSString::from_str(&format.name);
            let data = NSData::with_bytes(&format.data);
            kept |= pasteboard_item.setData_forType(&data, &name);
        }
        if kept {
            items.push(pasteboard_item);
        }
    }
    items
}

/// `NSPasteboard.changeCount` cut to the trait's width. It only has to be
/// compared for equality since Voice's own write, and the counter starts at
/// zero for the pasteboard server.
fn count(board: &NSPasteboard) -> u32 {
    board.changeCount() as u32
}

/// Serializes the pasteboard work in this process, as on Windows: a paste's
/// delayed restore must finish before the next paste or capture snapshots.
static CLIPBOARD_GATE: Gate = Gate::new();

/// Holds the gate for a paste whose restore runs on another thread, as
/// `win.rs` does: the turn moves to that thread so the next paste waits.
pub(crate) fn enter_gate() -> crate::clipboard::Turn {
    CLIPBOARD_GATE.enter()
}

/// The app Cmd+V went to, when its restore is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetState {
    /// It quit, or no app was in front at the paste.
    Gone,
    /// Another app is in front now: the user switched away.
    Behind,
    /// Still in front, and it answered an Accessibility request in time.
    Answers,
    /// Still in front, and its main thread didn't answer in time.
    Hung,
}

/// Whether the user's clipboard may go back. An app that quit or that the user left will read
/// nothing more, so the clipboard is theirs again, as on Windows, where a window that is gone
/// counts as answering. A hung app in front may still paste when it wakes up, and must get the
/// dictation then, not the old clipboard.
pub(crate) fn may_restore(state: TargetState) -> bool {
    state != TargetState::Hung
}

/// The app Cmd+V went to.
///
/// Windows asks the target window `WM_NULL`. On macOS Voice first looks at
/// whether the app still runs and is still in front, and only then asks it one
/// Accessibility request (`ax::app_answers`), which a hung app can't answer.
pub(crate) fn target() -> PasteTarget {
    PasteTarget(
        super::Frontmost::read()
            .map(|app| app.pid)
            .unwrap_or_default(),
    )
}

/// An app by pid, asked from the restore thread.
pub(crate) struct PasteTarget(i32);

impl PasteTarget {
    fn state(&self, timeout: std::time::Duration) -> TargetState {
        let pid = self.0;
        if pid == 0 || !super::is_running(pid) {
            TargetState::Gone
        } else if !super::is_in_front(pid) {
            TargetState::Behind
        } else if super::ax::app_answers(pid, timeout) {
            TargetState::Answers
        } else {
            TargetState::Hung
        }
    }
}

impl crate::clipboard::Target for PasteTarget {
    fn answers(&self, timeout: std::time::Duration) -> bool {
        may_restore(self.state(timeout))
    }
}

pub(crate) fn wait_for_clipboard_restore() {
    drop(CLIPBOARD_GATE.enter());
}

/// The last text [`copy_hidden`] wrote, by a hash rather than the secret itself, with the change
/// count it got. While the count is still that one, the pasteboard holds exactly that text, and
/// [`clear_if`] needs no read to know it.
static HIDDEN: Mutex<Option<(u64, u32)>> = Mutex::new(None);

fn fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Whether the pasteboard still holds Voice's hidden text, from the change counts alone: `None`
/// when Voice didn't write `text` itself in this run, and only a read can tell.
pub(crate) fn still_hidden(current: u32, written: Option<u32>) -> Option<bool> {
    written.map(|written| current == written)
}

pub(crate) fn copy_hidden(text: &str) -> Result<(), InputError> {
    // After a pending paste restore, which would otherwise put the user's old
    // clipboard back over this text.
    let _turn = CLIPBOARD_GATE.enter();
    let mut pasteboard = Pasteboard;
    let written = pasteboard.set_text(text)?;
    *HIDDEN.lock().unwrap_or_else(PoisonError::into_inner) = Some((fingerprint(text), written));
    Ok(())
}

pub(crate) fn clear_if(text: &str) -> Result<bool, InputError> {
    let _turn = CLIPBOARD_GATE.enter();
    let written = HIDDEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .filter(|(hash, _)| *hash == fingerprint(text))
        .map(|(_, written)| written);
    autoreleasepool(|_| {
        let board = NSPasteboard::generalPasteboard();
        let before = count(&board);
        let holds = match still_hidden(before, written) {
            Some(holds) => holds,
            // Reading it would make macOS ask the user, and an alert about a code Voice copied
            // minutes ago helps nobody: it stays.
            None if !reads_allowed(access_behavior()) => false,
            None => board
                .stringForType(string_type())
                .is_some_and(|current| current.to_string() == text),
        };
        // Only touch the pasteboard while it still holds what Voice put there:
        // a copy that lands during this check wins, as on Windows, where the
        // clipboard stays open between the read and the empty.
        if !holds || count(&board) != before {
            return Ok(false);
        }
        board.clearContents();
        Ok(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_only_writes_while_the_change_count_is_still_ours() {
        assert_eq!(plan_restore(7, Some(7)), RestorePlan::Replace);
        assert_eq!(plan_restore(8, Some(7)), RestorePlan::Leave);
        assert_eq!(
            plan_restore(8, None),
            RestorePlan::Replace,
            "without a count, the caller wants the snapshot back"
        );
    }

    #[test]
    fn the_clipboard_goes_back_unless_the_app_in_front_hangs() {
        assert!(may_restore(TargetState::Gone), "the app quit");
        assert!(may_restore(TargetState::Behind), "the user switched away");
        assert!(may_restore(TargetState::Answers), "it pasted and answers");
        assert!(
            !may_restore(TargetState::Hung),
            "a hung app in front may still paste"
        );
    }

    #[test]
    fn only_always_allow_lets_voice_read_unasked() {
        assert!(reads_allowed(None), "before macOS 15.4 nothing asks");
        assert!(reads_allowed(Some(NSPasteboardAccessBehavior::AlwaysAllow)));
        for behavior in [
            NSPasteboardAccessBehavior::Default,
            NSPasteboardAccessBehavior::Ask,
            NSPasteboardAccessBehavior::AlwaysDeny,
        ] {
            assert!(!reads_allowed(Some(behavior)), "{behavior:?}");
        }
    }

    #[test]
    fn a_hidden_copy_is_known_by_its_change_count() {
        assert_eq!(still_hidden(7, Some(7)), Some(true));
        assert_eq!(
            still_hidden(8, Some(7)),
            Some(false),
            "the user copied since"
        );
        assert_eq!(still_hidden(7, None), None, "not written in this run");
    }

    #[test]
    fn a_snapshot_keeps_every_item_type_and_its_bytes_in_order() {
        let items = vec![
            vec![
                ("public.utf8-plain-text".into(), b"plain".to_vec()),
                ("public.rtf".into(), b"{\\rtf1}".to_vec()),
            ],
            vec![
                ("public.file-url".into(), b"file:///tmp/a".to_vec()),
                ("public.utf8-plain-text".into(), b"a".to_vec()),
            ],
        ];
        let snapshot = PasteboardSnapshot::new(items);
        assert_eq!(snapshot.items.len(), 2);
        assert_eq!(snapshot.items[0].types.len(), 2);
        assert_eq!(snapshot.items[0].types[0].name, "public.utf8-plain-text");
        assert_eq!(snapshot.items[0].types[0].data, b"plain");
        assert_eq!(snapshot.items[0].types[1].name, "public.rtf");
        assert_eq!(snapshot.items[1].types[0].name, "public.file-url");
        assert_eq!(snapshot.items[1].types[0].data, b"file:///tmp/a");
        assert!(PasteboardSnapshot::new(Vec::new()).items.is_empty());
    }

    /// Building the items touches no pasteboard: they are plain objects until they are written.
    #[test]
    fn the_items_to_restore_keep_every_type_and_its_bytes() {
        let snapshot = PasteboardSnapshot::new(vec![
            vec![
                ("public.utf8-plain-text".into(), b"plain".to_vec()),
                ("public.rtf".into(), b"{\\rtf1}".to_vec()),
            ],
            vec![("public.file-url".into(), b"file:///tmp/a".to_vec())],
        ]);
        let built = autoreleasepool(|_| {
            items(&snapshot)
                .iter()
                .map(|item| {
                    item.types()
                        .to_vec()
                        .into_iter()
                        .map(|name| {
                            let data = item.dataForType(&name).map(|d| d.to_vec());
                            (name.to_string(), data.unwrap_or_default())
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        });
        let expected: Vec<Vec<(String, Vec<u8>)>> = snapshot
            .items
            .iter()
            .map(|item| {
                item.types
                    .iter()
                    .map(|t| (t.name.clone(), t.data.clone()))
                    .collect()
            })
            .collect();
        assert_eq!(built, expected);
    }

    #[test]
    fn voice_marks_its_own_text_concealed_and_transient() {
        assert_eq!(
            hidden_types(),
            [
                "org.nspasteboard.ConcealedType",
                "org.nspasteboard.TransientType"
            ]
        );
    }

    /// These change the real general pasteboard, so they need a desktop
    /// nobody is using. Run them there with
    /// `cargo test -p voice-input -- --ignored`. Each test puts the
    /// pasteboard back as it found it, also when it panics.
    mod desktop {
        use std::sync::{Mutex, MutexGuard};

        use super::super::*;

        /// The ignored tests share one pasteboard; run them one at a time.
        static PASTEBOARD: Mutex<()> = Mutex::new(());

        fn exclusive() -> MutexGuard<'static, ()> {
            PASTEBOARD.lock().unwrap_or_else(|err| err.into_inner())
        }

        /// Restores what was on the pasteboard before the test.
        struct PutBack(PasteboardSnapshot);

        impl Drop for PutBack {
            fn drop(&mut self) {
                let mut pasteboard = Pasteboard;
                let _ = pasteboard.restore(&self.0, None);
            }
        }

        fn keep_the_original() -> PutBack {
            let mut pasteboard = Pasteboard;
            PutBack(
                pasteboard
                    .snapshot()
                    .expect("read the pasteboard before the test"),
            )
        }

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn snapshot_and_restore_round_trip_the_real_pasteboard() {
            let _lock = exclusive();
            let _put_back = keep_the_original();
            let mut pasteboard = Pasteboard;
            pasteboard.set_text("before").unwrap();
            let snapshot = pasteboard.snapshot().unwrap();
            pasteboard.set_text("after").unwrap();
            assert_eq!(pasteboard.restore(&snapshot, None), Ok(true));
            assert_eq!(pasteboard.text().unwrap().0.as_deref(), Some("before"));
        }

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn a_snapshot_keeps_multiple_items_and_rich_types() {
            let _lock = exclusive();
            let _put_back = keep_the_original();
            let mut pasteboard = Pasteboard;
            let written = PasteboardSnapshot::new(vec![
                vec![
                    ("public.utf8-plain-text".into(), b"first".to_vec()),
                    ("public.html".into(), b"<b>first</b>".to_vec()),
                ],
                vec![(
                    "public.file-url".into(),
                    b"file:///tmp/rukoo-voice-test".to_vec(),
                )],
            ]);
            assert_eq!(pasteboard.restore(&written, None), Ok(true));
            assert_eq!(pasteboard.snapshot().unwrap(), written);
        }

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn set_text_returns_the_change_count_its_text_keeps() {
            let _lock = exclusive();
            let _put_back = keep_the_original();
            let mut pasteboard = Pasteboard;
            let ours = pasteboard.set_text("ours").unwrap();
            assert_eq!(pasteboard.sequence(), ours);
            // The dictation is concealed and transient, and still text.
            let snapshot = pasteboard.snapshot().unwrap();
            let types: Vec<&str> = snapshot.items[0]
                .types
                .iter()
                .map(|format| format.name.as_str())
                .collect();
            assert!(types.contains(&string_type().to_string().as_str()));
            for hidden in hidden_types() {
                assert!(types.contains(&hidden), "{hidden}");
            }
        }

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn a_restore_leaves_a_copy_the_user_made_in_between() {
            let _lock = exclusive();
            let _put_back = keep_the_original();
            let mut pasteboard = Pasteboard;
            pasteboard.set_text("ours").unwrap();
            let ours = pasteboard.sequence();
            let snapshot = pasteboard.snapshot().unwrap();
            // The user copies something while Voice waits to restore.
            pasteboard.set_text("the user's text").unwrap();
            assert_eq!(pasteboard.restore(&snapshot, Some(ours)), Ok(false));
            assert_eq!(
                pasteboard.text().unwrap().0.as_deref(),
                Some("the user's text")
            );
        }

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn a_hidden_copy_is_cleared_only_while_it_is_still_there() {
            let _lock = exclusive();
            let _put_back = keep_the_original();
            let mut pasteboard = Pasteboard;
            copy_hidden("RKV1-TEST").unwrap();
            assert_eq!(pasteboard.text().unwrap().0.as_deref(), Some("RKV1-TEST"));
            assert_eq!(clear_if("something else"), Ok(false));
            assert_eq!(pasteboard.text().unwrap().0.as_deref(), Some("RKV1-TEST"));
            assert_eq!(clear_if("RKV1-TEST"), Ok(true));
            assert_eq!(pasteboard.text().unwrap().0, None);
            // The user copied something since: it stays.
            copy_hidden("RKV1-TEST").unwrap();
            pasteboard.set_text("the user's text").unwrap();
            assert_eq!(clear_if("RKV1-TEST"), Ok(false));
            assert_eq!(
                pasteboard.text().unwrap().0.as_deref(),
                Some("the user's text")
            );
        }
    }
}
