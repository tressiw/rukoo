//! The 1/8-scale luma thumbnail of a frame, averaged on the CPU: the saver and the video gate
//! only need a small picture of how the screen changed, and a screen-sized buffer is cheap to
//! read once per frame at most five times a second. The luma is the same BT.601 mix the Windows
//! thumbnailer's GPU path produces, so a threshold set there means the same here.

use voice_screen_files::frames::Thumb;

use crate::capture::mac::PixelBuffer;
use crate::platform::PlatformError;

pub struct Thumbnailer {
    width: u32,
    height: u32,
}

impl Thumbnailer {
    /// A thumbnailer for frames of `width` x `height`.
    pub fn new(width: u32, height: u32) -> Thumbnailer {
        Thumbnailer { width, height }
    }

    /// The frame size this thumbnailer takes.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The thumbnail of `picture`, a frame of this thumbnailer's size.
    pub fn thumb(&mut self, picture: &PixelBuffer) -> Result<Thumb, PlatformError> {
        let (width, height) = (self.width as usize, self.height as usize);
        let (thumb_width, thumb_height) = ((width / 8).max(1), (height / 8).max(1));
        let locked = picture
            .lock()
            .ok_or_else(|| PlatformError::new("can't read the frame for a thumbnail"))?;
        let row = locked.bytes_per_row();
        let base = locked.base();
        let mut luma = vec![0u8; thumb_width * thumb_height];
        for ty in 0..thumb_height {
            let top = ty * 8;
            let bottom = ((ty + 1) * 8).min(height);
            for tx in 0..thumb_width {
                let left = tx * 8;
                let right = ((tx + 1) * 8).min(width);
                let mut sum = 0u32;
                let mut count = 0u32;
                for y in top..bottom {
                    // SAFETY: the row is inside the locked buffer, which has `height` rows of
                    // `row` bytes and at least `width * 4` bytes in each.
                    let pixels =
                        unsafe { std::slice::from_raw_parts(base.add(y * row), right * 4) };
                    for px in pixels[left * 4..].chunks_exact(4) {
                        sum += u32::from(luma_of(px[0], px[1], px[2]));
                        count += 1;
                    }
                }
                luma[ty * thumb_width + tx] = ((sum + count / 2) / count.max(1)) as u8;
            }
        }
        Ok(Thumb {
            width: thumb_width as u32,
            height: thumb_height as u32,
            luma,
        })
    }
}

/// BT.601 luma from BGRA, as the Windows thumbnail's GPU path mixes it.
pub fn luma_of(b: u8, g: u8, r: u8) -> u8 {
    ((u32::from(b) * 29 + u32::from(g) * 150 + u32::from(r) * 77) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Pixels;

    #[test]
    fn luma_of_primaries() {
        assert_eq!(luma_of(0, 0, 0), 0);
        assert_eq!(luma_of(255, 255, 255), 255);
        assert!(luma_of(0, 255, 0) > luma_of(0, 0, 255));
        assert!(luma_of(0, 0, 255) > luma_of(255, 0, 0));
    }

    /// A white frame with a black band thumbnails to light cells and some dark ones.
    #[test]
    fn thumbnails_a_frame() {
        let (w, h) = (640u32, 360u32);
        let mut pixels = vec![255u8; (w * h * 4) as usize];
        for y in 96..136 {
            for x in 0..w as usize {
                let i = (y * w as usize + x) * 4;
                pixels[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let picture = PixelBuffer::from_pixels(&Pixels {
            bgra: pixels,
            width: w,
            height: h,
        })
        .unwrap();
        let mut thumbs = Thumbnailer::new(w, h);
        assert_eq!(thumbs.size(), (w, h));
        let thumb = thumbs.thumb(&picture).unwrap();
        assert_eq!((thumb.width, thumb.height), (80, 45));
        assert_eq!(thumb.luma[0], 255);
        let dark = thumb.luma.iter().filter(|&&l| l < 64).count();
        assert_eq!(dark, 80 * 5, "rows 96..136 are cells 12..17");
    }
}
