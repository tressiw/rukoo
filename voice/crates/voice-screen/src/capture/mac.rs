//! The picture of a frame on macOS: a `CVPixelBuffer` backed by an IOSurface, which is what
//! ScreenCaptureKit hands over and VideoToolbox takes. The synthetic screen makes BGRA ones;
//! ScreenCaptureKit sends `420f`, bi-planar full-range YCbCr, which the encoder takes as it is and
//! the JPEG and OCR paths convert to BGRA from. It travels from the capture's thread to the main
//! thread and on to the encoder's, so it is declared sendable: a pixel buffer is only read after
//! it was handed over, and Core Video's locks serialize the accesses anyway.

use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_core_video::{
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelFormatType_32BGRA,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, CVImageBuffer, CVPixelBuffer,
    CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBaseAddressOfPlane,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetPlaneCount, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};

use crate::platform::Pixels;

/// A frame's pixels in a `CVPixelBuffer`.
#[derive(Clone)]
pub struct PixelBuffer {
    buffer: CFRetained<CVPixelBuffer>,
}

// SAFETY: Core Video objects are reference counted and free-threaded. The orchestration moves a
// picture to the encoder's thread and reads it there while the main thread no longer touches it.
unsafe impl Send for PixelBuffer {}

impl PixelBuffer {
    /// The picture a capture handed over, in the format it came in.
    pub fn from_image(buffer: CFRetained<CVImageBuffer>) -> PixelBuffer {
        PixelBuffer { buffer }
    }

    /// A BGRA pixel buffer of `width` x `height`, IOSurface-backed, as VideoToolbox wants the
    /// frames it encodes. `None` when Core Video can't allocate it.
    pub fn new(width: u32, height: u32) -> Option<PixelBuffer> {
        if width == 0 || height == 0 {
            return None;
        }
        // An empty IOSurface property dictionary asks for an IOSurface-backed buffer.
        let surface = CFDictionary::<CFString, CFString>::empty();
        // SAFETY: the key is the property Core Video documents for IOSurface backing.
        let key: &CFString = unsafe { kCVPixelBufferIOSurfacePropertiesKey };
        let attributes = CFDictionary::from_slices(&[key], &[&*surface]);
        let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
        // SAFETY: the arguments describe the buffer and `out` receives a retained buffer.
        let status = unsafe {
            CVPixelBufferCreate(
                None,
                width as usize,
                height as usize,
                kCVPixelFormatType_32BGRA,
                Some(opaque(&attributes)),
                NonNull::new(&mut out).expect("a pointer to the out parameter"),
            )
        };
        if status != 0 {
            return None;
        }
        let buffer = unsafe { CFRetained::from_raw(NonNull::new(out)?) };
        Some(PixelBuffer { buffer })
    }

    /// A pixel buffer holding `pixels`, which must be BGRA of the same size.
    pub fn from_pixels(pixels: &Pixels) -> Option<PixelBuffer> {
        let buffer = PixelBuffer::new(pixels.width, pixels.height)?;
        {
            let mut locked = buffer.lock()?;
            let row = pixels.width as usize * 4;
            for y in 0..pixels.height as usize {
                // SAFETY: the destination has `height` rows of `bytes_per_row` and the source
                // holds `height` rows of `width * 4`.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        pixels.bgra.as_ptr().add(y * row),
                        locked.base_mut().add(y * locked.bytes_per_row()),
                        row,
                    );
                }
            }
        }
        Some(buffer)
    }

    /// The Core Video buffer itself, for VideoToolbox and Vision.
    pub fn buffer(&self) -> &CVPixelBuffer {
        &self.buffer
    }

    /// Sets every pixel to `bgra`, alphas included.
    pub fn fill(&self, bgra: [u8; 4]) {
        let Some(mut locked) = self.lock() else {
            return;
        };
        let row = self.width() as usize * 4;
        for y in 0..self.height() as usize {
            let start = unsafe { locked.base_mut().add(y * locked.bytes_per_row()) };
            // SAFETY: the row is at least `row` bytes long inside the locked buffer.
            let pixels = unsafe { std::slice::from_raw_parts_mut(start, row) };
            for px in pixels.chunks_exact_mut(4) {
                px.copy_from_slice(&bgra);
            }
        }
    }

    pub fn width(&self) -> u32 {
        CVPixelBufferGetWidth(&self.buffer) as u32
    }

    pub fn height(&self) -> u32 {
        CVPixelBufferGetHeight(&self.buffer) as u32
    }

    /// The Core Video pixel format, `32BGRA` for the synthetic screen.
    pub fn pixel_format(&self) -> u32 {
        CVPixelBufferGetPixelFormatType(&self.buffer)
    }

    /// Locks the buffer's memory for reading and writing until the guard drops.
    pub fn lock(&self) -> Option<Locked<'_>> {
        // SAFETY: the lock is balanced by the guard's `Drop`.
        if unsafe { CVPixelBufferLockBaseAddress(&self.buffer, CVPixelBufferLockFlags(0)) } != 0 {
            return None;
        }
        // A bi-planar buffer has no single base address: its planes are read one by one.
        let base = CVPixelBufferGetBaseAddress(&self.buffer) as *mut u8;
        if base.is_null() && CVPixelBufferGetPlaneCount(&self.buffer) == 0 {
            // SAFETY: the buffer is locked above.
            unsafe { CVPixelBufferUnlockBaseAddress(&self.buffer, CVPixelBufferLockFlags(0)) };
            return None;
        }
        Some(Locked {
            buffer: &self.buffer,
            base,
            bytes_per_row: CVPixelBufferGetBytesPerRow(&self.buffer),
        })
    }

    /// Whether this picture is bi-planar luma and chroma, as ScreenCaptureKit sends it.
    pub fn is_biplanar(&self) -> bool {
        let format = self.pixel_format();
        format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            || format == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
    }

    /// Whether a bi-planar picture uses the whole 0 to 255 range for luma (`420f`), not 16 to 235.
    pub fn is_full_range(&self) -> bool {
        self.pixel_format() != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
    }

    /// The picture's pixels, BGRA and rows top-down without padding. A bi-planar picture is
    /// converted, with the BT.709 matrix the capture asks the stream to use.
    pub fn pixels(&self) -> Option<Pixels> {
        let locked = self.lock()?;
        let (width, height) = (self.width(), self.height());
        let row = width as usize * 4;
        let mut bgra = vec![0u8; row * height as usize];
        if self.is_biplanar() {
            let full = self.is_full_range();
            let (luma, luma_row) = locked.plane(0)?;
            let (chroma, chroma_row) = locked.plane(1)?;
            for row_y in 0..height as usize {
                // SAFETY: plane 0 has `height` rows of at least `width` bytes, and plane 1 has
                // half as many rows of two bytes (Cb, Cr) per two luma pixels, so a row of
                // `width` rounded up to even bytes.
                let luma = unsafe {
                    std::slice::from_raw_parts(luma.add(row_y * luma_row), width as usize)
                };
                let chroma = unsafe {
                    std::slice::from_raw_parts(
                        chroma.add((row_y / 2) * chroma_row),
                        (width as usize).div_ceil(2) * 2,
                    )
                };
                let out = &mut bgra[row_y * row..(row_y + 1) * row];
                for (x, (&luma, out)) in luma.iter().zip(out.chunks_exact_mut(4)).enumerate() {
                    let pair = x / 2 * 2;
                    out.copy_from_slice(&ycbcr_to_bgra(luma, chroma[pair], chroma[pair + 1], full));
                }
            }
        } else {
            for y in 0..height as usize {
                // SAFETY: both buffers have `height` rows of at least `row` bytes.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        locked.base().add(y * locked.bytes_per_row()),
                        bgra.as_mut_ptr().add(y * row),
                        row,
                    );
                }
            }
        }
        Some(Pixels {
            bgra,
            width,
            height,
        })
    }
}

/// One picture element of the BT.709 conversion: luma, blue-difference and red-difference, to
/// BGRA. `full` says whether the luma is full range (`420f`) or video range (`420v`).
fn ycbcr_to_bgra(y: u8, cb: u8, cr: u8, full: bool) -> [u8; 4] {
    let (cb, cr) = (i32::from(cb) - 128, i32::from(cr) - 128);
    let y = if full {
        i32::from(y)
    } else {
        // 1.164 with the video-range offset, in 1/256ths.
        (298 * (i32::from(y) - 16) + 128) >> 8
    };
    let r = y + ((403 * cr + 128) >> 8);
    let g = y - ((48 * cb + 120 * cr + 128) >> 8);
    let b = y + ((475 * cb + 128) >> 8);
    [
        b.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        r.clamp(0, 255) as u8,
        0xff,
    ]
}

impl std::fmt::Debug for PixelBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture {}x{}", self.width(), self.height())
    }
}

/// A locked pixel buffer's memory. Unlocks when dropped.
pub struct Locked<'a> {
    buffer: &'a CVPixelBuffer,
    base: *mut u8,
    bytes_per_row: usize,
}

impl Locked<'_> {
    pub fn base(&self) -> *const u8 {
        self.base
    }

    pub fn base_mut(&mut self) -> *mut u8 {
        self.base
    }

    pub fn bytes_per_row(&self) -> usize {
        self.bytes_per_row
    }

    /// A plane's first byte and its row length, for a bi-planar picture. `None` when the plane
    /// isn't there.
    pub fn plane(&self, index: usize) -> Option<(*const u8, usize)> {
        if index >= CVPixelBufferGetPlaneCount(self.buffer) {
            return None;
        }
        // SAFETY: the buffer is locked and the plane is inside it.
        let base = CVPixelBufferGetBaseAddressOfPlane(self.buffer, index) as *const u8;
        if base.is_null() {
            return None;
        }
        // SAFETY: the call takes a plane inside the locked buffer.
        let row = CVPixelBufferGetBytesPerRowOfPlane(self.buffer, index);
        Some((base, row))
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        // SAFETY: the buffer is locked until this drop, and the flags match the lock's.
        unsafe { CVPixelBufferUnlockBaseAddress(self.buffer, CVPixelBufferLockFlags(0)) };
    }
}

/// A `CFDictionary` of concrete key and value types as the untyped one Core Video's API takes:
/// every instantiation has the same layout, only the Rust-side types differ.
fn opaque<K: Sized, V: Sized>(dict: &CFDictionary<K, V>) -> &CFDictionary {
    // SAFETY: the two types are the same Core Foundation object.
    unsafe { &*(dict as *const CFDictionary<K, V> as *const CFDictionary) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixel_buffer_reads_back_its_pixels() {
        let pixels = Pixels {
            bgra: (0..(64 * 48 * 4)).map(|n| (n % 251) as u8).collect(),
            width: 64,
            height: 48,
        };
        let picture = PixelBuffer::from_pixels(&pixels).unwrap();
        assert_eq!((picture.width(), picture.height()), (64, 48));
        assert_eq!(picture.pixel_format(), kCVPixelFormatType_32BGRA);
        let read = picture.pixels().unwrap();
        for y in 0..48usize {
            assert_eq!(
                &read.bgra[y * 64 * 4..(y + 1) * 64 * 4],
                &pixels.bgra[y * 64 * 4..(y + 1) * 64 * 4],
                "row {y}"
            );
        }
        // Clones share the buffer.
        let clone = picture.clone();
        assert_eq!(clone.width(), 64);
    }

    #[test]
    fn a_zero_sized_picture_is_refused() {
        assert!(PixelBuffer::new(0, 10).is_none());
    }
}
