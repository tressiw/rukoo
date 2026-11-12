//! The macOS backend: Core Video pixel buffers, the synthetic screen for tests, VideoToolbox for
//! video, Vision for text, ImageIO for JPEG and Core Graphics for the displays. The real screen
//! capture, ScreenCaptureKit, comes with its own task: until then `Displays::can_capture` is
//! false, so `record` refuses a real screen and only the synthetic one has a source.

use std::path::Path;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use voice_screen_files::clock::Now;
use voice_screen_files::frames::Thumb;
use voice_screen_files::monitors::{MonitorInfo, WindowInfo};

use crate::args::TestSpec;
use crate::capture::mac::PixelBuffer;
use crate::capture::{monitors_mac, session_mac, synthetic_mac};
use crate::clock_mac;
use crate::frames::jpeg_mac;
use crate::frames::ocr_vision::VisionReader;
use crate::frames::thumb_mac::Thumbnailer;
use crate::record::Ev;
use crate::video::writer_mac::{MacVideoMaker, MacVideoWriter};

use super::{
    Clocks, DeviceInfo, Disk, Displays, Frame, FrameIo, Jpeg, Pixels, Platform, PlatformError,
    Reader, Source, ThreadInit, Thumbs, VideoMaker,
};

/// The screen process's backend on macOS.
#[derive(Clone)]
pub struct MacPlatform {
    disk: Arc<MacDisk>,
    jpeg: Arc<MacJpeg>,
    setup: Arc<MacThreadInit>,
}

impl MacPlatform {
    pub fn new() -> MacPlatform {
        MacPlatform::default()
    }
}

impl Default for MacPlatform {
    fn default() -> MacPlatform {
        MacPlatform {
            disk: Arc::new(MacDisk),
            jpeg: Arc::new(MacJpeg),
            setup: Arc::new(MacThreadInit),
        }
    }
}

/// What the platform calls "the graphics device": on macOS the encoder and the display are not
/// one object, so this only carries whether VideoToolbox has a hardware encoder, which is what
/// the video's recovery rules ask about. `VideoToolbox` has no device that can be removed.
#[derive(Clone, Debug)]
pub struct MacDevice {
    pub video: bool,
}

impl Platform for MacPlatform {
    type Picture = PixelBuffer;
    type Device = MacDevice;
    type Thumbs = Thumbnailer;
    type Video = MacVideoWriter;

    fn init(&self) {}

    fn thread_init(&self) -> Arc<dyn ThreadInit> {
        self.setup.clone()
    }

    fn clocks(&self) -> &dyn Clocks {
        &MAC_CLOCKS
    }

    fn displays(&self) -> &dyn Displays {
        &MAC_DISPLAYS
    }

    fn disk(&self) -> Arc<dyn Disk> {
        self.disk.clone()
    }

    fn jpeg(&self) -> Arc<dyn Jpeg> {
        self.jpeg.clone()
    }

    fn reader(&self) -> Box<dyn Reader> {
        Box::new(VisionReader::new())
    }

    fn locked(&self) -> bool {
        session_mac::locked()
    }

    fn frame_io(&self) -> FrameIo {
        FrameIo {
            setup: self.thread_init(),
            jpeg: self.jpeg(),
            disk: self.disk(),
            reader: Box::new(|| Box::new(VisionReader::new())),
        }
    }

    fn create_device(&self, want_video: bool, _warp: bool) -> Result<MacDevice, PlatformError> {
        // There is no graphics device to create: a capture hands over pixel buffers. Video
        // support means a hardware H.264 encoder, which is VideoToolbox's.
        Ok(MacDevice {
            video: want_video && crate::video::vt::hardware_available(),
        })
    }

    fn device_info(&self, device: &MacDevice) -> DeviceInfo {
        DeviceInfo {
            video: device.video,
            warp: false,
        }
    }

    fn device_removed(&self, _device: &MacDevice) -> bool {
        false
    }

    fn device_memory(&self, _device: &MacDevice) -> Option<u64> {
        None
    }

    fn source(
        &self,
        _device: MacDevice,
        test: Option<&TestSpec>,
        to_main: SyncSender<Ev<PixelBuffer>>,
    ) -> Box<dyn Source> {
        match test {
            Some(spec) => Box::new(synthetic_mac::new(to_main, spec)),
            None => Box::new(NoCapture),
        }
    }

    fn thumbs(
        &self,
        _device: &MacDevice,
        width: u32,
        height: u32,
    ) -> Result<Thumbnailer, PlatformError> {
        Ok(Thumbnailer::new(width, height))
    }

    fn pixels(
        &self,
        _device: &MacDevice,
        frame: &Frame<PixelBuffer>,
    ) -> Result<Pixels, PlatformError> {
        frame
            .picture
            .pixels()
            .ok_or_else(|| PlatformError::new("can't read the frame's pixels"))
    }

    fn video_maker(&self, _device: &MacDevice) -> Option<Box<dyn VideoMaker<File = Self::Video>>> {
        Some(Box::new(MacVideoMaker::new()))
    }
}

/// The real screen until ScreenCaptureKit lands. `record` refuses it because `can_capture` is
/// false, so this only answers if something asks for it anyway.
struct NoCapture;

impl Source for NoCapture {
    fn start(&mut self, _monitor: &MonitorInfo, _generation: u64) -> Result<(), PlatformError> {
        Err(PlatformError::new(
            "capturing the real screen isn't available on macOS yet",
        ))
    }

    fn stop(&mut self) {}
}

/// macOS' `mach_continuous_time` and wall clock, as `clock_mac` reads them.
struct MacClocks;

static MAC_CLOCKS: MacClocks = MacClocks;

impl Clocks for MacClocks {
    fn boot_100ns(&self) -> u64 {
        clock_mac::boot_100ns()
    }

    fn awake_100ns(&self) -> u64 {
        clock_mac::awake_100ns()
    }

    fn now(&self) -> Now {
        clock_mac::now()
    }

    fn at_boot(&self, boot: u64) -> Now {
        clock_mac::at_boot(boot)
    }
}

/// The monitors Core Graphics lists, and the synthetic ones a test spec asks for. The window
/// Z-order and the cursor's monitor come with ScreenCaptureKit: nothing records the real screen
/// before it.
struct MacDisplays;

static MAC_DISPLAYS: MacDisplays = MacDisplays;

impl Displays for MacDisplays {
    fn list(&self, test: Option<&TestSpec>) -> Vec<MonitorInfo> {
        match test {
            Some(spec) => synthetic_mac::monitors(spec),
            None => monitors_mac::list(),
        }
    }

    fn z_order(&self) -> Vec<WindowInfo> {
        Vec::new()
    }

    fn cursor_monitor(&self) -> Option<String> {
        None
    }

    fn can_capture(&self) -> bool {
        false
    }
}

/// Free space on a volume, through `statvfs`.
struct MacDisk;

impl Disk for MacDisk {
    fn free_bytes(&self, path: &Path, test_file: Option<&Path>) -> Option<u64> {
        crate::disk::free_bytes(path, test_file)
    }

    fn is_disk_full(&self, e: &std::io::Error) -> bool {
        crate::disk::is_disk_full(e)
    }
}

/// Frames as JPEG through ImageIO.
struct MacJpeg;

impl Jpeg for MacJpeg {
    fn encode(&self, pixels: &Pixels) -> Result<(Vec<u8>, u32, u32), String> {
        jpeg_mac::encode(pixels).map_err(|e| format!("JPEG encoding failed: {e}"))
    }
}

impl Thumbs for Thumbnailer {
    type Picture = PixelBuffer;

    fn size(&self) -> (u32, u32) {
        Thumbnailer::size(self)
    }

    fn thumb(&mut self, picture: &PixelBuffer) -> Result<Thumb, PlatformError> {
        Thumbnailer::thumb(self, picture)
    }
}

/// Nothing to set up per thread: Vision and VideoToolbox live on the threads that use them.
struct MacThreadInit;

impl ThreadInit for MacThreadInit {
    fn init_thread(&self) {}
}
