//! Frames: a small luma thumbnail of each captured frame decides with
//! `voice_screen_files::frames::Saver` which ones to keep; a kept frame becomes a JPEG and the
//! text the platform reads on it. The queue and its two threads (`worker`) are platform-neutral;
//! the thumbnail, JPEG and OCR implementations are behind `crate::platform`.

pub mod upscale;
pub mod worker;

#[cfg(windows)]
pub mod jpeg;
#[cfg(windows)]
pub mod ocr;
#[cfg(windows)]
pub mod thumb;

#[cfg(target_os = "macos")]
pub mod cgimage;
#[cfg(target_os = "macos")]
pub mod jpeg_mac;
#[cfg(target_os = "macos")]
pub mod ocr_vision;
#[cfg(target_os = "macos")]
pub mod thumb_mac;

/// The macOS JPEG code under the name the Windows module has, so callers (and the crash tests)
/// name one module on both platforms.
#[cfg(target_os = "macos")]
pub use self::jpeg_mac as jpeg;

/// Every language this computer's text reader reads, as BCP-47 tags.
#[cfg(windows)]
pub fn languages() -> Vec<String> {
    ocr::languages()
}

/// Every language this computer's text reader reads, as BCP-47 tags.
#[cfg(target_os = "macos")]
pub fn languages() -> Vec<String> {
    ocr_vision::languages()
}
