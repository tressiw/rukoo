//! The picture of a frame on macOS: a `CVPixelBuffer` backed by an IOSurface, which is what
//! ScreenCaptureKit hands over and VideoToolbox takes. The synthetic screen makes BGRA ones;
//! ScreenCaptureKit sends `420f`, bi-planar full-range YCbCr, which the encoder takes as it is and
//! the JPEG and OCR paths convert to BGRA from. It travels from the capture's thread to the main
//! thread and on to the encoder's, so it is declared sendable: a pixel buffer is only written
//! before it is handed over and only read after. Core Video's lock pins the memory for the CPU;
//! it isn't a mutex. Readers take it read-only, which leaves the surface's caches valid while the
//! encoder reads the same IOSurface.

use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::{
    kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_709_2,
    kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2,
    kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_709_2,
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelFormatType_32BGRA,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, CVAttachmentMode, CVImageBuffer,
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetPlaneCount, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
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
        PixelBuffer::create(width, height, kCVPixelFormatType_32BGRA)
    }

    /// A pixel buffer of `width` x `height` in `format`, IOSurface-backed.
    fn create(width: u32, height: u32, format: u32) -> Option<PixelBuffer> {
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
                format,
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

    /// A `420f` pixel buffer holding `pixels`, BGRA of any size, as ScreenCaptureKit delivers a
    /// frame: bi-planar full-range YCbCr with the BT.709 matrix, primaries and transfer function
    /// attached. Each chroma sample is the mean of its two by two luma pixels, fewer at an odd
    /// edge. The synthetic screen's `format=420f` and the tests make the real capture's frames
    /// with it; the video writer primes its encoder with a black one.
    pub fn from_pixels_420f(pixels: &Pixels) -> Option<PixelBuffer> {
        let (width, height) = (pixels.width as usize, pixels.height as usize);
        let buffer = PixelBuffer::create(
            pixels.width,
            pixels.height,
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        )?;
        {
            let locked = buffer.lock()?;
            let (luma, luma_row) = locked.plane_mut(0)?;
            let (chroma, chroma_row) = locked.plane_mut(1)?;
            let rgb = |x: usize, y: usize| -> (f32, f32, f32) {
                let at = (y * width + x) * 4;
                let px = &pixels.bgra[at..at + 4];
                (f32::from(px[2]), f32::from(px[1]), f32::from(px[0]))
            };
            for y in 0..height {
                for x in 0..width {
                    let (r, g, b) = rgb(x, y);
                    // SAFETY: plane 0 has `height` rows of `luma_row` bytes, at least `width`.
                    unsafe { *luma.add(y * luma_row + x) = to_byte(luma_709(r, g, b)) };
                }
            }
            for cy in 0..height.div_ceil(2) {
                for cx in 0..width.div_ceil(2) {
                    let (mut cb, mut cr, mut n) = (0f32, 0f32, 0f32);
                    for y in cy * 2..(cy * 2 + 2).min(height) {
                        for x in cx * 2..(cx * 2 + 2).min(width) {
                            let (r, g, b) = rgb(x, y);
                            let luma = luma_709(r, g, b);
                            cb += (b - luma) / 1.8556;
                            cr += (r - luma) / 1.5748;
                            n += 1.0;
                        }
                    }
                    // SAFETY: plane 1 has `height` / 2 rounded up rows of `chroma_row` bytes, at
                    // least two a pair of luma columns.
                    unsafe {
                        let at = chroma.add(cy * chroma_row + cx * 2);
                        *at = to_byte(cb / n + 128.0);
                        *at.add(1) = to_byte(cr / n + 128.0);
                    }
                }
            }
        }
        buffer.attach_709();
        Some(buffer)
    }

    /// A black `420f` buffer of `width` x `height`, attached as `from_pixels_420f`'s are: luma 0
    /// and chroma 128.
    pub fn black_420f(width: u32, height: u32) -> Option<PixelBuffer> {
        let buffer = PixelBuffer::create(
            width,
            height,
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        )?;
        {
            let locked = buffer.lock()?;
            for (plane, value, rows) in [
                (0, 0u8, height as usize),
                (1, 128, (height as usize).div_ceil(2)),
            ] {
                let (base, row) = locked.plane_mut(plane)?;
                // SAFETY: the plane has `rows` rows of `row` bytes.
                unsafe { std::ptr::write_bytes(base, value, rows * row) };
            }
        }
        buffer.attach_709();
        Some(buffer)
    }

    /// Says on the buffer that it is BT.709 YCbCr, as ScreenCaptureKit's buffers say with the
    /// matrix the capture asks for, so VideoToolbox signals it in the stream.
    fn attach_709(&self) {
        // SAFETY: the keys and values are Core Video's static strings, and the buffer retains
        // them.
        unsafe {
            for (key, value) in [
                (
                    kCVImageBufferYCbCrMatrixKey,
                    kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                ),
                (
                    kCVImageBufferColorPrimariesKey,
                    kCVImageBufferColorPrimaries_ITU_R_709_2,
                ),
                (
                    kCVImageBufferTransferFunctionKey,
                    kCVImageBufferTransferFunction_ITU_R_709_2,
                ),
            ] {
                let value: &CFType = value.as_ref();
                self.buffer
                    .set_attachment(key, value, CVAttachmentMode::ShouldPropagate);
            }
        }
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

    /// Locks the buffer's memory for reading and writing until the guard drops. Only a buffer
    /// that hasn't been handed over yet is written.
    pub fn lock(&self) -> Option<Locked<'_>> {
        self.lock_with(CVPixelBufferLockFlags(0))
    }

    /// Locks the buffer's memory for reading only until the guard drops. `CVPixelBuffer.h`: a
    /// reader should say so, "to avoid potentially invalidating any existing caches of the buffer
    /// contents".
    pub fn lock_read(&self) -> Option<Locked<'_>> {
        self.lock_with(CVPixelBufferLockFlags::ReadOnly)
    }

    fn lock_with(&self, flags: CVPixelBufferLockFlags) -> Option<Locked<'_>> {
        // SAFETY: the lock is balanced by the guard's `Drop`, with the same flags.
        if unsafe { CVPixelBufferLockBaseAddress(&self.buffer, flags) } != 0 {
            return None;
        }
        // A bi-planar buffer has no single base address: its planes are read one by one.
        let base = CVPixelBufferGetBaseAddress(&self.buffer) as *mut u8;
        if base.is_null() && CVPixelBufferGetPlaneCount(&self.buffer) == 0 {
            // SAFETY: the buffer is locked above, with these flags.
            unsafe { CVPixelBufferUnlockBaseAddress(&self.buffer, flags) };
            return None;
        }
        Some(Locked {
            buffer: &self.buffer,
            base,
            bytes_per_row: CVPixelBufferGetBytesPerRow(&self.buffer),
            flags,
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
        let locked = self.lock_read()?;
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

/// BT.709 luma of red, green and blue.
fn luma_709(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn to_byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
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
    /// The lock's flags, which the unlock must repeat.
    flags: CVPixelBufferLockFlags,
}

impl Locked<'_> {
    pub fn base(&self) -> *const u8 {
        self.base
    }

    /// The memory to write, under a lock taken with `lock`.
    pub fn base_mut(&mut self) -> *mut u8 {
        debug_assert!(
            !self.flags.contains(CVPixelBufferLockFlags::ReadOnly),
            "a write under a read-only lock"
        );
        self.base
    }

    pub fn bytes_per_row(&self) -> usize {
        self.bytes_per_row
    }

    /// A plane's first byte to write and its row length, under a lock taken with `lock`.
    pub fn plane_mut(&self, index: usize) -> Option<(*mut u8, usize)> {
        debug_assert!(
            !self.flags.contains(CVPixelBufferLockFlags::ReadOnly),
            "a write under a read-only lock"
        );
        self.plane(index).map(|(base, row)| (base.cast_mut(), row))
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
        // SAFETY: the buffer is locked until this drop, and the flags are the lock's.
        unsafe { CVPixelBufferUnlockBaseAddress(self.buffer, self.flags) };
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

    /// A picture of coloured blocks, two by two pixels each so chroma subsampling keeps them:
    /// the primaries, grey levels and a few mixes.
    fn blocks(width: u32, height: u32) -> Pixels {
        const COLOURS: [[u8; 3]; 8] = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 255],
            [0, 0, 0],
            [128, 128, 128],
            [200, 120, 40],
            [30, 90, 210],
        ];
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = COLOURS[((x / 2 + y / 2 * 3) % 8) as usize];
                bgra.extend_from_slice(&[b, g, r, 0xff]);
            }
        }
        Pixels {
            bgra,
            width,
            height,
        }
    }

    fn max_difference(a: &Pixels, b: &Pixels) -> u8 {
        assert_eq!((a.width, a.height), (b.width, b.height));
        a.bgra
            .chunks_exact(4)
            .zip(b.bgra.chunks_exact(4))
            .flat_map(|(a, b)| (0..3).map(move |i| a[i].abs_diff(b[i])))
            .max()
            .unwrap_or(0)
    }

    /// The bi-planar path, which every real frame takes: a `420f` picture reads back as the BGRA
    /// it was made from, up to rounding. Red stays red and blue stays blue, so a swapped Cb and Cr
    /// fails here, and the odd size has padded rows and a half chroma pair at the edge.
    #[test]
    fn a_420f_picture_reads_back_its_pixels() {
        for (width, height) in [(64, 48), (1281, 721)] {
            let pixels = blocks(width, height);
            let picture = PixelBuffer::from_pixels_420f(&pixels).unwrap();
            assert_eq!(
                picture.pixel_format(),
                kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
            );
            assert!(picture.is_biplanar() && picture.is_full_range());
            {
                let locked = picture.lock_read().unwrap();
                let (_, luma_row) = locked.plane(0).unwrap();
                assert!(luma_row >= width as usize);
            }
            let read = picture.pixels().unwrap();
            let worst = max_difference(&read, &pixels);
            assert!(worst <= 3, "{width}x{height}: off by {worst}");
            // The first block is pure red: B, G, R.
            assert!(
                read.bgra[2] >= 250 && read.bgra[0] <= 3,
                "{:?}",
                &read.bgra[..4]
            );
        }
        // An odd edge mixes nothing it doesn't have: a 3 x 3 blue picture stays blue.
        let blue = Pixels {
            bgra: [255u8, 0, 0, 255].repeat(9),
            width: 3,
            height: 3,
        };
        let read = PixelBuffer::from_pixels_420f(&blue)
            .unwrap()
            .pixels()
            .unwrap();
        assert!(max_difference(&read, &blue) <= 3);
    }

    /// The video writer's priming frame: black in full range, luma 0 and chroma 128.
    #[test]
    fn a_black_420f_picture_is_black() {
        let picture = PixelBuffer::black_420f(64, 36).unwrap();
        assert!(picture.is_biplanar());
        let read = picture.pixels().unwrap();
        assert!(read.bgra.chunks_exact(4).all(|px| px[..3] == [0, 0, 0]));
    }

    #[test]
    fn a_zero_sized_picture_is_refused() {
        assert!(PixelBuffer::new(0, 10).is_none());
    }
}
