//! The synthetic screen of tests and live runs on macOS (`--test synthetic:…`): numbered slides
//! drawn on the CPU with Core Graphics and Core Text into a `CVPixelBuffer`, so nothing of the
//! real desktop is ever captured. Like ScreenCaptureKit it sends frames only when the picture
//! changes: a short burst at each new slide, one frame per keystroke with `type-every`, and
//! nothing in between, unless `idle-every` sends the unchanged picture again as a real screen
//! can on a still screen. With `dirty` each frame says what share of the screen changed since
//! the frame before, as the real capture's dirty regions do.
//!
//! The monitor ids are the Windows synthetic screen's (`\\.\SYNTHETIC1` …), so the crash tests
//! pin and switch monitors with one spec on both platforms. They never name a real display.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use objc2_core_foundation::{
    CFAttributedString, CFDictionary, CFRetained, CFString, CFType, CGAffineTransform,
};
use objc2_core_graphics::{CGColorSpace, CGContext};
use objc2_core_text::{kCTFontAttributeName, CTFont, CTLine};
use voice_screen_files::monitors::MonitorInfo;

use super::mac::PixelBuffer;
use crate::args::TestSpec;
use crate::clock_mac;
use crate::platform::{Frame, Pixels, PlatformError, Source};
use crate::record::Ev;

/// Frames sent after each change, 66 ms apart, as a real screen sends while a slide draws.
const BURST: u32 = 5;
/// A keystroke's mark: a dark square this many pixels wide, on the 8-pixel grid of the 1/8-scale
/// thumbnail, so it darkens two by two thumbnail pixels.
const KEY: u32 = 16;
/// The distance from one mark to the next, and between rows.
const KEY_STEP: u32 = 24;
/// Rows of marks before typing starts again at the first.
const KEY_ROWS: u32 = 4;
const FRAME_GAP: Duration = Duration::from_millis(66);
/// Without `text=`, slides read "Screen <n>".
const DEFAULT_WORD: &str = "Screen";

/// The synthetic monitors: `\\.\SYNTHETIC1` … side by side, the first primary, each the size of
/// the spec.
pub fn monitors(spec: &TestSpec) -> Vec<MonitorInfo> {
    (1..=spec.monitors.max(1))
        .map(|n| MonitorInfo {
            id: format!(r"\\.\SYNTHETIC{n}"),
            name: format!("Synthetic {n}"),
            x: ((n - 1) * spec.width) as i32,
            y: 0,
            width: spec.width,
            height: spec.height,
            primary: n == 1,
        })
        .collect()
}

/// The slides count on one clock for the whole process, so a monitor switch or a new device
/// doesn't start them again at 1.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

pub struct SyntheticSource {
    to_main: SyncSender<Ev<PixelBuffer>>,
    spec: TestSpec,
    worker: Option<(JoinHandle<()>, Arc<AtomicBool>)>,
}

pub fn new(to_main: SyncSender<Ev<PixelBuffer>>, spec: &TestSpec) -> SyntheticSource {
    epoch();
    SyntheticSource {
        to_main,
        spec: spec.clone(),
        worker: None,
    }
}

impl Source for SyntheticSource {
    fn start(&mut self, monitor: &MonitorInfo, generation: u64) -> Result<(), PlatformError> {
        self.stop();
        let index: u32 = monitor
            .id
            .trim_start_matches(r"\\.\SYNTHETIC")
            .parse()
            .unwrap_or(1);
        let stop = Arc::new(AtomicBool::new(false));
        let (to_main, spec, flag) = (self.to_main.clone(), self.spec.clone(), stop.clone());
        let handle = std::thread::Builder::new()
            .name("synthetic-screen".into())
            .spawn(move || run(to_main, spec, index, generation, flag))
            .map_err(|e| PlatformError::new(format!("can't start the synthetic screen: {e}")))?;
        self.worker = Some((handle, stop));
        Ok(())
    }

    fn stop(&mut self) {
        if let Some((handle, stop)) = self.worker.take() {
            stop.store(true, Ordering::SeqCst);
            let _ = handle.join();
        }
    }
}

impl Drop for SyntheticSource {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    to_main: SyncSender<Ev<PixelBuffer>>,
    spec: TestSpec,
    monitor: u32,
    generation: u64,
    stop: Arc<AtomicBool>,
) {
    let (width, height) = (spec.width & !1, spec.height & !1);
    let Some(mut canvas) = Canvas::new(width, height) else {
        crate::log("the synthetic screen couldn't make its canvas");
        return;
    };
    let word = spec.text.clone().unwrap_or_else(|| DEFAULT_WORD.into());
    let slide_at = |now: Instant| match spec.change_every_ms {
        Some(every) => 1 + now.duration_since(epoch()).as_millis() as u64 / every,
        None => 1,
    };
    let mut shown = 0u64;
    let mut burst = 0u32;
    // Keystrokes typed on this slide, when the next is due, and the share of the screen that
    // changed since the last frame sent.
    let mut typed = 0u32;
    let mut next_key = spec
        .type_every_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    let mut dirty = 0f32;
    let mut last_sent: Option<Instant> = None;
    while !stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        let slide = slide_at(now);
        if slide != shown {
            shown = slide;
            canvas.draw(&format!("{word} {slide}"), slide, monitor);
            typed = 0;
            burst = BURST;
            dirty = 1.0;
        }
        if let (Some(every), Some(at)) = (spec.type_every_ms, next_key) {
            if now >= at {
                next_key = Some(at + Duration::from_millis(every));
                dirty = (dirty + canvas.type_key(typed)).min(1.0);
                typed += 1;
                burst = burst.max(1);
            }
        }
        let idle = spec.idle_every_ms.is_some_and(|every| {
            last_sent.is_some_and(|at| now - at >= Duration::from_millis(every))
        });
        if burst == 0 && idle {
            burst = 1;
        }
        if burst > 0 {
            burst -= 1;
            last_sent = Some(now);
            let picture = PixelBuffer::from_pixels(&canvas.pixels());
            let Some(picture) = picture else {
                // The buffer couldn't be allocated: the graphics device is the closest thing to it.
                let _ = to_main.try_send(Ev::DeviceLost(generation));
                return;
            };
            let frame = Frame {
                picture,
                boot: clock_mac::boot_100ns(),
                width,
                height,
                dirty: spec.dirty.then_some(dirty),
                generation,
            };
            dirty = 0.0;
            if to_main.try_send(Ev::Frame(frame)).is_err() {
                return;
            }
            std::thread::sleep(FRAME_GAP);
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A top-down BGRA canvas to draw slides into.
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Option<Canvas> {
        if width < 16 || height < 16 {
            return None;
        }
        Some(Canvas {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        })
    }

    /// Black text on white, centered, under a band whose colour says which monitor this is, over
    /// a row of eight cells that show the slide number in binary. A new slide flips at least one
    /// cell, 2% of the screen, so it counts as a change the way a real slide does; one changed
    /// digit alone stays under the saver's 1% threshold.
    fn draw(&mut self, text: &str, slide: u64, monitor: u32) {
        const BANDS: [(u8, u8, u8); 4] = [
            (0x2d, 0x7a, 0xb5),
            (0x3b, 0x9a, 0x30),
            (0xc7, 0x4f, 0x2f),
            (0x8e, 0x3a, 0x80),
        ];
        // White everywhere, opaque.
        for px in self.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        }
        let (r, g, b) = BANDS[(monitor.max(1) as usize - 1) % BANDS.len()];
        self.fill(0, 0, self.width, self.height / 12, (r, g, b));
        let cell = self.width / 8;
        for bit in 0..8 {
            if slide >> bit & 1 == 1 {
                self.fill(
                    (7 - bit) * cell,
                    self.height * 5 / 6,
                    cell,
                    self.height - self.height * 5 / 6,
                    (0x40, 0x40, 0x40),
                );
            }
        }
        self.text(text);
    }

    /// Fills a rectangle, clipped to the canvas, with an opaque colour.
    fn fill(&mut self, x: u32, y: u32, width: u32, height: u32, (r, g, b): (u8, u8, u8)) {
        let right = (x + width).min(self.width);
        let bottom = (y + height).min(self.height);
        for row in y..bottom {
            let start = (row as usize * self.width as usize + x as usize) * 4;
            let end = (row as usize * self.width as usize + right as usize) * 4;
            for px in self.pixels[start..end].chunks_exact_mut(4) {
                px.copy_from_slice(&[b, g, r, 0xff]);
            }
        }
    }

    /// The centered line, in bold black, as big as the Windows synthetic screen's font.
    fn text(&mut self, text: &str) {
        let Some(context) = self.context() else {
            return;
        };
        let size = f64::from((self.height / 8).max(16));
        let font = unsafe {
            CTFont::with_name(
                &CFString::from_static_str("Helvetica-Bold"),
                size,
                std::ptr::null(),
            )
        };
        let attributes: CFRetained<CFDictionary<CFString, CFType>> =
            CFDictionary::from_slices(&[unsafe { kCTFontAttributeName }], &[as_cf_type(&*font)]);
        let string = CFString::from_str(text);
        // SAFETY: the attributed string and the line retain their parts for their lifetime.
        let Some(attributed) =
            (unsafe { CFAttributedString::new(None, Some(&string), Some(opaque(&attributes))) })
        else {
            return;
        };
        let line = unsafe { CTLine::with_attributed_string(&attributed) };
        // The context runs top-down like the canvas; the text matrix turns the glyphs upright.
        CGContext::translate_ctm(Some(&context), 0.0, f64::from(self.height));
        CGContext::scale_ctm(Some(&context), 1.0, -1.0);
        CGContext::set_rgb_fill_color(Some(&context), 0.0, 0.0, 0.0, 1.0);
        CGContext::set_text_matrix(
            Some(&context),
            CGAffineTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
        let (mut ascent, mut descent, mut leading) = (0.0, 0.0, 0.0);
        // SAFETY: the pointers are to three initialized doubles.
        let width = unsafe { line.typographic_bounds(&mut ascent, &mut descent, &mut leading) };
        let x = (f64::from(self.width) - width).max(0.0) / 2.0;
        let y = (f64::from(self.height) + ascent + descent) / 2.0;
        CGContext::set_text_position(Some(&context), x, y);
        // SAFETY: the line and the context live until the call returns.
        unsafe { line.draw(&context) };
    }

    /// A context drawing into this canvas, path-down: fills and text land where the canvas does.
    fn context(&mut self) -> Option<CFRetained<CGContext>> {
        let space = CGColorSpace::new_device_rgb()?;
        // SAFETY: the context writes into the canvas for as long as it lives, which is this call.
        let context = unsafe {
            CGBitmapContextCreate(
                self.pixels.as_mut_ptr().cast(),
                self.width as usize,
                self.height as usize,
                8,
                self.width as usize * 4,
                &space,
                // kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little: BGRA bytes.
                0x2002,
            )
        };
        let context = NonNull::new(context)?;
        // SAFETY: the call returns a context with a reference of its own.
        Some(unsafe { CFRetained::from_raw(context) })
    }

    /// Types keystroke `n` of the slide: a dark square in a row under the text, the next one
    /// along. Returns the share of the screen it covers.
    fn type_key(&mut self, n: u32) -> f32 {
        let per_row = (self.width.saturating_sub(2 * 40) / KEY_STEP).max(1);
        let left = 40 + (n % per_row) * KEY_STEP;
        let top = ((self.height * 2 / 3) & !7) + (n / per_row % KEY_ROWS) * KEY_STEP;
        let (right, bottom) = ((left + KEY).min(self.width), (top + KEY).min(self.height));
        if right <= left || bottom <= top {
            return 0.0;
        }
        self.fill(left, top, right - left, bottom - top, (0x20, 0x20, 0x20));
        ((right - left) * (bottom - top)) as f32 / (self.width * self.height) as f32
    }

    fn pixels(&self) -> Pixels {
        Pixels {
            bgra: self.pixels.clone(),
            width: self.width,
            height: self.height,
        }
    }
}

extern "C" {
    /// A bitmap context over `data`, which `context` keeps using until it is released.
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: &CGColorSpace,
        bitmap_info: u32,
    ) -> *mut CGContext;
}

/// A `CFDictionary` of concrete types as the untyped one the Core APIs take.
fn opaque<K: Sized, V: Sized>(dict: &CFDictionary<K, V>) -> &CFDictionary {
    // SAFETY: every CFDictionary is the same Core Foundation object.
    unsafe { &*(dict as *const CFDictionary<K, V> as *const CFDictionary) }
}

/// A Core Foundation object as the untyped `CFType` an attribute dictionary takes.
fn as_cf_type<T: Sized>(value: &T) -> &CFType {
    // SAFETY: only CF objects are handed to this, whose layout starts with the CF type header.
    unsafe { &*(value as *const T as *const CFType) }
}

/// A slide of the synthetic screen as pixels, for the tests of the pieces that read pictures.
#[cfg(test)]
pub(crate) fn slide(text: &str, number: u64, width: u32, height: u32) -> Option<Pixels> {
    let mut canvas = Canvas::new(width, height)?;
    canvas.draw(text, number, 1);
    Some(canvas.pixels())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slide_is_drawn_white_with_a_band() {
        let mut canvas = Canvas::new(320, 180).unwrap();
        canvas.draw("Slide 2", 2, 1);
        let pixels = canvas.pixels();
        // The band's red at the top, a white middle, and a lit cell for bit 1 at the bottom.
        assert_eq!(&pixels.bgra[..4], &[0xb5, 0x7a, 0x2d, 0xff]);
        let middle = (90 * 320 + 10) * 4;
        assert_eq!(&pixels.bgra[middle..middle + 3], &[0xff, 0xff, 0xff]);
        // Slide 2 has bit 1 set: the second cell from the right is dark.
        let cell = 320 / 8;
        let lit = (170 * 320 + (7 - 1) * cell + 2) * 4;
        assert_eq!(&pixels.bgra[lit..lit + 3], &[0x40, 0x40, 0x40]);
        assert!(pixels.bgra.chunks_exact(4).all(|px| px[3] == 0xff));
    }

    #[test]
    fn a_keystroke_covers_its_share() {
        let mut canvas = Canvas::new(1280, 720).unwrap();
        let share = canvas.type_key(0);
        // 16 x 16 pixels of 1280 x 720.
        assert!((share - 0.000_278).abs() < 0.000_01, "{share}");
        // The next mark is 24 pixels along.
        assert_eq!(canvas.pixels().bgra[(480 * 1280 + 40 + 8) * 4], 0x20);
    }

    /// The point of the synthetic screen's text: a text reader reads it. Checked where the
    /// platform's reader is (`frames::ocr_vision`).
    #[test]
    fn the_synthetic_monitors_are_the_windows_ones() {
        let spec = TestSpec {
            width: 640,
            height: 360,
            monitors: 2,
            ..TestSpec::default()
        };
        let monitors = monitors(&spec);
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].id, r"\\.\SYNTHETIC1");
        assert_eq!(monitors[1].id, r"\\.\SYNTHETIC2");
        assert_eq!((monitors[1].x, monitors[1].y), (640, 0));
        assert!(monitors[0].primary && !monitors[1].primary);
    }
}
