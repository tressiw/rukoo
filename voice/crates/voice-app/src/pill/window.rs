//! Win32 for the pill window (pill-settings-design.md 3.2 and 3.3). Everything here runs on the event loop
//! thread, which owns the window.
//!
//! tao rewrites `GWL_EXSTYLE` from its own flags on every show, hide and flag change, and it has no flag
//! for `WS_EX_TOOLWINDOW`, so only this module shows or hides the pill and it reapplies the styles after
//! each tao call.

use std::sync::OnceLock;

use tauri::WebviewWindow;
use voice_engine::Edge;

#[cfg(windows)]
pub use imp::*;

/// What the pill reacts to while it runs: a display change (after it settles) and another app coming to
/// the foreground (to stay above other topmost windows).
///
/// `main.rs` registers these on every platform; the Windows window hooks are their only callers today,
/// and the macOS ones will be. Gating them to Windows would gate that registration and the pill's two
/// handlers with it, so they stay where they are and only their dead-code check is Windows'.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Callbacks {
    pub display_changed: Box<dyn Fn() + Send + Sync>,
    pub foreground_changed: Box<dyn Fn() + Send + Sync>,
}

static CALLBACKS: OnceLock<Callbacks> = OnceLock::new();

pub fn set_callbacks(callbacks: Callbacks) {
    let _ = CALLBACKS.set(callbacks);
}

/// A monitor handle as a plain integer, so it can cross threads.
pub type Monitor = isize;

/// The window's extended styles, decoded for logs and the self-test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Styles {
    pub ex_style: u32,
    pub toolwindow: bool,
    pub noactivate: bool,
    pub topmost: bool,
    pub transparent: bool,
    pub layered: bool,
    pub appwindow: bool,
    pub visible: bool,
}

#[cfg(windows)]
mod imp {
    use std::cell::Cell;

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR,
        MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::System::RemoteDesktop::{
        WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
    };
    use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
        IsWindowVisible, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, HWND_TOPMOST, SPI_SETWORKAREA, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE,
        WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
        WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE, WM_WTSSESSION_CHANGE, WS_EX_APPWINDOW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    };

    use super::*;
    use crate::pill::place::{self, Rect};

    /// The pill never activates, never shows in the taskbar or Alt+Tab, stays in the topmost band and lets
    /// every click through. The stop button is a separate window (hit.rs).
    const SET: u32 = WS_EX_TOOLWINDOW.0
        | WS_EX_NOACTIVATE.0
        | WS_EX_TOPMOST.0
        | WS_EX_TRANSPARENT.0
        | WS_EX_LAYERED.0;
    /// tao adds it to every window without an owner, even with skip_taskbar, and it beats TOOLWINDOW.
    const CLEAR: u32 = WS_EX_APPWINDOW.0;

    const WTS_CONSOLE_CONNECT: usize = 0x1;
    const WTS_REMOTE_CONNECT: usize = 0x3;
    const WTS_SESSION_UNLOCK: usize = 0x8;
    const SUBCLASS_ID: usize = 0x5256_4f49; // "RVOI"

    thread_local! {
        static FOREGROUND_HOOK: Cell<isize> = const { Cell::new(0) };
    }

    pub fn hwnd(window: &WebviewWindow) -> Option<HWND> {
        window.hwnd().ok()
    }

    /// Once after the window is built: click-through, our styles, and the display-change listeners.
    pub fn prepare(window: &WebviewWindow) {
        let _ = window.set_ignore_cursor_events(true);
        apply_styles(window);
        let Some(hwnd) = hwnd(window) else { return };
        unsafe {
            if !SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0).as_bool() {
                log::warn!("pill: could not listen for display changes");
            }
            if let Err(error) = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) {
                log::warn!("pill: no session notifications: {error}");
            }
        }
    }

    pub fn apply_styles(window: &WebviewWindow) {
        let Some(hwnd) = hwnd(window) else { return };
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let next = (ex | SET) & !CLEAR;
            if next != ex {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next as isize);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
    }

    /// Shows without activating; doesn't rely on tao's don't-focus marker staying set.
    pub fn show(window: &WebviewWindow) {
        let _ = window.show();
        apply_styles(window);
        if let Some(hwnd) = hwnd(window) {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    pub fn hide(window: &WebviewWindow) {
        let _ = window.hide();
        apply_styles(window);
    }

    /// Ownkey #14: SetWindowPos(HWND_TOPMOST) without activating, on every show and resync. A hidden
    /// window keeps WS_EX_TOPMOST while normal windows stack above it, and tao only calls SetWindowPos
    /// when its own always-on-top flag changes.
    pub fn raise(window: &WebviewWindow) -> bool {
        let Some(hwnd) = hwnd(window) else {
            return false;
        };
        raise_hwnd(hwnd)
    }

    pub fn raise_hwnd(hwnd: HWND) -> bool {
        unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .is_ok()
        }
    }

    /// The monitor the user works on: the foreground window's, unless that is the desktop or the
    /// taskbar; else the cursor's; else the primary monitor.
    pub fn target_monitor() -> Monitor {
        unsafe {
            let foreground = GetForegroundWindow();
            if !foreground.is_invalid() && !place::is_shell_class(&class_name(foreground)) {
                let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL);
                if !monitor.is_invalid() {
                    return monitor.0 as isize;
                }
            }
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor).is_ok() {
                return MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST).0 as isize;
            }
            MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY).0 as isize
        }
    }

    /// The work area and scale of the monitor under the cursor, for Settings.
    pub fn cursor_work_area() -> Option<(Rect, f64)> {
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor) }.ok()?;
        work_area_and_scale(
            unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) }.0 as isize,
        )
    }

    /// The monitor the window is on, or the nearest one when that monitor is gone.
    pub fn current_monitor(window: &WebviewWindow) -> Option<Monitor> {
        let hwnd = hwnd(window)?;
        Some(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST).0 as isize })
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }

    fn work_area_and_scale(monitor: Monitor) -> Option<(Rect, f64)> {
        let monitor = HMONITOR(monitor as *mut _);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return None;
            }
            let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
            let dpi = match GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
                Ok(()) => dpi_x,
                Err(_) => 96,
            };
            let w = info.rcWork;
            Some((
                Rect {
                    left: w.left,
                    top: w.top,
                    right: w.right,
                    bottom: w.bottom,
                },
                place::scale_for_dpi(dpi),
            ))
        }
    }

    /// Puts the window at the edge of `monitor`'s work area, sized for that monitor's scale.
    pub fn place(window: &WebviewWindow, monitor: Monitor, edge: Edge) -> Option<Rect> {
        let hwnd = hwnd(window)?;
        let (work, scale) = work_area_and_scale(monitor)?;
        let r = place::pill_window_rect(&work, scale, edge);
        unsafe {
            let set = || {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    r.left,
                    r.top,
                    r.width(),
                    r.height(),
                    SWP_NOACTIVATE,
                )
            };
            let _ = set();
            // Crossing to a monitor with another DPI sends WM_DPICHANGED during the call above, and tao
            // then resizes the window to keep its logical size. Measure again and correct once.
            let mut now = RECT::default();
            if GetWindowRect(hwnd, &mut now).is_ok()
                && (now.left, now.top, now.right, now.bottom) != (r.left, r.top, r.right, r.bottom)
            {
                let _ = set();
            }
        }
        Some(r)
    }

    /// The client area's top left on screen and the window's scale, to put the stop window over the
    /// button.
    pub fn origin_and_scale(window: &WebviewWindow) -> Option<((i32, i32), f64)> {
        let hwnd = hwnd(window)?;
        unsafe {
            let mut origin = POINT::default();
            if !ClientToScreen(hwnd, &mut origin).as_bool() {
                return None;
            }
            Some((
                (origin.x, origin.y),
                place::scale_for_dpi(GetDpiForWindow(hwnd)),
            ))
        }
    }

    pub fn set_capture_excluded(hwnd: HWND, excluded: bool) {
        let affinity = if excluded {
            WDA_EXCLUDEFROMCAPTURE
        } else {
            WDA_NONE
        };
        if let Err(error) = unsafe { SetWindowDisplayAffinity(hwnd, affinity) } {
            log::warn!("pill: display affinity {}: {error}", affinity.0);
        }
    }

    pub fn styles_of(hwnd: HWND) -> Styles {
        let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        Styles {
            ex_style: ex,
            toolwindow: ex & WS_EX_TOOLWINDOW.0 != 0,
            noactivate: ex & WS_EX_NOACTIVATE.0 != 0,
            topmost: ex & WS_EX_TOPMOST.0 != 0,
            transparent: ex & WS_EX_TRANSPARENT.0 != 0,
            layered: ex & WS_EX_LAYERED.0 != 0,
            appwindow: ex & WS_EX_APPWINDOW.0 != 0,
            visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
        }
    }

    pub fn styles(window: &WebviewWindow) -> Option<Styles> {
        hwnd(window).map(styles_of)
    }

    /// While the pill shows, every new foreground window re-raises it, so a call app's own topmost
    /// window (Teams' compact view, Zoom's controls) can't keep it covered.
    pub fn foreground_hook(on: bool) {
        FOREGROUND_HOOK.with(|hook| unsafe {
            if on && hook.get() == 0 {
                let handle = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(foreground_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                );
                hook.set(handle.0 as isize);
            } else if !on && hook.get() != 0 {
                let _ = UnhookWinEvent(HWINEVENTHOOK(hook.get() as *mut _));
                hook.set(0);
            }
        });
    }

    unsafe extern "system" fn foreground_proc(
        _hook: HWINEVENTHOOK,
        _event: u32,
        _hwnd: HWND,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if let Some(callbacks) = CALLBACKS.get() {
            (callbacks.foreground_changed)();
        }
    }

    /// Display changes (#13): Remote Desktop in and out, a monitor added or removed, a new scale or a
    /// moved taskbar. The pill repairs itself once they settle.
    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        let changed = match msg {
            WM_DISPLAYCHANGE | WM_DPICHANGED => true,
            WM_SETTINGCHANGE => wparam.0 as u32 == SPI_SETWORKAREA.0,
            WM_WTSSESSION_CHANGE => matches!(
                wparam.0,
                WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT | WTS_SESSION_UNLOCK
            ),
            _ => false,
        };
        // tao handles WM_DPICHANGED itself; let it finish before the repair is scheduled.
        let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
        if changed {
            if let Some(callbacks) = CALLBACKS.get() {
                (callbacks.display_changed)();
            }
        }
        result
    }
}

#[cfg(not(windows))]
mod imp {
    //! Other platforms get a plain always-on-top window; their non-activating panels are later work
    //! (pill-settings-design.md 3.10).
    use super::*;
    use crate::pill::place::Rect;

    pub fn prepare(window: &WebviewWindow) {
        let _ = window.set_ignore_cursor_events(true);
    }
    pub fn apply_styles(_window: &WebviewWindow) {}
    pub fn show(window: &WebviewWindow) {
        let _ = window.show();
    }
    pub fn hide(window: &WebviewWindow) {
        let _ = window.hide();
    }
    pub fn raise(_window: &WebviewWindow) -> bool {
        true
    }
    pub fn target_monitor() -> Monitor {
        0
    }
    pub fn current_monitor(_window: &WebviewWindow) -> Option<Monitor> {
        None
    }
    pub fn cursor_work_area() -> Option<(Rect, f64)> {
        None
    }
    pub fn place(_window: &WebviewWindow, _monitor: Monitor, _edge: Edge) -> Option<Rect> {
        None
    }
    pub fn styles(_window: &WebviewWindow) -> Option<Styles> {
        None
    }
    pub fn foreground_hook(_on: bool) {}
}

#[cfg(not(windows))]
pub use imp::*;
