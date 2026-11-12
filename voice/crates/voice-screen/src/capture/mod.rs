//! Where frames come from: Windows.Graphics.Capture on one monitor on Windows, or the synthetic
//! screen of the tests and live runs. Both copy each frame into a picture the process owns and
//! hand it to the main thread.
//!
//! On macOS the picture is a `CVPixelBuffer` (`mac`). The real screen is ScreenCaptureKit (`sck`)
//! and the synthetic one is `synthetic_mac`.

#[cfg(windows)]
pub mod d3d;
#[cfg(windows)]
pub mod monitors_win;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
pub mod synthetic;
#[cfg(windows)]
pub mod wgc;

#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_os = "macos")]
pub mod monitors_mac;
#[cfg(target_os = "macos")]
pub mod sck;
#[cfg(target_os = "macos")]
pub mod session_mac;
#[cfg(target_os = "macos")]
pub mod synthetic_mac;

#[cfg(windows)]
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

/// The picture of a frame on Windows: a texture of the process's own, copied out of the capture.
/// It travels from the capture's thread to the main thread and on to the encoder's, so it is
/// declared sendable: Direct3D textures are free-threaded and are only read after the copy.
#[cfg(windows)]
#[derive(Clone)]
pub struct Texture(pub ID3D11Texture2D);

#[cfg(windows)]
unsafe impl Send for Texture {}

/// A captured frame on Windows.
#[cfg(windows)]
pub type CapturedFrame = crate::platform::Frame<Texture>;

/// The picture of a frame, as the types above name it.
#[cfg(windows)]
pub type Picture = Texture;
