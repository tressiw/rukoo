//! The text on a frame, read with Vision's `VNRecognizeTextRequest`.
//!
//! The language follows the Windows rule: the user's preferred languages first, taking the first
//! one Vision reads, and Vision's first supported language when none of them is. Small text reads
//! better twice as large, so frames up to 1080 px high are upscaled first, as on Windows.
//!
//! The request is created on the OCR thread, in `crate::platform::FrameIo`'s reader factory, so
//! Vision's per-thread state never leaves that thread.

use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary, NSLocale, NSString};
use objc2_vision::{
    VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
    VNRequestTextRecognitionLevel,
};

use super::cgimage::from_pixels;
use super::jpeg_mac::decode_image;
use super::upscale::{upscale_2x, UPSCALE_MAX_HEIGHT};
use crate::platform::{ImageText, Pixels, Reader};

/// Frames whose longest side is over this are not read: Vision refuses them, and an upscale of
/// them is pointless.
const MAX_SIDE: u32 = 8192;

// SAFETY: the request is created on the OCR thread (`FrameIo`'s reader factory runs there) and
// used by that thread only; Vision requests may be used from a background thread, and nothing
// touches one from two threads at once.
unsafe impl Send for VisionReader {}

pub struct VisionReader {
    request: objc2::rc::Retained<VNRecognizeTextRequest>,
    language: Option<String>,
}

impl VisionReader {
    /// A reader for the user's language, when Vision reads it.
    pub fn new() -> VisionReader {
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let supported = supported_languages(&request);
        let language = choose_language(&supported);
        if let Some(language) = &language {
            let tag = NSString::from_str(language);
            let languages = NSArray::from_slice(&[&*tag]);
            request.setRecognitionLanguages(&languages);
        }
        VisionReader { request, language }
    }

    pub fn language(&self) -> Option<String> {
        self.language.clone()
    }

    /// The text lines on a Core Graphics image.
    fn read_cgimage(&self, image: &CGImage) -> Result<Vec<String>, String> {
        let options = NSDictionary::<VNImageOption, AnyObject>::dictionary();
        // SAFETY: the handler reads the image for the duration of the call.
        let handler = unsafe {
            VNImageRequestHandler::initWithCGImage_options(
                VNImageRequestHandler::alloc(),
                image,
                &options,
            )
        };
        let request: &VNRequest = &self.request;
        let requests = NSArray::from_slice(&[request]);
        handler
            .performRequests_error(&requests)
            .map_err(|e| e.localizedDescription().to_string())?;
        let mut lines = Vec::new();
        for observation in self.request.results().iter().flat_map(|r| r.iter()) {
            if let Some(candidate) = observation.topCandidates(1).firstObject() {
                lines.push(candidate.string().to_string());
            }
        }
        Ok(lines)
    }
}

impl Default for VisionReader {
    fn default() -> VisionReader {
        VisionReader::new()
    }
}

impl Reader for VisionReader {
    fn available(&self) -> bool {
        !supported_languages(&self.request).is_empty()
    }

    fn language(&self) -> Option<String> {
        VisionReader::language(self)
    }

    fn read_pixels(&self, pixels: &Pixels) -> Result<Vec<String>, String> {
        let (width, height) = (pixels.width, pixels.height);
        if width > MAX_SIDE || height > MAX_SIDE {
            return Ok(Vec::new());
        }
        if height <= UPSCALE_MAX_HEIGHT {
            let big = upscale_2x(pixels);
            return match from_pixels(&big) {
                Some(image) => self.read_cgimage(&image),
                None => Err("can't make a CGImage of the frame".into()),
            };
        }
        match from_pixels(pixels) {
            Some(image) => self.read_cgimage(&image),
            None => Err("can't make a CGImage of the frame".into()),
        }
    }

    fn read_image(&self, file: &[u8]) -> Result<(ImageText, (u32, u32)), String> {
        let image = decode_image(file)?;
        let size = (
            objc2_core_graphics::CGImage::width(Some(&image)) as u32,
            objc2_core_graphics::CGImage::height(Some(&image)) as u32,
        );
        if !self.available() {
            return Ok((ImageText::None, size));
        }
        match self.read_cgimage(&image) {
            Ok(lines) => Ok((ImageText::Text(lines), size)),
            Err(_) => Ok((ImageText::Failed, size)),
        }
    }
}

/// Every language this Mac's Vision reads, as BCP-47 tags.
pub fn languages() -> Vec<String> {
    supported_languages(&VNRecognizeTextRequest::new())
}

fn supported_languages(request: &VNRecognizeTextRequest) -> Vec<String> {
    // SAFETY: the method only reads the request's configuration and returns a new array.
    match unsafe { request.supportedRecognitionLanguagesAndReturnError() } {
        Ok(list) => list.iter().map(|l| l.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

/// The user's first preferred language Vision reads, else Vision's first supported one. A bare
/// language matches a regional tag of the same language (`nl` matches `nl-NL`).
fn choose_language(supported: &[String]) -> Option<String> {
    let preferred = NSLocale::preferredLanguages();
    for language in preferred.iter() {
        let language = language.to_string();
        if let Some(found) = supported
            .iter()
            .find(|supported| **supported == language)
            .or_else(|| {
                let tag = language.split(['-', '_']).next().unwrap_or(&language);
                supported.iter().find(|supported| {
                    supported
                        .split(['-', '_'])
                        .next()
                        .is_some_and(|s| s.eq_ignore_ascii_case(tag))
                })
            })
        {
            return Some(found.clone());
        }
    }
    supported.first().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_synthetic_slide() {
        let reader = VisionReader::new();
        assert!(reader.available(), "this Mac has no Vision text reader");
        assert!(
            reader.language().is_some(),
            "a reader without a language: {supported:?}",
            supported = languages()
        );
        // Slide 3 says "Slide 3", drawn by the synthetic screen at 1280 x 720.
        let slide = crate::capture::synthetic_mac::slide("Slide 3", 3, 1280, 720).unwrap();
        let lines = reader.read_pixels(&slide).unwrap();
        let text = lines.join("\n");
        assert!(text.contains("Slide 3"), "read {text:?}");
    }

    /// The user's language wins when Vision reads it: a Dutch Mac reads Dutch first.
    #[test]
    fn the_preferred_language_comes_first() {
        let supported = vec![
            "de-DE".to_string(),
            "nl-NL".to_string(),
            "en-US".to_string(),
        ];
        let chosen = choose_language(&supported).unwrap();
        assert!(supported.contains(&chosen), "{chosen}");
        assert_eq!(choose_language(&[]), None);
        // A bare language matches its regional tag.
        assert_eq!(
            choose_language(&["en-US".to_string()]).as_deref(),
            Some("en-US")
        );
    }

    /// A blank frame has no text, and a frame that isn't an image fails as an image.
    #[test]
    fn a_blank_frame_reads_nothing() {
        let reader = VisionReader::new();
        let blank = Pixels {
            bgra: vec![255u8; 200 * 100 * 4],
            width: 200,
            height: 100,
        };
        let lines = reader.read_pixels(&blank).unwrap();
        assert!(lines.iter().all(|l| l.trim().is_empty()), "{lines:?}");
        assert!(reader.read_image(b"not an image").is_err());
    }

    /// A saved slide reads back through the image path a crash leaves frames on.
    #[test]
    fn reads_a_saved_slide() {
        let slide = crate::capture::synthetic_mac::slide("Slide 7", 7, 1280, 720).unwrap();
        let (jpeg, _, _) = super::super::jpeg_mac::encode(&slide).unwrap();
        let (text, (width, height)) = VisionReader::new().read_image(&jpeg).unwrap();
        assert_eq!((width, height), (1280, 720));
        let ImageText::Text(lines) = text else {
            panic!("the saved slide has no text: {text:?}");
        };
        let text = lines.join("\n");
        assert!(text.contains("Slide 7"), "read {text:?}");
    }

    /// An image with no text is legal, and leaves the size alone.
    #[test]
    fn a_blank_image_reads_no_lines() {
        let blank = Pixels {
            bgra: vec![255u8; 320 * 180 * 4],
            width: 320,
            height: 180,
        };
        let (jpeg, _, _) = super::super::jpeg_mac::encode(&blank).unwrap();
        let (text, size) = VisionReader::new().read_image(&jpeg).unwrap();
        assert_eq!(size, (320, 180));
        match text {
            ImageText::Text(lines) => assert!(lines.iter().all(|l| l.trim().is_empty())),
            other => panic!("{other:?}"),
        }
    }
}
