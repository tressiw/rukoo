//! A `CGImage` of a frame's BGRA bytes, for the JPEG encoder and Vision: both take Core Graphics
//! images, and neither needs a bitmap context to get one.

use objc2_core_foundation::{CFData, CFDictionary, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo,
};

use crate::platform::Pixels;

/// `kCGBitmapByteOrder32Little` with an alpha first in each word: BGRA bytes, as a
/// `CVPixelBuffer` holds its `32BGRA` pixels.
const BGRA: u32 = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;

/// The pixels as an 8-bit BGRA image. The image keeps the data alive for as long as it lives.
pub fn from_pixels(pixels: &Pixels) -> Option<CFRetained<CGImage>> {
    if pixels.width == 0 || pixels.height == 0 {
        return None;
    }
    let data = CFData::from_bytes(&pixels.bgra);
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let space = CGColorSpace::new_device_rgb()?;
    // SAFETY: the provider holds the pixels, and the layout matches `BGRA`.
    unsafe {
        CGImage::new(
            pixels.width as usize,
            pixels.height as usize,
            8,
            32,
            pixels.width as usize * 4,
            Some(&space),
            objc2_core_graphics::CGBitmapInfo(BGRA),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

/// A `CFDictionary` of concrete types as the untyped one the Core APIs take.
pub fn opaque<K: Sized, V: Sized>(dict: &CFDictionary<K, V>) -> &CFDictionary {
    // SAFETY: every CFDictionary is the same Core Foundation object.
    unsafe { &*(dict as *const CFDictionary<K, V> as *const CFDictionary) }
}

/// A Core Foundation object as the untyped `CFType` an attribute dictionary takes.
pub fn as_cf_type<T: Sized>(value: &T) -> &CFType {
    // SAFETY: only CF objects are handed to this.
    unsafe { &*(value as *const T as *const CFType) }
}

/// A Core Foundation string for a constant the framework hands out as one.
pub fn static_string(text: &str) -> CFRetained<CFString> {
    CFString::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_has_the_frames_size() {
        let pixels = Pixels {
            bgra: vec![0xff; 64 * 32 * 4],
            width: 64,
            height: 32,
        };
        let image = from_pixels(&pixels).expect("a CGImage");
        assert_eq!(CGImage::width(Some(&image)), 64);
        assert_eq!(CGImage::height(Some(&image)), 32);
    }
}
