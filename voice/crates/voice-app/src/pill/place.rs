//! Where the pill window and its stop window go, in physical pixels. Pure, so the tests cover every
//! scale, taskbar edge and monitor layout (pill-settings-design.md 3.3).

use serde::Deserialize;
#[cfg(any(windows, test))]
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
}
