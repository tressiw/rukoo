//! `rukoo-voice-screen probe`: what this computer can record, for Voice's Settings and CI logs.

use voice_screen_files::caps::Capabilities;
use voice_screen_files::journal::FORMAT_VERSION;

#[cfg(windows)]
pub fn probe() -> Capabilities {
    use crate::capture::{monitors_win, wgc};
    use crate::frames::ocr;
    use crate::video::{encoders, mf};

    crate::com::init_mta();
    crate::com::dpi_aware();
    let encoders = mf::shared().map(encoders::all_h264).unwrap_or_default();
    let hardware_encoder = encoders.iter().find(|e| e.hardware).cloned();
    Capabilities {
        version: FORMAT_VERSION,
        capture: wgc::supported(),
        encoders,
        hardware_encoder,
        ocr: ocr::languages(),
        monitors: monitors_win::list(),
    }
}

/// The encoders VideoToolbox lists, with the hardware one from its own `ave` id and from a
/// session that requires hardware; `capture` stays false until ScreenCaptureKit lands.
#[cfg(target_os = "macos")]
pub fn probe() -> Capabilities {
    let encoders = crate::video::vt::h264_encoders();
    let hardware_encoder = encoders.iter().find(|e| e.hardware).cloned();
    Capabilities {
        version: FORMAT_VERSION,
        capture: false,
        encoders,
        hardware_encoder,
        ocr: crate::frames::languages(),
        monitors: crate::capture::monitors_mac::list(),
    }
}
