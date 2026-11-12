//! The monitors as Core Graphics lists them: every active display with its pixel size and its
//! place in the desktop. The id is the `CGDirectDisplayID`, which ScreenCaptureKit names a
//! display by too, so `--monitor` and the future capture agree on one monitor.
//!
//! The window Z-order and the cursor's monitor come with ScreenCaptureKit: nothing records the
//! real screen before it, so `record` refuses a real screen and the choice falls back to the
//! primary display.

use objc2_core_foundation::CGRect;
use voice_screen_files::monitors::MonitorInfo;

extern "C" {
    /// Fills `active` with the ids of the displays that are on, and writes how many there are.
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    /// The rotation in degrees: 0, 90, 180 or 270.
    fn CGDisplayRotation(display: u32) -> f64;
}

/// Every active display, left to right, the main one marked primary.
pub fn list() -> Vec<MonitorInfo> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    // SAFETY: the call writes at most `ids.len()` ids and the count it wrote.
    if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0 {
        return Vec::new();
    }
    let count = (count as usize).min(ids.len());
    let mut monitors: Vec<MonitorInfo> = ids[..count]
        .iter()
        .enumerate()
        .map(|(n, &id)| {
            let bounds = unsafe { CGDisplayBounds(id) };
            // SAFETY: the calls take the display id and answer with its mode's size and rotation.
            let (mut width, mut height) = unsafe {
                (
                    CGDisplayPixelsWide(id) as u32,
                    CGDisplayPixelsHigh(id) as u32,
                )
            };
            let rotation = unsafe { CGDisplayRotation(id) };
            if (rotation - 90.0).abs() < 1.0 || (rotation - 270.0).abs() < 1.0 {
                std::mem::swap(&mut width, &mut height);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
