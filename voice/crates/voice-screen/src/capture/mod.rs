//! Where frames come from: Windows.Graphics.Capture on one monitor, or the synthetic screen of
//! the tests and live runs. Both copy each frame into a texture the process owns and hand it to
//! the main thread.

pub mod d3d;
pub mod monitors_win;
pub mod session;
pub mod synthetic;
pub mod wgc;

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

/// The picture of a frame on Windows: a texture of the process's own, copied out of the capture.
/// It travels from the capture's thread to the main thread and on to the encoder's, so it is
/// declared sendable: Direct3D textures are free-threaded and are only read after the copy.
#[derive(Clone)]
pub struct Texture(pub ID3D11Texture2D);

unsafe impl Send for Texture {}

/// A captured frame on Windows.
pub type CapturedFrame = crate::platform::Frame<Texture>;

/// The picture of a frame, as the types above name it.
pub type Picture = Texture;
