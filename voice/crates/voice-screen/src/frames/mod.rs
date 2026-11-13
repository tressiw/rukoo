//! Frames: a small luma thumbnail of each captured frame decides with
//! `voice_screen_files::frames::Saver` which ones to keep; a kept frame becomes a JPEG and the
//! text the platform reads on it. The queue and its two threads (`worker`) are platform-neutral;
//! the thumbnail, JPEG and OCR implementations are behind `crate::platform`.

pub mod worker;

#[cfg(windows)]
pub mod jpeg;
#[cfg(windows)]
pub mod ocr;
#[cfg(windows)]
pub mod thumb;
