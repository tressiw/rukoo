//! The video files: a queue and one encoder thread per file, with the platform's writer behind
//! `platform::VideoWriter` (Media Foundation's sink writer on Windows).

pub mod size;
pub mod thread;

#[cfg(windows)]
pub mod encoders;
#[cfg(windows)]
pub mod fault;
#[cfg(windows)]
pub mod mf;
#[cfg(target_os = "macos")]
pub mod vt;
#[cfg(windows)]
pub mod writer;
#[cfg(target_os = "macos")]
pub mod writer_mac;

/// The nominal frame rate written into the media types; the real rate is variable, at most this.
pub const FPS: u32 = 15;
