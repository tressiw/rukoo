//! The Windows backend: key state, SendInput, the clipboard and the focus.

use std::mem::size_of;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowThreadProcessId, IsWindow, SendMessageTimeoutW, GUITHREADINFO, HWND_MESSAGE,
    SMTO_ABORTIFHUNG, SMTO_BLOCK, WINDOW_EX_STYLE, WINDOW_STYLE, WM_NULL,
};

use crate::clipboard::{
    self, format, BeforeKeys, Clipboard, Format, Gate, Guarded, Keyboard, Snapshot, SystemClock,
    Target,
};
use crate::strokes::{self, Stroke};
use crate::target::{classify, WindowInfo};
use crate::{InputError, Key};
/// pyperclip, which Ownkey used, retried OpenClipboard every 10 ms for 0.5 s.
const OPEN_TIMEOUT: Duration = Duration::from_millis(500);
const OPEN_RETRY: Duration = Duration::from_millis(10);

static CLIPBOARD_GATE: Gate = Gate::new();

pub(crate) fn key_state() -> Result<fn(Key) -> bool, InputError> {
    Ok(is_key_down)
}

pub(crate) fn is_key_down(key: Key) -> bool {
    // Only the high bit, "down right now". The low bit, "pressed since the
    // last call", is shared with every other caller and unreliable.
    let state = unsafe { GetAsyncKeyState(i32::from(key.vk())) };
    state as u16 & 0x8000 != 0
}

pub(crate) fn type_text(text: &str, before_keys: BeforeKeys) -> Result<(), InputError> {
    let mut keys = SendKeys;
    let mut guarded = Guarded::new(&mut keys, before_keys);
    clipboard::type_text(text, &mut guarded, &SystemClock::new())
}

pub(crate) fn paste_text(text: &str, before_keys: BeforeKeys) -> Result<(), InputError> {
    let turn = CLIPBOARD_GATE.enter();
    let pending = {
        let mut keys = SendKeys;
        let mut guarded = Guarded::new(&mut keys, before_keys);
        clipboard::paste(text, &mut WinClipboard, &mut guarded, &SystemClock::new())?
    };
    // Ctrl+V went to the foreground window, which reads the clipboard.
    let target = PasteTarget(unsafe { GetForegroundWindow() }.0 as isize);
    // Restoring on another thread lets the caller report the insertion now.
    // The turn moves along, so the next paste or capture waits for it. If the
    // thread can't start or the restore fails, our text stays on the
    // clipboard, which is what Ownkey always did.
    let _ = thread::Builder::new()
        .name("clipboard-restore".into())
        .spawn(move || {
            let clock = SystemClock::new();
            let _ = clipboard::finish_paste(&mut WinClipboard, &clock, &target, &pending);
            drop(turn);
        });
    Ok(())
}

pub(crate) fn capture_selection(
    timeout: Duration,
    before_keys: BeforeKeys,
) -> Result<Option<String>, InputError> {
    let _turn = CLIPBOARD_GATE.enter();
    let mut keys = SendKeys;
    let mut guarded = Guarded::new(&mut keys, before_keys);
    clipboard::capture(
        &mut WinClipboard,
        &mut guarded,
        &SystemClock::new(),
        timeout,
    )
}

/// The window Ctrl+V went to, by handle, so the restore thread can ask it.
struct PasteTarget(isize);

impl Target for PasteTarget {
    fn answers(&self, timeout: Duration) -> bool {
        let hwnd = HWND(self.0 as *mut core::ffi::c_void);
        // No window, or one that closed since: nothing reads our text any more.
        if self.0 == 0 || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return true;
        }
        // WM_NULL does nothing; the answer only says the window's thread
        // handles messages again. A hung window fails at once and a busy one
        // after the timeout. So does a window Windows won't let Voice message,
        // which leaves the dictation on the clipboard.
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let mut result = 0usize;
        let answered = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_NULL,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                millis,
                Some(&mut result),
            )
        };
        answered.0 != 0
    }
}

pub(crate) fn wait_for_clipboard_restore() {
    drop(CLIPBOARD_GATE.enter());
}

pub(crate) fn copy_hidden(text: &str) -> Result<(), InputError> {
    // After a pending paste restore, which would otherwise put the user's old clipboard back
    // over this text.
    let _turn = CLIPBOARD_GATE.enter();
    WinClipboard.set_text(text).map(|_| ())
}

pub(crate) fn clear_if(text: &str) -> Result<bool, InputError> {
    let _turn = CLIPBOARD_GATE.enter();
    // Read and emptied while the clipboard is open, so nobody's copy can come between the two.
    let _open = Open::new()?;
    let holds = read_global(format::UNICODETEXT).is_some_and(|data| utf16_text(&data) == text);
    if holds {
        unsafe { EmptyClipboard() }.map_err(clipboard_error)?;
    }
    Ok(holds)
}

pub(crate) fn foreground_window() -> Option<WindowInfo> {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() {
        return None;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(foreground, Some(&mut process_id)) };
    let mut gui = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    let has_info = thread_id != 0 && unsafe { GetGUIThreadInfo(thread_id, &mut gui) }.is_ok();
    let (focus_class, has_caret) = if has_info {
        let focused = if gui.hwndFocus.is_invalid() {
            foreground
        } else {
            gui.hwndFocus
        };
        (Some(class_name(focused)), !gui.hwndCaret.is_invalid())
    } else {
        (None, false)
    };
    Some(WindowInfo {
        hwnd: foreground.0 as isize,
        process_id,
        class_name: class_name(foreground),
        target: classify(focus_class.as_deref(), has_caret),
        focus_class,
        has_caret,
    })
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

struct SendKeys;

impl Keyboard for SendKeys {
    fn send(&mut self, batch: &[Stroke]) -> Result<(), InputError> {
        let inputs: Vec<INPUT> = batch.iter().map(to_input).collect();
        let inserted = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) } as usize;
        if inserted >= inputs.len() {
            return Ok(());
        }
        // UIPI blocks input into an app that runs as administrator, and
        // SendInput then inserts fewer events than it was given. Release
        // whatever did go down so Ctrl isn't left held.
        let release: Vec<INPUT> = strokes::release_after(&batch[..inserted])
            .iter()
            .map(to_input)
            .collect();
        if !release.is_empty() {
            unsafe { SendInput(&release, size_of::<INPUT>() as i32) };
        }
        Err(InputError::Blocked)
    }
}

fn to_input(stroke: &Stroke) -> INPUT {
    let (vk, scan, flags) = match *stroke {
        Stroke::Key { vk, up } => {
            // Ownkey's keyboard library sent the scan code with the virtual
            // key; some apps look only at the scan code.
            let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16;
            let flags = if up {
                KEYEVENTF_KEYUP
            } else {
                KEYBD_EVENT_FLAGS(0)
            };
            (VIRTUAL_KEY(vk), scan, flags)
        }
        Stroke::Unicode { unit, up } => {
            let flags = if up {
                KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
            } else {
                KEYEVENTF_UNICODE
            };
            (VIRTUAL_KEY(0), unit, flags)
        }
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// The clipboard, open on this thread until dropped.
struct Open {
    owner: HWND,
}

impl Open {
    fn new() -> Result<Open, InputError> {
        // Opened without a window, EmptyClipboard leaves the clipboard without
        // an owner and SetClipboardData may fail. The message-only window
        // lives only while the clipboard is open, so no other app ever waits
        // on it for a clipboard message.
        let owner = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .map_err(clipboard_error)?;
        let deadline = Instant::now() + OPEN_TIMEOUT;
        loop {
            if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
                return Ok(Open { owner });
            }
            if Instant::now() >= deadline {
                let _ = unsafe { DestroyWindow(owner) };
                return Err(InputError::ClipboardBusy);
            }
            thread::sleep(OPEN_RETRY);
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
            let _ = DestroyWindow(self.owner);
        }
    }
}

struct WinClipboard;

impl Clipboard for WinClipboard {
    type Snapshot = Snapshot;

    fn sequence(&self) -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    fn snapshot(&mut self) -> Result<Snapshot, InputError> {
        let _open = Open::new()?;
        let mut formats = Vec::new();
        let mut id = 0;
        loop {
            id = unsafe { EnumClipboardFormats(id) };
            if id == 0 {
                break;
            }
            if !clipboard::holds_hglobal(id)
                || format_name(id).is_some_and(|name| clipboard::is_process_bound(&name))
            {
                continue;
            }
            // A format its owner fails to render is left out rather than
            // failing the whole paste.
            if let Some(data) = read_global(id) {
                formats.push(Format { id, data });
            }
        }
        Ok(Snapshot { formats })
    }

    fn set_text(&mut self, text: &str) -> Result<u32, InputError> {
        let data: Vec<u8> = text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        let _open = Open::new()?;
        unsafe { EmptyClipboard() }.map_err(clipboard_error)?;
        put(format::UNICODETEXT, &data)?;
        hide_from_history()?;
        // Read while the clipboard is still open, so nobody's copy can come
        // between our text and its number.
        Ok(self.sequence())
    }

    fn text(&mut self) -> Result<(Option<String>, u32), InputError> {
        let _open = Open::new()?;
        let data = read_global(format::UNICODETEXT);
        // Read while the clipboard is open, so it belongs to exactly this text.
        let seen = self.sequence();
        let Some(data) = data else {
            return Ok((None, seen));
        };
        Ok((Some(utf16_text(&data)), seen))
    }

    fn restore(&mut self, snapshot: &Snapshot, expected: Option<u32>) -> Result<bool, InputError> {
        let _open = Open::new()?;
        // Nobody else can copy while the clipboard is open, so a copy made
        // after the caller's own check, while Open retried, is seen here.
        if expected.is_some_and(|expected| expected != self.sequence()) {
            return Ok(false);
        }
        unsafe { EmptyClipboard() }.map_err(clipboard_error)?;
        if snapshot.formats.is_empty() {
            return Ok(true);
        }
        let mut result = Ok(());
        for format in &snapshot.formats {
            if let Err(err) = put(format.id, &format.data) {
                result = result.and(Err(err));
            }
        }
        // Putting the user's own content back isn't a new copy, so it doesn't
        // belong in clipboard history a second time.
        hide_from_history()?;
        result.map(|()| true)
    }
}

/// The text of a `CF_UNICODETEXT` block. GlobalSize can round up, so it stops at the terminating
/// zero.
fn utf16_text(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn format_name(id: u32) -> Option<String> {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClipboardFormatNameW(id, &mut buf) };
    (len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

/// Copies the bytes of a global-memory format on the open clipboard.
fn read_global(id: u32) -> Option<Vec<u8>> {
    let handle = unsafe { GetClipboardData(id) }.ok()?;
    if handle.is_invalid() {
        return None;
    }
    let global = HGLOBAL(handle.0);
    unsafe {
        let size = GlobalSize(global);
        let ptr = GlobalLock(global) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let data = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(data)
    }
}

/// Puts one format on the open clipboard.
fn put(id: u32, data: &[u8]) -> Result<(), InputError> {
    unsafe {
        // A zero-size moveable block can't be locked, so allocate at least one zeroed byte.
        let global = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, data.len().max(1))
            .map_err(clipboard_error)?;
        let ptr = GlobalLock(global) as *mut u8;
        if ptr.is_null() {
            let err = windows::core::Error::from_thread();
            let _ = GlobalFree(Some(global));
            return Err(clipboard_error(err));
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        let _ = GlobalUnlock(global);
        if let Err(err) = SetClipboardData(id, Some(HANDLE(global.0))) {
            let _ = GlobalFree(Some(global));
            return Err(clipboard_error(err));
        }
        // The clipboard owns the memory now.
        Ok(())
    }
}

/// Keeps the clipboard's content out of clipboard history and cloud clipboard.
fn hide_from_history() -> Result<(), InputError> {
    let id =
        unsafe { RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")) };
    if id == 0 {
        return Err(clipboard_error(windows::core::Error::from_thread()));
    }
    put(id, &0u32.to_le_bytes())
}

fn clipboard_error(err: windows::core::Error) -> InputError {
    InputError::Clipboard(err.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(input: &INPUT) -> (u16, u16, u32) {
        let ki = unsafe { input.Anonymous.ki };
        (ki.wVk.0, ki.wScan, ki.dwFlags.0)
    }

    #[test]
    fn unicode_strokes_carry_the_unit_in_the_scan_code() {
        let down = to_input(&Stroke::Unicode {
            unit: 0xD83D,
            up: false,
        });
        let up = to_input(&Stroke::Unicode {
            unit: 0xD83D,
            up: true,
        });
        assert_eq!(down.r#type, INPUT_KEYBOARD);
        assert_eq!(fields(&down), (0, 0xD83D, KEYEVENTF_UNICODE.0));
        assert_eq!(
            fields(&up),
            (0, 0xD83D, (KEYEVENTF_UNICODE | KEYEVENTF_KEYUP).0)
        );
    }

    #[test]
    fn ctrl_v_becomes_four_keyboard_inputs() {
        let inputs: Vec<INPUT> = strokes::paste().iter().map(to_input).collect();
        let vks: Vec<(u16, u32)> = inputs.iter().map(|i| (fields(i).0, fields(i).2)).collect();
        let up = KEYEVENTF_KEYUP.0;
        assert_eq!(vks, [(0x11, 0), (0x56, 0), (0x56, up), (0x11, up)]);
        // MapVirtualKeyW only reads the layout. Left Ctrl is scan code 0x1D on every layout.
        assert_eq!(fields(&inputs[0]).1, 0x1D);
    }

    #[test]
    fn reads_physical_key_state() {
        // Port of Ownkey's test_reads_physical_key_state. F15 is on very few
        // keyboards, so it isn't held while the tests run.
        assert!(!is_key_down(Key::F15));
        assert!(key_state().is_ok());
    }

    #[test]
    fn reads_the_foreground_window_without_failing() {
        // Read-only: GetForegroundWindow and GetGUIThreadInfo change nothing.
        if let Some(window) = foreground_window() {
            assert_ne!(window.hwnd, 0);
            assert!(!window.class_name.is_empty());
        }
    }

    /// These change the real clipboard or send keys to the focused app, so
    /// they need a dedicated desktop (a VM or a spare account), never a PC
    /// someone is using. Run them there with `cargo test -p voice-input -- --ignored`.
    mod desktop {
        use super::super::*;
        use crate::InsertMode;

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn snapshot_and_restore_round_trip_the_real_clipboard() {
            let mut clipboard = WinClipboard;
            clipboard.set_text("before").unwrap();
            let snapshot = clipboard.snapshot().unwrap();
            clipboard.set_text("after").unwrap();
            assert_eq!(clipboard.restore(&snapshot, None), Ok(true));
            assert_eq!(clipboard.text().unwrap().0.as_deref(), Some("before"));
        }

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn set_text_returns_the_number_its_text_keeps_after_closing() {
            // The paste treats this number as its own: closing the clipboard
            // must not move it.
            let mut clipboard = WinClipboard;
            let ours = clipboard.set_text("ours").unwrap();
            assert_eq!(clipboard.sequence(), ours);
        }

        #[test]
        #[ignore = "needs a dedicated desktop: changes the real clipboard"]
        fn a_hidden_copy_is_cleared_only_while_it_is_still_there() {
            let mut clipboard = WinClipboard;
            crate::copy_hidden("RKV1-TEST").unwrap();
            assert_eq!(clipboard.text().unwrap().0.as_deref(), Some("RKV1-TEST"));
            assert_eq!(crate::clear_if("something else"), Ok(false));
            assert_eq!(clipboard.text().unwrap().0.as_deref(), Some("RKV1-TEST"));
            assert_eq!(crate::clear_if("RKV1-TEST"), Ok(true));
            assert_eq!(clipboard.text().unwrap().0, None);
            // The user copied something since: it stays.
            crate::copy_hidden("RKV1-TEST").unwrap();
            clipboard.set_text("the user's text").unwrap();
            assert_eq!(crate::clear_if("RKV1-TEST"), Ok(false));
            assert_eq!(
                clipboard.text().unwrap().0.as_deref(),
                Some("the user's text")
            );
        }

        #[test]
        #[ignore = "needs a dedicated desktop with Notepad focused: sends Ctrl+V and changes the clipboard"]
        fn paste_puts_the_clipboard_back() {
            let mut clipboard = WinClipboard;
            clipboard.set_text("the user's text").unwrap();
            crate::insert_text("Pasted by voice-input\nsecond line", InsertMode::Paste).unwrap();
            crate::wait_for_clipboard_restore();
            assert_eq!(
                clipboard.text().unwrap().0.as_deref(),
                Some("the user's text")
            );
        }

        #[test]
        #[ignore = "needs a dedicated desktop with Notepad focused: sends keys"]
        fn types_unicode_and_newlines() {
            crate::insert_text("Typed é 😀\nsecond line", InsertMode::Type).unwrap();
        }

        #[test]
        #[ignore = "needs a dedicated desktop with text selected in Notepad: sends Ctrl+C and changes the clipboard"]
        fn captures_the_selection_and_puts_the_clipboard_back() {
            let mut clipboard = WinClipboard;
            clipboard.set_text("the user's text").unwrap();
            let selection = crate::capture_selection(Duration::from_millis(500)).unwrap();
            assert!(selection.is_some_and(|text| !text.is_empty()));
            assert_eq!(
                clipboard.text().unwrap().0.as_deref(),
                Some("the user's text")
            );
        }

        #[test]
        #[ignore = "needs a dedicated desktop with an app running as administrator focused: sends keys"]
        fn typing_into_an_elevated_app_reports_blocked() {
            assert_eq!(
                crate::insert_text("x", InsertMode::Type),
                Err(InputError::Blocked)
            );
        }
    }
}
