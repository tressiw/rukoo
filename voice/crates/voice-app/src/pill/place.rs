//! Where the pill window and its stop window go, in physical pixels. Pure, so the tests cover every
//! scale, taskbar edge and monitor layout (pill-settings-design.md 3.3).

use serde::Deserialize;
#[cfg(any(windows, target_os = "macos", test))]
use voice_engine::Edge;

/// The pill window in logical pixels (voice/ui/API.md). The pill draws itself inside it with room for its
/// shadow: 24 px from the top and 40 px from the bottom.
pub const WINDOW_W: f64 = 384.0;
pub const WINDOW_H: f64 = 100.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

// The five helpers below place the pill window and its button windows on Windows; nothing else calls
// them yet, so they are built for Windows and for the tests, which check every scale, taskbar edge
// and monitor layout on every platform. The macOS pill will use them too.

/// `dpi / 96`; 0 (an unknown DPI) counts as 100 %.
#[cfg(any(windows, test))]
pub fn scale_for_dpi(dpi: u32) -> f64 {
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / 96.0
    }
}

/// The pill window's size at `scale`.
#[cfg(any(windows, test))]
pub fn window_size(scale: f64) -> (i32, i32) {
    (
        (WINDOW_W * scale).round() as i32,
        (WINDOW_H * scale).round() as i32,
    )
}

/// Centred on the work area, with its bottom on the work area's bottom (the pill then floats 40 px above
/// the taskbar) or its top on the work area's top. The work area leaves out the taskbar wherever it is.
#[cfg(any(windows, test))]
pub fn pill_window_rect(work: &Rect, scale: f64, edge: Edge) -> Rect {
    let (w, h) = window_size(scale);
    let left = work.left + (work.width() - w) / 2;
    let top = match edge {
        Edge::Bottom => work.bottom - h,
        Edge::Top => work.top,
    };
    Rect {
        left,
        top,
        right: left + w,
        bottom: top + h,
    }
}

/// A rectangle from `pill_layout`, in CSS pixels from the pill window's top left.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// `pill_layout`'s payload (voice/ui/API.md).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Layout {
    pub seq: u64,
    pub pill: Option<CssRect>,
    pub button: Option<CssRect>,
    /// The screen button, left of the stop button, while the pill shows it.
    #[serde(default, rename = "screenButton")]
    pub screen_button: Option<CssRect>,
}

/// Where the stop window goes on screen: the pill window's client origin plus the button scaled to
/// physical pixels, rounded outwards so the hit area never falls short of the drawn button. `None` for an
/// empty or nonsensical rectangle.
#[cfg(any(windows, test))]
pub fn css_to_physical(origin: (i32, i32), scale: f64, r: &CssRect) -> Option<Rect> {
    let values = [r.x, r.y, r.width, r.height, scale];
    if values.iter().any(|v| !v.is_finite()) || r.width <= 0.0 || r.height <= 0.0 || scale <= 0.0 {
        return None;
    }
    let left = (r.x * scale).floor() as i32;
    let top = (r.y * scale).floor() as i32;
    let right = ((r.x + r.width) * scale).ceil() as i32;
    let bottom = ((r.y + r.height) * scale).ceil() as i32;
    Some(Rect {
        left: origin.0 + left,
        top: origin.1 + top,
        right: origin.0 + right,
        bottom: origin.1 + bottom,
    })
}

// ---------- macOS: AppKit points, origin at the primary display's bottom left, y up ----------
//
// The macOS pill places itself with the helpers below. They take the screens as plain numbers, so the
// tests cover one display, two, a notch, a side Dock and a display that went away on every platform.

/// A rectangle in AppKit's screen space: points, `y` growing upwards, `(x, y)` its bottom left.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[cfg(any(target_os = "macos", test))]
impl Frame {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Frame {
        Frame { x, y, w, h }
    }

    pub fn top(&self) -> f64 {
        self.y + self.h
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Half-open, as AppKit hit-tests: the left and bottom edges are inside, the others aren't.
    pub fn contains(&self, (x, y): (f64, f64)) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.top()
    }

    fn overlap(&self, other: &Frame) -> f64 {
        let w = self.right().min(other.right()) - self.x.max(other.x);
        let h = self.top().min(other.top()) - self.y.max(other.y);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }

    /// The square of the distance from `point` to the nearest point of this rectangle.
    fn distance2(&self, (x, y): (f64, f64)) -> f64 {
        let dx = (self.x - x).max(0.0).max(x - self.right());
        let dy = (self.y - y).max(0.0).max(y - self.top());
        dx * dx + dy * dy
    }
}

/// One display as `NSScreen` describes it.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    /// The display's `CGDirectDisplayID` (`NSScreenNumber`), which stays the same while displays are
    /// rearranged.
    pub id: u32,
    pub frame: Frame,
    /// The frame without the menu bar and the Dock.
    pub visible: Frame,
    /// `safeAreaInsets.top` and `.bottom`: a notch, on a built-in display whose menu bar is hidden.
    pub safe_top: f64,
    pub safe_bottom: f64,
}

/// The pill window on `screen`: centred on the visible frame, so a Dock on the left or right is left
/// out, with its bottom on the visible frame's bottom (above the Dock) or its top under the menu bar
/// and the notch, whichever reaches lower. The page draws the pill 24 pt from the window's top and
/// 40 pt from its bottom, as on Windows.
#[cfg(any(target_os = "macos", test))]
pub fn mac_pill_frame(screen: &Screen, edge: Edge) -> Frame {
    let (w, h) = (WINDOW_W, WINDOW_H);
    let x = screen.visible.x + (screen.visible.w - w) / 2.0;
    // The menu bar's and the notch's bottom: whichever is lower. A hidden menu bar leaves the visible
    // frame's top at the screen's, and the notch is then the safe area alone.
    let ceiling = screen
        .visible
        .top()
        .min(screen.frame.top() - screen.safe_top.max(0.0));
    let floor = screen
        .visible
        .y
        .max(screen.frame.y + screen.safe_bottom.max(0.0));
    let y = match edge {
        Edge::Bottom => floor,
        Edge::Top => ceiling - h,
    };
    // A screen too short for both: under the menu bar wins, so the pill is never behind it.
    let y = y.min(ceiling - h);
    Frame::new(x.round(), y.round(), w, h)
}

/// The screen that holds `point`.
#[cfg(any(target_os = "macos", test))]
pub fn screen_at(screens: &[Screen], point: (f64, f64)) -> Option<usize> {
    screens.iter().position(|s| s.frame.contains(point))
}

/// The display the user works on: the one the front window overlaps most (or the nearest, for a
/// window off every display), else the pointer's, else the primary display (the first `NSScreen`, the
/// one with the menu bar).
#[cfg(any(target_os = "macos", test))]
pub fn choose_screen(
    screens: &[Screen],
    front: Option<Frame>,
    mouse: Option<(f64, f64)>,
) -> Option<usize> {
    if screens.is_empty() {
        return None;
    }
    front
        .and_then(|f| screen_for_frame(screens, &f))
        .or_else(|| mouse.and_then(|p| screen_at(screens, p)))
        .or(Some(0))
}

/// The smallest window the front-window lookup takes for the one the user works in, in points:
/// smaller ones are an app's helper windows (a drag image, a tracking overlay).
#[cfg(any(target_os = "macos", test))]
pub const MIN_FRONT_WINDOW: f64 = 40.0;

/// Whether a window-list entry can be the window the user works in: not fully transparent
/// (`kCGWindowAlpha` 0, missing counts as opaque) and not tiny.
#[cfg(any(target_os = "macos", test))]
pub fn is_user_window(alpha: Option<f64>, frame: &Frame) -> bool {
    alpha.is_none_or(|a| a > 0.0) && frame.w >= MIN_FRONT_WINDOW && frame.h >= MIN_FRONT_WINDOW
}

/// The screen a window is on: the one it overlaps most, else (its display is gone, or it sits in a
/// gap after a rearrangement) the one nearest its centre.
#[cfg(any(target_os = "macos", test))]
pub fn screen_for_frame(screens: &[Screen], frame: &Frame) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (i, s) in screens.iter().enumerate() {
        let area = s.frame.overlap(frame);
        if area > 0.0 && best.is_none_or(|(_, a)| area > a) {
            best = Some((i, area));
        }
    }
    if let Some((i, _)) = best {
        return Some(i);
    }
    let centre = frame.centre();
    screens
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.frame
                .distance2(centre)
                .total_cmp(&b.frame.distance2(centre))
        })
        .map(|(i, _)| i)
}

/// The display a show goes to after a display change: the one the pill was placed on while it is
/// still there, else the one nearest the window.
#[cfg(any(target_os = "macos", test))]
pub fn screen_after_change(screens: &[Screen], last: Option<u32>, window: &Frame) -> Option<usize> {
    last.and_then(|id| screens.iter().position(|s| s.id == id))
        .or_else(|| screen_for_frame(screens, window))
}

/// A window's bounds from `CGWindowListCopyWindowInfo` (top left origin, `y` down from the primary
/// display's top) in AppKit's space.
#[cfg(any(target_os = "macos", test))]
pub fn cg_to_appkit(primary_height: f64, x: f64, y: f64, w: f64, h: f64) -> Frame {
    Frame::new(x, primary_height - y - h, w, h)
}

/// Where a rectangle of the pill's page is on screen. The page measures in CSS pixels from the
/// window's top left, and a CSS pixel is a point on macOS, so only `y` turns around. Rounded outwards
/// so the click area never falls short of the drawn button. `None` for an empty or broken rectangle.
#[cfg(any(target_os = "macos", test))]
pub fn css_to_points(window: &Frame, r: &CssRect) -> Option<Frame> {
    let values = [r.x, r.y, r.width, r.height, window.x, window.y, window.h];
    if values.iter().any(|v| !v.is_finite()) || r.width <= 0.0 || r.height <= 0.0 {
        return None;
    }
    let left = (window.x + r.x).floor();
    let right = (window.x + r.x + r.width).ceil();
    let top = (window.top() - r.y).ceil();
    let bottom = (window.top() - r.y - r.height).floor();
    Some(Frame::new(left, bottom, right - left, top - bottom))
}

/// How far a macOS button panel reaches past its drawn button, in CSS pixels (points): a near miss
/// still hits the button instead of falling through to the wallpaper, where a click reveals the desktop.
#[cfg(any(target_os = "macos", test))]
pub const BUTTON_MARGIN: f64 = 6.0;
/// Left of the first button the reach stops at half of `pill.css`'s 8px between the pill's text and
/// that button, so a click on the text never stops the meeting.
#[cfg(any(target_os = "macos", test))]
pub const LABEL_CLEARANCE: f64 = 4.0;
/// How far the inert panel under the buttons reaches past the drawn pill.
#[cfg(any(target_os = "macos", test))]
pub const BODY_MARGIN: f64 = 6.0;

/// One button's panel on macOS: where it takes clicks, and the drawn button inside it, which alone
/// shows hover and pressed. In CSS pixels from the pill window's top left, as `pill_layout` sends.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitArea {
    pub hit: CssRect,
    pub button: CssRect,
}

/// The panels over the pill on macOS: the two buttons' and the inert one over the drawn pill.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitAreas {
    pub stop: Option<HitArea>,
    pub screen: Option<HitArea>,
    pub body: Option<CssRect>,
}

#[cfg(any(target_os = "macos", test))]
fn usable(r: &CssRect) -> bool {
    [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite()) && r.width > 0.0 && r.height > 0.0
}

/// The macOS panels for `layout`. Each button reaches `BUTTON_MARGIN` past its edges, except towards
/// the other button, where the gap between them is split in half so the two never overlap, and
/// towards the text on its left, where it stops at `LABEL_CLEARANCE`. The body is the drawn pill with
/// `BODY_MARGIN` around it.
#[cfg(any(target_os = "macos", test))]
pub fn hit_areas(layout: &Layout) -> HitAreas {
    let stop = layout.button.filter(usable);
    let screen = layout.screen_button.filter(usable);
    let area = |button: CssRect, other: Option<CssRect>| {
        let right_edge = button.x + button.width;
        let mut left = LABEL_CLEARANCE;
        let mut right = BUTTON_MARGIN;
        if let Some(other) = other {
            if other.x < button.x {
                left = ((button.x - (other.x + other.width)) / 2.0).max(0.0);
            } else {
                right = ((other.x - right_edge) / 2.0).max(0.0);
            }
        }
        HitArea {
            hit: CssRect {
                x: button.x - left,
                y: button.y - BUTTON_MARGIN,
                width: button.width + left + right,
                height: button.height + 2.0 * BUTTON_MARGIN,
            },
            button,
        }
    };
    HitAreas {
        stop: stop.map(|b| area(b, screen)),
        screen: screen.map(|b| area(b, stop)),
        body: layout.pill.filter(usable).map(|p| CssRect {
            x: p.x - BODY_MARGIN,
            y: p.y - BODY_MARGIN,
            width: p.width + 2.0 * BODY_MARGIN,
            height: p.height + 2.0 * BODY_MARGIN,
        }),
    }
}

/// Windows that are the desktop or the taskbar, not an app the user dictates into.
#[cfg(any(windows, test))]
pub fn is_shell_class(class: &str) -> bool {
    matches!(
        class,
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn every_common_scale_centres_the_window_above_a_bottom_taskbar() {
        // 1920x1080 with a 48 px taskbar at 100 %; at higher scales the taskbar is taller in pixels.
        for (dpi, w, h) in [
            (96, 384, 100),
            (120, 480, 125),
            (144, 576, 150),
            (168, 672, 175),
            (192, 768, 200),
        ] {
            let scale = scale_for_dpi(dpi);
            let work = rect(0, 0, 1920, 1080 - (48.0 * scale) as i32);
            let r = pill_window_rect(&work, scale, Edge::Bottom);
            assert_eq!((r.width(), r.height()), (w, h), "dpi {dpi}");
            assert_eq!(r.bottom, work.bottom, "dpi {dpi}");
            assert_eq!(r.left, (1920 - w) / 2, "dpi {dpi}");
        }
    }

    #[test]
    fn a_taskbar_on_any_edge_is_respected() {
        let scale = 1.5;
        // Left taskbar: the work area starts at x = 72.
        let r = pill_window_rect(&rect(72, 0, 2560, 1440), scale, Edge::Bottom);
        assert_eq!(
            r,
            rect(
                72 + (2488 - 576) / 2,
                1440 - 150,
                72 + (2488 - 576) / 2 + 576,
                1440
            )
        );
        // Right taskbar.
        let r = pill_window_rect(&rect(0, 0, 2488, 1440), scale, Edge::Bottom);
        assert_eq!(r.right, (2488 - 576) / 2 + 576);
        // Top taskbar with the pill at the top: it sits right under the taskbar.
        let r = pill_window_rect(&rect(0, 72, 2560, 1440), scale, Edge::Top);
        assert_eq!((r.top, r.bottom), (72, 72 + 150));
        // Auto-hide: the work area is the whole monitor.
        let r = pill_window_rect(&rect(0, 0, 2560, 1440), 1.0, Edge::Bottom);
        assert_eq!(r.bottom, 1440);
    }

    #[test]
    fn monitors_left_of_or_above_the_primary_have_negative_coordinates() {
        let r = pill_window_rect(&rect(-1920, -312, 0, 768), 1.0, Edge::Bottom);
        assert_eq!(r, rect(-1920 + 768, 668, -1920 + 768 + 384, 768));
        let r = pill_window_rect(&rect(-1080, -1920, 0, -48), 1.25, Edge::Top);
        assert_eq!(
            r,
            rect(
                -1080 + (1080 - 480) / 2,
                -1920,
                -1080 + (1080 - 480) / 2 + 480,
                -1920 + 125
            )
        );
    }

    #[test]
    fn a_portrait_monitor_narrower_than_usual_still_centres() {
        let r = pill_window_rect(&rect(1920, 0, 2520, 1024), 2.0, Edge::Bottom);
        // 768 px wide window on a 600 px wide work area: centred, overhanging both sides equally.
        assert_eq!((r.left, r.right), (1920 - 84, 1920 - 84 + 768));
    }

    #[test]
    fn the_button_scales_to_physical_pixels_and_rounds_outwards() {
        let button = CssRect {
            x: 257.0,
            y: 28.0,
            width: 28.0,
            height: 28.0,
        };
        assert_eq!(
            css_to_physical((100, 900), 1.0, &button),
            Some(rect(357, 928, 385, 956))
        );
        assert_eq!(
            css_to_physical((0, 0), 1.5, &button),
            Some(rect(385, 42, 428, 84))
        );
        // 125 %: 257 * 1.25 = 321.25 -> 321; (257 + 28) * 1.25 = 356.25 -> 357.
        assert_eq!(
            css_to_physical((-1920, 0), 1.25, &button),
            Some(rect(-1920 + 321, 35, -1920 + 357, 70))
        );
        // The widened "Stop?" button at 175 %.
        let wide = CssRect {
            x: 233.5,
            y: 28.0,
            width: 52.5,
            height: 28.0,
        };
        assert_eq!(
            css_to_physical((0, 0), 1.75, &wide),
            Some(rect(408, 49, 501, 98))
        );
    }

    #[test]
    fn empty_or_broken_rectangles_place_nothing() {
        let zero = CssRect {
            x: 10.0,
            y: 10.0,
            width: 0.0,
            height: 28.0,
        };
        assert_eq!(css_to_physical((0, 0), 1.0, &zero), None);
        let nan = CssRect {
            x: f64::NAN,
            y: 0.0,
            width: 28.0,
            height: 28.0,
        };
        assert_eq!(css_to_physical((0, 0), 1.0, &nan), None);
        let ok = CssRect {
            x: 0.0,
            y: 0.0,
            width: 28.0,
            height: 28.0,
        };
        assert_eq!(css_to_physical((0, 0), 0.0, &ok), None);
    }

    #[test]
    fn dpi_zero_counts_as_one_hundred_percent() {
        assert_eq!(scale_for_dpi(0), 1.0);
        assert_eq!(scale_for_dpi(96), 1.0);
        assert_eq!(scale_for_dpi(240), 2.5);
    }

    #[test]
    fn the_layout_payload_parses_as_the_page_sends_it() {
        let layout: Layout = serde_json::from_str(
            r#"{ "seq": 418, "pill": { "x": 96, "y": 24, "width": 193, "height": 36 }, "button": { "x": 257, "y": 28, "width": 28, "height": 28 } }"#,
        )
        .unwrap();
        assert_eq!(layout.seq, 418);
        assert_eq!(layout.button.unwrap().x, 257.0);
        let none: Layout =
            serde_json::from_str(r#"{ "seq": 3, "pill": null, "button": null }"#).unwrap();
        assert!(none.pill.is_none() && none.button.is_none());
    }

    #[test]
    fn the_desktop_and_taskbar_are_not_where_the_user_works() {
        for class in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ] {
            assert!(is_shell_class(class), "{class}");
        }
        for class in ["Chrome_WidgetWin_1", "Notepad", "OpusApp", ""] {
            assert!(!is_shell_class(class), "{class}");
        }
    }

    // ---------- macOS ----------

    /// A 1920x1080 display at the origin with a 30 pt menu bar and a 60 pt Dock at the bottom.
    fn mac_display(id: u32, x: f64, y: f64) -> Screen {
        Screen {
            id,
            frame: Frame::new(x, y, 1920.0, 1080.0),
            visible: Frame::new(x, y + 60.0, 1920.0, 1080.0 - 60.0 - 30.0),
            safe_top: 0.0,
            safe_bottom: 0.0,
        }
    }

    /// A 14" MacBook Pro: 1512x982 points, a 38 pt menu bar the notch's height, the Dock hidden.
    fn notched(id: u32) -> Screen {
        Screen {
            id,
            frame: Frame::new(0.0, 0.0, 1512.0, 982.0),
            visible: Frame::new(0.0, 0.0, 1512.0, 982.0 - 38.0),
            safe_top: 32.0,
            safe_bottom: 0.0,
        }
    }

    #[test]
    fn mac_one_display_centres_the_pill_above_the_dock_and_under_the_menu_bar() {
        let screen = mac_display(1, 0.0, 0.0);
        let bottom = mac_pill_frame(&screen, Edge::Bottom);
        assert_eq!(bottom, Frame::new(768.0, 60.0, 384.0, 100.0));
        let top = mac_pill_frame(&screen, Edge::Top);
        assert_eq!(top, Frame::new(768.0, 1080.0 - 30.0 - 100.0, 384.0, 100.0));
        assert!(top.top() <= screen.visible.top());
    }

    #[test]
    fn mac_the_spike_display_lands_where_the_spike_measured() {
        // S4: visible frame (0, 90) 1920x960 with a 30 pt menu bar put the window at (768, 90).
        let screen = Screen {
            id: 1,
            frame: Frame::new(0.0, 0.0, 1920.0, 1080.0),
            visible: Frame::new(0.0, 90.0, 1920.0, 960.0),
            safe_top: 0.0,
            safe_bottom: 0.0,
        };
        assert_eq!(
            mac_pill_frame(&screen, Edge::Bottom),
            Frame::new(768.0, 90.0, 384.0, 100.0)
        );
    }

    #[test]
    fn mac_a_notch_keeps_a_top_pill_under_it_even_with_the_menu_bar_hidden() {
        let screen = notched(1);
        let top = mac_pill_frame(&screen, Edge::Top);
        assert_eq!(top.top(), 982.0 - 38.0);
        // The menu bar hidden: the visible frame reaches the top, the safe area still holds the notch.
        let hidden = Screen {
            visible: Frame::new(0.0, 0.0, 1512.0, 982.0),
            ..screen
        };
        let top = mac_pill_frame(&hidden, Edge::Top);
        assert_eq!(top.top(), 982.0 - 32.0);
        assert_eq!(top.x, (1512.0_f64 - 384.0) / 2.0);
        // At the bottom the notch changes nothing.
        assert_eq!(mac_pill_frame(&hidden, Edge::Bottom).y, 0.0);
    }

    #[test]
    fn mac_a_side_dock_is_left_out_of_the_centring() {
        let screen = Screen {
            visible: Frame::new(80.0, 0.0, 1840.0, 1050.0),
            ..mac_display(1, 0.0, 0.0)
        };
        let frame = mac_pill_frame(&screen, Edge::Bottom);
        assert_eq!(frame.x, 80.0 + (1840.0 - 384.0) / 2.0);
        assert_eq!(frame.y, 0.0);
    }

    #[test]
    fn mac_a_bottom_safe_area_lifts_the_pill() {
        let screen = Screen {
            visible: Frame::new(0.0, 0.0, 1920.0, 1050.0),
            safe_bottom: 20.0,
            ..mac_display(1, 0.0, 0.0)
        };
        assert_eq!(mac_pill_frame(&screen, Edge::Bottom).y, 20.0);
    }

    #[test]
    fn mac_a_screen_too_short_keeps_the_pill_under_the_menu_bar() {
        let screen = Screen {
            id: 1,
            frame: Frame::new(0.0, 0.0, 800.0, 120.0),
            visible: Frame::new(0.0, 60.0, 800.0, 30.0),
            safe_top: 0.0,
            safe_bottom: 0.0,
        };
        let frame = mac_pill_frame(&screen, Edge::Bottom);
        assert_eq!(frame.top(), 90.0);
    }

    #[test]
    fn mac_two_displays_the_front_window_decides_then_the_pointer_then_the_primary() {
        // A second display to the left and lower: negative x, its own Dock-less visible frame.
        let left = Screen {
            id: 2,
            frame: Frame::new(-2560.0, -300.0, 2560.0, 1440.0),
            visible: Frame::new(-2560.0, -300.0, 2560.0, 1440.0),
            safe_top: 0.0,
            safe_bottom: 0.0,
        };
        let screens = [mac_display(1, 0.0, 0.0), left];
        let on_left = Frame::new(-1400.0, 0.0, 800.0, 600.0);
        assert_eq!(
            choose_screen(&screens, Some(on_left), Some((10.0, 10.0))),
            Some(1)
        );
        // Straddling both: the display with most of it, wherever its centre is.
        let straddling = Frame::new(-300.0, 100.0, 1000.0, 600.0);
        assert_eq!(
            choose_screen(&screens, Some(straddling), Some((-5.0, 0.0))),
            Some(0)
        );
        assert_eq!(choose_screen(&screens, None, Some((-5.0, 0.0))), Some(1));
        // A front window off every display goes to the nearest one, not the pointer's.
        let above_left = Frame::new(-2000.0, 1300.0, 400.0, 300.0);
        assert_eq!(
            choose_screen(&screens, Some(above_left), Some((100.0, 100.0))),
            Some(1)
        );
        assert_eq!(choose_screen(&screens, None, None), Some(0));
        assert_eq!(choose_screen(&[], None, None), None);
        let frame = mac_pill_frame(&screens[1], Edge::Bottom);
        assert_eq!(frame, Frame::new(-2560.0 + 1088.0, -300.0, 384.0, 100.0));
    }

    #[test]
    fn mac_a_display_going_away_moves_the_pill_to_the_nearest_one() {
        let main = mac_display(1, 0.0, 0.0);
        let right = mac_display(2, 1920.0, 0.0);
        let on_right = mac_pill_frame(&right, Edge::Bottom);
        // Both there: the display it was placed on.
        assert_eq!(
            screen_after_change(&[main, right], Some(2), &on_right),
            Some(1)
        );
        // The right display unplugged: the pill's frame overlaps nothing; the nearest is the main one.
        assert_eq!(screen_after_change(&[main], Some(2), &on_right), Some(0));
        // Rearranged: the right display now sits above the main one. Its id still finds it, whatever
        // the old frame overlaps now.
        let above = mac_display(2, 0.0, 1080.0);
        assert_eq!(
            screen_after_change(&[main, above], Some(2), &on_right),
            Some(1)
        );
        // No record of the last display: the one the window overlaps most.
        let straddling = Frame::new(1800.0, 60.0, 384.0, 100.0);
        assert_eq!(
            screen_after_change(&[main, right], None, &straddling),
            Some(1)
        );
        assert_eq!(screen_for_frame(&[], &straddling), None);
    }

    #[test]
    fn mac_transparent_and_tiny_windows_are_not_the_front_window() {
        let window = Frame::new(0.0, 0.0, 800.0, 600.0);
        assert!(is_user_window(Some(1.0), &window));
        assert!(is_user_window(None, &window));
        assert!(is_user_window(Some(0.5), &window));
        assert!(!is_user_window(Some(0.0), &window));
        assert!(!is_user_window(
            Some(1.0),
            &Frame::new(0.0, 0.0, 20.0, 600.0)
        ));
        assert!(!is_user_window(
            Some(1.0),
            &Frame::new(0.0, 0.0, 800.0, 1.0)
        ));
        assert!(is_user_window(Some(1.0), &Frame::new(0.0, 0.0, 40.0, 40.0)));
    }

    #[test]
    fn mac_window_list_bounds_turn_into_appkit_space() {
        // A 800x600 window 100 pt from the top of a 1080 pt primary display.
        let frame = cg_to_appkit(1080.0, 50.0, 100.0, 800.0, 600.0);
        assert_eq!(frame, Frame::new(50.0, 380.0, 800.0, 600.0));
        // On a display above the primary, CG's y is negative.
        let frame = cg_to_appkit(1080.0, 0.0, -900.0, 400.0, 300.0);
        assert_eq!(frame.y, 1680.0);
    }

    #[test]
    fn mac_the_buttons_turn_into_screen_points_rounded_outwards() {
        let window = Frame::new(768.0, 60.0, 384.0, 100.0);
        let stop = CssRect {
            x: 257.0,
            y: 28.0,
            width: 28.0,
            height: 28.0,
        };
        // Top of the window is 160; the button spans y 28..56 from the top, so 104..132 on screen.
        assert_eq!(
            css_to_points(&window, &stop),
            Some(Frame::new(1025.0, 104.0, 28.0, 28.0))
        );
        // The widened "Stop?" button, at fractional CSS pixels.
        let wide = CssRect {
            x: 233.5,
            y: 28.25,
            width: 52.5,
            height: 28.0,
        };
        assert_eq!(
            css_to_points(&window, &wide),
            Some(Frame::new(1001.0, 103.0, 53.0, 29.0))
        );
        // The screen button left of it, on a display left of the primary.
        let screen = CssRect {
            x: 225.0,
            y: 28.0,
            width: 28.0,
            height: 28.0,
        };
        let left = Frame::new(-1792.0, -300.0, 384.0, 100.0);
        assert_eq!(
            css_to_points(&left, &screen),
            Some(Frame::new(-1567.0, -256.0, 28.0, 28.0))
        );
        for broken in [
            CssRect { width: 0.0, ..stop },
            CssRect {
                x: f64::NAN,
                ..stop
            },
            CssRect {
                height: -1.0,
                ..stop
            },
        ] {
            assert_eq!(css_to_points(&window, &broken), None);
        }
    }

    fn css(x: f64, y: f64, width: f64, height: f64) -> CssRect {
        CssRect {
            x,
            y,
            width,
            height,
        }
    }

    fn overlap(a: &CssRect, b: &CssRect) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    }

    /// A meeting pill as the page lays it out: pill 96..289 x 24..60, the screen button 4px left of
    /// the stop button, both 4px inside the pill's right and vertical edges, the text ending 8px left
    /// of the screen button.
    fn meeting_layout() -> Layout {
        Layout {
            seq: 1,
            pill: Some(css(96.0, 24.0, 193.0, 36.0)),
            button: Some(css(257.0, 28.0, 28.0, 28.0)),
            screen_button: Some(css(225.0, 28.0, 28.0, 28.0)),
        }
    }

    #[test]
    fn mac_the_buttons_reach_past_their_edges_without_meeting_or_covering_the_text() {
        let areas = hit_areas(&meeting_layout());
        let stop = areas.stop.unwrap();
        let screen = areas.screen.unwrap();
        // The stop button: 6 above, below and right; half the 4px gap on its left.
        assert_eq!(stop.hit, css(255.0, 22.0, 36.0, 40.0));
        assert_eq!(stop.button, css(257.0, 28.0, 28.0, 28.0));
        // The screen button: half the gap on its right, 4 of the 8px to the text on its left.
        assert_eq!(screen.hit, css(221.0, 22.0, 34.0, 40.0));
        // They touch and don't overlap.
        assert_eq!(screen.hit.x + screen.hit.width, stop.hit.x);
        assert!(!overlap(&stop.hit, &screen.hit));
        // The text ends at 225 - 8 = 217; nothing reaches it.
        assert!(screen.hit.x >= 217.0);
        // Each drawn button is inside its own area.
        for a in [stop, screen] {
            assert!(a.hit.x <= a.button.x && a.hit.y <= a.button.y);
            assert!(a.hit.x + a.hit.width >= a.button.x + a.button.width);
            assert!(a.hit.y + a.hit.height >= a.button.y + a.button.height);
        }
        // The body is the drawn pill and a margin, under both.
        assert_eq!(areas.body, Some(css(90.0, 18.0, 205.0, 48.0)));
    }

    #[test]
    fn mac_the_stop_button_alone_keeps_clear_of_the_text() {
        let layout = Layout {
            screen_button: None,
            ..meeting_layout()
        };
        let areas = hit_areas(&layout);
        assert_eq!(areas.stop.unwrap().hit, css(253.0, 22.0, 38.0, 40.0));
        assert_eq!(areas.screen, None);
        // The widened "Stop?" button moves its left edge; the reach follows it.
        let wide = Layout {
            button: Some(css(233.5, 28.0, 52.5, 28.0)),
            ..layout
        };
        let hit = hit_areas(&wide).stop.unwrap().hit;
        assert_eq!((hit.x, hit.width), (229.5, 62.5));
    }

    #[test]
    fn mac_touching_or_overlapping_buttons_split_nothing() {
        let layout = Layout {
            button: Some(css(250.0, 28.0, 28.0, 28.0)),
            screen_button: Some(css(225.0, 28.0, 28.0, 28.0)),
            ..meeting_layout()
        };
        let areas = hit_areas(&layout);
        let (stop, screen) = (areas.stop.unwrap().hit, areas.screen.unwrap().hit);
        assert_eq!(stop.x, 250.0);
        assert_eq!(screen.x + screen.width, 253.0);
    }

    #[test]
    fn mac_a_dictation_pill_has_only_the_body_and_broken_rectangles_nothing() {
        let layout = Layout {
            seq: 2,
            pill: Some(css(130.0, 24.0, 124.0, 36.0)),
            button: None,
            screen_button: None,
        };
        let areas = hit_areas(&layout);
        assert_eq!((areas.stop, areas.screen), (None, None));
        assert!(areas.body.is_some());
        let broken = Layout {
            seq: 3,
            pill: Some(css(f64::NAN, 0.0, 10.0, 10.0)),
            button: Some(css(0.0, 0.0, 0.0, 28.0)),
            screen_button: None,
        };
        assert_eq!(hit_areas(&broken), HitAreas::default());
    }
}
