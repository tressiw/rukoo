//! The text on a frame, read on this computer by `Windows.Media.Ocr`: the user profile's
//! language when Windows has an OCR pack for it, else the first pack installed, else no text.
//! Small text reads better twice as large, so frames up to 1080 px high are upscaled first.

use windows::Globalization::Language;
use windows::Graphics::Imaging::SoftwareBitmap;
use windows::Media::Ocr::OcrEngine;

use super::jpeg;
use super::upscale::{upscale_2x, UPSCALE_MAX_HEIGHT};
use crate::platform::Pixels;

pub struct Ocr {
    engine: Option<OcrEngine>,
    language: Option<String>,
    max_side: u32,
}

impl Ocr {
    pub fn new() -> Ocr {
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .ok()
            .or_else(|| {
                let languages = OcrEngine::AvailableRecognizerLanguages().ok()?;
                let first: Language = languages.GetAt(0).ok()?;
                OcrEngine::TryCreateFromLanguage(&first).ok()
            });
        let language = engine.as_ref().and_then(|e| {
            e.RecognizerLanguage()
                .and_then(|l| l.LanguageTag())
                .ok()
                .map(|tag| tag.to_string())
        });
        Ocr {
            engine,
            language,
            max_side: OcrEngine::MaxImageDimension().unwrap_or(4096),
        }
    }

    /// The BCP-47 tag of the language it reads, or `None` without an OCR pack.
    pub fn language(&self) -> Option<String> {
        self.language.clone()
    }

    pub fn available(&self) -> bool {
        self.engine.is_some()
    }

    /// The text lines on a bitmap.
    pub fn read(&self, bitmap: &SoftwareBitmap) -> windows::core::Result<Vec<String>> {
        let Some(engine) = &self.engine else {
            return Ok(Vec::new());
        };
        let result = engine.RecognizeAsync(bitmap)?.join()?;
        let mut lines = Vec::new();
        for line in &result.Lines()? {
            lines.push(line.Text()?.to_string());
        }
        Ok(lines)
    }

    /// Reads a frame's pixels, upscaled when it is small enough to gain from it.
    pub fn read_pixels(&self, pixels: &Pixels) -> windows::core::Result<Vec<String>> {
        let upscale = pixels.height <= UPSCALE_MAX_HEIGHT
            && pixels.width * 2 <= self.max_side
            && pixels.height * 2 <= self.max_side;
        let bitmap = if upscale {
            jpeg::bitmap(&upscale_2x(pixels))?
        } else if pixels.width > self.max_side || pixels.height > self.max_side {
            return Ok(Vec::new());
        } else {
            jpeg::bitmap(pixels)?
        };
        self.read(&bitmap)
    }
}

impl Default for Ocr {
    fn default() -> Ocr {
        Ocr::new()
    }
}

/// Every OCR language Windows has a pack for, as BCP-47 tags.
pub fn languages() -> Vec<String> {
    let Ok(list) = OcrEngine::AvailableRecognizerLanguages() else {
        return Vec::new();
    };
    (&list)
        .into_iter()
        .filter_map(|l| l.LanguageTag().ok().map(|t| t.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_languages_and_reads_nothing_on_a_blank_frame() {
        crate::com::init_mta();
        let ocr = Ocr::new();
        assert_eq!(ocr.available(), !languages().is_empty());
        let blank = Pixels {
            bgra: vec![255u8; 200 * 100 * 4],
            width: 200,
            height: 100,
        };
        let lines = ocr.read_pixels(&blank).unwrap();
        assert!(lines.iter().all(|l| l.trim().is_empty()), "{lines:?}");
    }
}
