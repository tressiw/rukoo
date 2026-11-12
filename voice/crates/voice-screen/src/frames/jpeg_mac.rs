//! A saved frame's pixels as a JPEG through ImageIO, and the way back for frames a crash left
//! without text. ImageIO takes a `CGImage` of the BGRA bytes, so no bitmap context is needed.

use objc2_core_foundation::{CFDictionary, CFMutableData, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::CGImage;
use objc2_image_io::{
    kCGImageDestinationLossyCompressionQuality, CGImageDestination, CGImageSource,
};
use voice_screen_files::frames::{JPEG_MAX_SIDE, JPEG_QUALITY};

use super::cgimage::{as_cf_type, opaque};
use super::upscale::scale_to;
use crate::platform::Pixels;

/// The size a JPEG gets: the frame's own, unless its longest side is over `JPEG_MAX_SIDE`.
pub fn jpeg_size(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= JPEG_MAX_SIDE {
        return (width, height);
    }
    let scale = |side: u32| {
        ((u64::from(side) * u64::from(JPEG_MAX_SIDE) / u64::from(longest)) as u32).max(1)
    };
    (scale(width), scale(height))
}

/// The JPEG at quality `JPEG_QUALITY`, scaled to `jpeg_size`, and its width and height.
pub fn encode(pixels: &Pixels) -> Result<(Vec<u8>, u32, u32), String> {
    let (width, height) = jpeg_size(pixels.width, pixels.height);
    let scaled;
    let source = if (width, height) == (pixels.width, pixels.height) {
        pixels
    } else {
        scaled = scale_to(pixels, width, height);
        &scaled
    };
    let image = super::cgimage::from_pixels(source)
        .ok_or_else(|| "can't make a CGImage of the frame".to_string())?;
    // SAFETY: the type is the JPEG UTI ImageIO documents.
    let kind = CFString::from_static_str("public.jpeg");
    let data = CFMutableData::new(None, 0).ok_or_else(|| "can't make a data buffer".to_string())?;
    // SAFETY: one image goes into the destination, and the data outlives it.
    let destination = unsafe { CGImageDestination::with_data(&data, &kind, 1, None) }
        .ok_or_else(|| "ImageIO has no JPEG encoder".to_string())?;
    // SAFETY: the key is the quality property and the number is a float it accepts.
    let properties: CFRetained<CFDictionary<CFString, CFType>> = CFDictionary::from_slices(
        &[unsafe { kCGImageDestinationLossyCompressionQuality }],
        &[as_cf_type(&*CFNumber::new_f32(JPEG_QUALITY))],
    );
    // SAFETY: the destination takes the image with its properties.
    unsafe { destination.add_image(&image, Some(opaque(&properties))) };
    // SAFETY: finalizing writes the JPEG into the data buffer.
    if !unsafe { destination.finalize() } {
        return Err("ImageIO couldn't finish the JPEG".into());
    }
    let bytes = data.to_vec();
    if bytes.len() < 2 || bytes[..2] != [0xFF, 0xD8] {
        return Err("ImageIO wrote no JPEG".into());
    }
    Ok((bytes, width, height))
}

/// An image file's `CGImage`, as a crash left it.
pub fn decode_image(bytes: &[u8]) -> Result<CFRetained<CGImage>, String> {
    let data = objc2_core_foundation::CFData::from_bytes(bytes);
    // SAFETY: the source reads the data, which it retains.
    let source = unsafe { CGImageSource::with_data(&data, None) }
        .ok_or_else(|| "the image doesn't open".to_string())?;
    // SAFETY: the file has at least one image; the source retains the data.
    unsafe { source.image_at_index(0, None) }
        .ok_or_else(|| "the image holds no picture".to_string())
}

/// The pixel size of an image file.
pub fn decode_size(bytes: &[u8]) -> Result<(u32, u32), String> {
    let image = decode_image(bytes)?;
    Ok((
        CGImage::width(Some(&image)) as u32,
        CGImage::height(Some(&image)) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_longest_side_at_most_2560() {
        assert_eq!(jpeg_size(1920, 1080), (1920, 1080));
        assert_eq!(jpeg_size(1440, 2560), (1440, 2560));
        assert_eq!(jpeg_size(3840, 2160), (2560, 1440));
        assert_eq!(jpeg_size(5120, 1440), (2560, 720));
    }

    #[test]
    fn encodes_and_decodes_a_jpeg() {
        let (width, height) = (320u32, 200u32);
        let mut bgra = vec![255u8; (width * height * 4) as usize];
        for px in bgra.chunks_exact_mut(4).take(5000) {
            px[..3].copy_from_slice(&[40, 80, 160]);
        }
        let pixels = Pixels {
            bgra,
            width,
            height,
        };
        let (bytes, w, h) = encode(&pixels).unwrap();
        assert_eq!((w, h), (width, height));
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a JPEG starts with SOI");
        assert_eq!(decode_size(&bytes).unwrap(), (width, height));

        let big = Pixels {
            bgra: vec![200u8; 3000 * 100 * 4],
            width: 3000,
            height: 100,
        };
        let (bytes, w, h) = encode(&big).unwrap();
        assert_eq!((w, h), (2560, 85));
        assert_eq!(decode_size(&bytes).unwrap(), (2560, 85));
    }
}
