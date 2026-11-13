//! The monitors as Core Graphics lists them: every active display with its pixel size and its
//! place in the desktop, the windows in the order the window server shows them, and the cursor.
//!
//! A monitor's `width` and `height` are the pixels the display's current mode draws, as on
//! Windows, so a scaled Retina display reports its backing store (3456 x 2234 on a 16-inch
//! MacBook Pro), not the logical size macOS lays windows out in. Its `x` and `y` stay in the
//! desktop's own unit, points, because that is what window frames use. Placing a window on a
//! display therefore uses the display's frame in points ([`frames`]), never the monitor's size.
//!
//! The display id is the `CGDirectDisplayID`, which ScreenCaptureKit names a display by too, so
//! `--monitor`, the journal and the capture agree on one monitor. A display keeps its id while it
//! is connected and the displays aren't rearranged; when the ids change, a resumed session finds
//! no monitor under its old id and chooses again, as it does on Windows.
//!
//! Reading the window list needs no Screen Recording grant: window titles do, and none are read.

use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect,
};
use objc2_core_graphics::{
    kCGWindowAlpha, kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowLayer, kCGWindowOwnerPID,
    CGDisplayBounds, CGDisplayCopyDisplayMode, CGDisplayMode, CGDisplayPixelsHigh,
    CGDisplayPixelsWide, CGError, CGEvent, CGGetActiveDisplayList, CGPreflightScreenCaptureAccess,
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
};
use voice_screen_files::monitors::{MonitorInfo, WindowInfo};

/// The ids of the active displays.
fn active_displays() -> Vec<u32> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    // SAFETY: the call writes at most `ids.len()` ids and the count it wrote.
    if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) }
        != CGError::Success
    {
        return Vec::new();
    }
    ids[..(count as usize).min(ids.len())].to_vec()
}

/// The pixel size of a display, oriented like its frame. `frame` is the display's size in points
/// as it is laid out, rotation included; `mode` is the pixel size of its current mode, which Core
/// Graphics reports for the panel as it is, unrotated. Without a usable mode the frame stands in,
/// which is right on a display that doesn't scale.
pub fn native_size(frame: (f64, f64), mode: Option<(usize, usize)>) -> (u32, u32) {
    let fallback = (frame.0.round() as u32, frame.1.round() as u32);
    let Some((width, height)) = mode.filter(|&(w, h)| w > 0 && h > 0) else {
        return fallback;
    };
    let (width, height) = (width as u32, height as u32);
    // The frame is portrait when the display is turned, and then so is the picture.
    if (frame.1 > frame.0) != (height > width) {
        (height, width)
    } else {
        (width, height)
    }
}

/// A display's pixel size from Core Graphics: its current mode's, else the size it reports.
fn display_pixels(id: u32, frame: &CGRect) -> (u32, u32) {
    let mode = CGDisplayCopyDisplayMode(id);
    let mode = mode.as_deref().map(|mode| {
        (
            CGDisplayMode::pixel_width(Some(mode)),
            CGDisplayMode::pixel_height(Some(mode)),
        )
    });
    let size = native_size((frame.size.width, frame.size.height), mode);
    if mode.is_some() {
        return size;
    }
    // No mode: the size Core Graphics reports for the display.
    let (width, height) = (
        CGDisplayPixelsWide(id) as u32,
        CGDisplayPixelsHigh(id) as u32,
    );
    if (frame.size.height > frame.size.width) != (height > width) {
        (height, width)
    } else {
        (width, height)
    }
}

/// Every active display, left to right, the main one marked primary.
pub fn list() -> Vec<MonitorInfo> {
    let mut monitors: Vec<MonitorInfo> = active_displays()
        .into_iter()
        .enumerate()
        .map(|(n, id)| {
            let bounds = CGDisplayBounds(id);
            let (width, height) = display_pixels(id, &bounds);
            MonitorInfo {
                id: id.to_string(),
                name: format!("Display {}", n + 1),
                x: bounds.origin.x.round() as i32,
                y: bounds.origin.y.round() as i32,
                width,
                height,
                // The main display is the one whose bounds hold the origin.
                primary: bounds.origin.x.abs() < 1.0 && bounds.origin.y.abs() < 1.0,
            }
        })
        .collect();
    monitors.sort_by_key(|m| (m.x, m.y));
    let has_primary = monitors.iter().any(|m| m.primary);
    if !has_primary {
        if let Some(first) = monitors.first_mut() {
            first.primary = true;
        }
    }
    monitors
}

/// Where a display is laid out: its id and its frame in points, which window frames share.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub id: String,
    pub frame: CGRect,
}

/// The frames of the active displays in points.
pub fn frames() -> Vec<Place> {
    active_displays()
        .into_iter()
        .map(|id| Place {
            id: id.to_string(),
            frame: CGDisplayBounds(id),
        })
        .collect()
}

/// Whether this process may read the screen: the Screen Recording grant, from the preflight call,
/// which never asks for it.
pub fn has_permission() -> bool {
    CGPreflightScreenCaptureAccess()
}

/// Why a real capture can't start, in the terms the session's start failure needs.
pub fn permission_error(granted: bool) -> Option<String> {
    (!granted).then(|| {
        "Screen Recording is off for Rukoo Voice: switch it on in System Settings > Privacy & \
         Security > Screen & System Audio Recording"
            .into()
    })
}

/// One top-level window as the window server describes it, before it is placed on a display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    /// The owning process.
    pub pid: i32,
    /// The window's layer: 0 for an ordinary application window, higher for panels, the menu bar
    /// and the Dock.
    pub layer: i32,
    pub alpha: f64,
    pub bounds: CGRect,
    pub on_screen: bool,
}

/// The inputs of `voice_screen_files::monitors::choose` from a window list: what the window is,
/// and the display it sits on. A window of a process in `exclude_pids`, or of this process, is
/// marked excluded, as a Windows window that hides itself from capture is; the choice then looks
/// past it to the display behind it.
pub fn windows(entries: &[Entry], places: &[Place], exclude_pids: &[u32]) -> Vec<WindowInfo> {
    let own = std::process::id() as i32;
    entries
        .iter()
        .map(|entry| WindowInfo {
            monitor: display_of(entry.bounds, places).unwrap_or_default(),
            visible: entry.on_screen && entry.alpha > 0.0,
            // A minimized window isn't on screen, so the list only holds the others.
            minimized: false,
            // A window in another Space isn't on screen either.
            cloaked: false,
            tool: entry.layer != 0,
            shell: false,
            excluded: entry.pid == own || exclude_pids.contains(&(entry.pid.max(0) as u32)),
        })
        .collect()
}

/// The display a window is on: the one under its centre, else the one it overlaps most, else the
/// empty id, which `choose` counts as a display that isn't there.
fn display_of(bounds: CGRect, places: &[Place]) -> Option<String> {
    let centre = (
        bounds.origin.x + bounds.size.width / 2.0,
        bounds.origin.y + bounds.size.height / 2.0,
    );
    if let Some(place) = places
        .iter()
        .find(|p| contains(&p.frame, centre.0, centre.1))
    {
        return Some(place.id.clone());
    }
    places
        .iter()
        .filter_map(|p| {
            let area = overlap(&p.frame, bounds);
            (area > 0.0).then_some((p, area))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p.id.clone())
}

fn contains(frame: &CGRect, x: f64, y: f64) -> bool {
    x >= frame.origin.x
        && y >= frame.origin.y
        && x < frame.origin.x + frame.size.width
        && y < frame.origin.y + frame.size.height
}

fn overlap(frame: &CGRect, bounds: CGRect) -> f64 {
    let left = frame.origin.x.max(bounds.origin.x);
    let top = frame.origin.y.max(bounds.origin.y);
    let right = (frame.origin.x + frame.size.width).min(bounds.origin.x + bounds.size.width);
    let bottom = (frame.origin.y + frame.size.height).min(bounds.origin.y + bounds.size.height);
    (right - left).max(0.0) * (bottom - top).max(0.0)
}

/// Top-level windows from the front window down the Z-order, as the window server lists them.
pub fn z_order(exclude_pids: &[u32]) -> Vec<WindowInfo> {
    let entries = entries();
    if entries.is_empty() {
        return Vec::new();
    }
    windows(&entries, &frames(), exclude_pids)
}

/// The windows the window server shows, front to back.
fn entries() -> Vec<Entry> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(options, 0) else {
        return Vec::new();
    };
    // SAFETY: the call answers with one dictionary per window.
    let list =
        unsafe { CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(list) };
    list.iter().filter_map(|window| entry(&window)).collect()
}

/// One window's numbers, read from its dictionary. A window without a bounds rectangle or an
/// owner is skipped: it can't be placed or attributed.
fn entry(window: &CFDictionary<CFString, CFType>) -> Option<Entry> {
    let number = |key: &CFString| -> Option<f64> {
        let value = window.get(key)?;
        // SAFETY: the keys this reads are numbers, as Core Graphics documents them.
        let value = unsafe { &*(&*value as *const CFType as *const CFNumber) };
        value.as_f64()
    };
    let flag = |key: &CFString| -> bool { number(key).is_some_and(|v| v != 0.0) };
    let pid = number(unsafe { kCGWindowOwnerPID })? as i32;
    let layer = number(unsafe { kCGWindowLayer })?.round() as i32;
    let alpha = number(unsafe { kCGWindowAlpha }).unwrap_or(1.0);
    let bounds = window
        .get(unsafe { kCGWindowBounds })
        .and_then(|value| rect(&value))?;
    Some(Entry {
        pid,
        layer,
        alpha,
        bounds,
        on_screen: flag(unsafe { kCGWindowIsOnscreen }),
    })
}

/// The rectangle in a `kCGWindowBounds` value.
fn rect(value: &CFRetained<CFType>) -> Option<CGRect> {
    // SAFETY: the value is the dictionary of "X", "Y", "Width" and "Height" Core Graphics
    // documents.
    let dict: &CFDictionary = unsafe { &*(&**value as *const CFType as *const CFDictionary) };
    let mut rect = CGRect::default();
    // SAFETY: the call writes only `rect`.
    let ok = unsafe { CGRectMakeWithDictionaryRepresentation(Some(dict), &mut rect) };
    ok.then_some(rect)
}

/// The cursor's display.
pub fn cursor_monitor() -> Option<String> {
    let point = CGEvent::location(CGEvent::new(None).as_deref());
    frames()
        .iter()
        .find(|p| contains(&p.frame, point.x, point.y))
        .map(|p| p.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: &str, x: i32, y: i32, width: u32, height: u32) -> MonitorInfo {
        MonitorInfo {
            id: id.into(),
            name: id.into(),
            x,
            y,
            width,
            height,
            primary: x == 0 && y == 0,
        }
    }

    fn rect_of(x: f64, y: f64, width: f64, height: f64) -> CGRect {
        CGRect {
            origin: objc2_core_foundation::CGPoint { x, y },
            size: objc2_core_foundation::CGSize { width, height },
        }
    }

    /// A GUI session has at least one display; a CI runner's may have none, which is allowed.
    #[test]
    fn lists_display_ids() {
        let monitors = list();
        for monitor in &monitors {
            assert!(!monitor.id.is_empty());
            assert!(monitor.width > 0 && monitor.height > 0, "{monitor:?}");
            // The id has to survive `--monitor`, which takes at most 64 characters.
            assert!(monitor.id.chars().count() <= 64);
        }
        let mut ids: Vec<&str> = monitors.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), monitors.len(), "duplicate display ids");
        assert!(monitors.iter().filter(|m| m.primary).count() <= 1);
    }

    /// The window list reads without a grant, and every window it names is on a display this
    /// process knows.
    #[test]
    fn the_window_list_reads() {
        let monitors = list();
        let windows = z_order(&[]);
        for window in &windows {
            assert!(
                window.monitor.is_empty() || monitors.iter().any(|m| m.id == window.monitor),
                "{window:?}"
            );
        }
    }

    /// This desktop in points: a 2560 x 1440 main display and a portrait 1440 x 2560 to its left.
    fn this_desktop() -> Vec<Place> {
        vec![
            Place {
                id: "1".into(),
                frame: rect_of(-1440.0, -425.0, 1440.0, 2560.0),
            },
            Place {
                id: "2".into(),
                frame: rect_of(0.0, 0.0, 2560.0, 1440.0),
            },
        ]
    }

    /// The same desktop as the monitor list reports it: the main display is a scaled Retina one
    /// whose pixels are twice its points, so its size here is not its frame's.
    fn this_desktop_monitors() -> Vec<MonitorInfo> {
        vec![
            monitor("1", -1440, -425, 1440, 2560),
            monitor("2", 0, 0, 5120, 2880),
        ]
    }

    fn entry(pid: i32, x: f64, y: f64, width: f64, height: f64) -> Entry {
        Entry {
            pid,
            layer: 0,
            alpha: 1.0,
            bounds: rect_of(x, y, width, height),
            on_screen: true,
        }
    }

    /// Each window is placed on the display its centre is on, else on the one it overlaps most.
    #[test]
    fn windows_land_on_their_display() {
        let places = this_desktop();
        let windows = windows(
            &[
                entry(10, 100.0, 100.0, 800.0, 600.0),
                entry(11, -1000.0, 0.0, 800.0, 600.0),
                // A window whose centre is on no display sits on the one it covers most.
                entry(12, -100.0, -1000.0, 400.0, 700.0),
                // One entirely off every display is on no display at all.
                entry(13, -5000.0, -5000.0, 100.0, 100.0),
            ],
            &places,
            &[],
        );
        assert_eq!(windows[0].monitor, "2");
        assert_eq!(windows[1].monitor, "1");
        assert_eq!(windows[2].monitor, "1");
        assert_eq!(windows[3].monitor, "");
    }

    /// Rukoo's and Voice's own windows count as excluded, so the choice looks past them; panels
    /// and off-screen windows are skipped by the choice itself.
    #[test]
    fn excluded_windows_are_marked() {
        let places = this_desktop();
        let mut status = entry(12, 0.0, 0.0, 2560.0, 24.0);
        status.layer = 25;
        let mut other_space = entry(13, 0.0, 0.0, 800.0, 600.0);
        other_space.on_screen = false;
        let own = std::process::id() as i32;
        let windows = windows(
            &[
                entry(4821, -1000.0, 0.0, 800.0, 600.0),
                entry(77, 100.0, 100.0, 800.0, 600.0),
                entry(own, 200.0, 200.0, 800.0, 600.0),
                status,
                other_space,
            ],
            &places,
            &[4821],
        );
        assert!(
            windows[0].excluded && windows[0].monitor == "1",
            "a pid from the command line"
        );
        assert!(!windows[1].excluded, "another application's window");
        assert!(windows[2].excluded, "this process");
        assert!(windows[3].tool, "a panel above the ordinary windows");
        assert!(!windows[3].excluded);
        assert!(!windows[4].visible, "a window in another Space");
    }

    /// Rukoo's window is in front on one display and a browser behind it on the other: the
    /// recording goes to the display of the browser, as on Windows with a capture-protected Rukoo.
    #[test]
    fn the_display_choice_looks_past_excluded_windows() {
        let places = this_desktop();
        let list = windows(
            &[
                entry(4821, -1000.0, 0.0, 800.0, 600.0),
                entry(77, 100.0, 100.0, 800.0, 600.0),
            ],
            &places,
            &[4821],
        );
        assert_eq!(
            voice_screen_files::monitors::choose(&list, Some("1"), &this_desktop_monitors())
                .as_deref(),
            Some("2")
        );
        // With every window excluded, the cursor's display decides.
        let list = windows(&[entry(4821, 100.0, 100.0, 800.0, 600.0)], &places, &[4821]);
        assert_eq!(
            voice_screen_files::monitors::choose(&list, Some("1"), &this_desktop_monitors())
                .as_deref(),
            Some("1")
        );
    }

    /// The size of a display in pixels is its mode's, turned like its frame, and the frame's own
    /// size when the mode isn't known.
    #[test]
    fn the_native_size_is_the_modes_oriented_like_the_frame() {
        // A 16-inch MacBook Pro in its default mode: 1728 x 1117 points on 3456 x 2234 pixels.
        assert_eq!(
            native_size((1728.0, 1117.0), Some((3456, 2234))),
            (3456, 2234)
        );
        // A 5K display looked at as 2560 x 1440 points.
        assert_eq!(
            native_size((2560.0, 1440.0), Some((5120, 2880))),
            (5120, 2880)
        );
        // A display that doesn't scale.
        assert_eq!(
            native_size((1920.0, 1080.0), Some((1920, 1080))),
            (1920, 1080)
        );
        // Turned a quarter: the frame is portrait and the mode reports the panel unrotated.
        assert_eq!(
            native_size((1080.0, 1920.0), Some((3840, 2160))),
            (2160, 3840)
        );
        // A mode that is already turned stays as it is.
        assert_eq!(
            native_size((1080.0, 1920.0), Some((2160, 3840))),
            (2160, 3840)
        );
        // No mode, or an empty one: the frame stands in.
        assert_eq!(native_size((1920.0, 1080.0), None), (1920, 1080));
        assert_eq!(native_size((1920.0, 1080.0), Some((0, 0))), (1920, 1080));
    }

    /// The displays this process lists report pixels, and a window is placed by the frames in
    /// points: the two agree on which displays there are.
    #[test]
    fn monitors_and_frames_are_the_same_displays() {
        let ids: Vec<String> = frames().into_iter().map(|p| p.id).collect();
        let mut listed: Vec<String> = list().into_iter().map(|m| m.id).collect();
        let mut framed = ids;
        listed.sort();
        framed.sort();
        assert_eq!(listed, framed);
        for monitor in list() {
            assert!(monitor.width > 0 && monitor.height > 0);
        }
    }

    /// The permission's failure names Screen Recording, and a granted one has no failure.
    #[test]
    fn the_permission_failure_names_the_setting() {
        let error = permission_error(false).expect("a refusal");
        assert!(error.contains("Screen Recording"), "{error}");
        assert!(permission_error(true).is_none());
    }
}
