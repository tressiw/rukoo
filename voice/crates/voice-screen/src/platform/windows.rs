//! The Windows backend: Direct3D 11 for capture, thumbnails and pixels, Media Foundation for
//! video, and WinRT for JPEG and OCR. It is the only place the orchestration's traits meet an
//! operating system; the code behind them is the screen process's original Windows code.

use std::path::Path;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use voice_screen_files::clock::Now;
use voice_screen_files::frames::Thumb;
use voice_screen_files::monitors::{MonitorInfo, WindowInfo};

use crate::args::TestSpec;
use crate::capture::d3d::{self, is_device_lost, D3D};
use crate::capture::{monitors_win, session, synthetic, wgc, Texture};
use crate::clockwin;
use crate::frames::thumb::Thumbnailer;
use crate::frames::{jpeg, ocr};
use crate::record::Ev;
use crate::video::writer::WinVideoMaker;

use super::{
    Clocks, DeviceInfo, Disk, Displays, FrameIo, ImageText, Jpeg, Pixels, Platform, PlatformError,
    Reader, SourceOptions, ThreadInit, Thumbs, VideoMaker,
};

impl From<windows::core::Error> for PlatformError {
    fn from(e: windows::core::Error) -> PlatformError {
        PlatformError {
            device_lost: is_device_lost(&e),
            code: format!("{:#010x}", e.code().0),
            message: e.message(),
        }
    }
}

/// The screen process's backend on Windows.
#[derive(Clone)]
pub struct WindowsPlatform {
    disk: Arc<WinDisk>,
    jpeg: Arc<WinJpeg>,
    setup: Arc<WinThreadInit>,
}

impl WindowsPlatform {
    pub fn new() -> WindowsPlatform {
        WindowsPlatform::default()
    }
}

impl Default for WindowsPlatform {
    fn default() -> WindowsPlatform {
        WindowsPlatform {
            disk: Arc::new(WinDisk),
            jpeg: Arc::new(WinJpeg),
            setup: Arc::new(WinThreadInit),
        }
    }
}

impl Platform for WindowsPlatform {
    type Picture = Texture;
    type Device = D3D;
    type Thumbs = Thumbnailer;
    type Video = crate::video::writer::VideoWriter;

    fn init(&self) {
        crate::com::init_mta();
        crate::com::dpi_aware();
    }

    fn thread_init(&self) -> Arc<dyn ThreadInit> {
        self.setup.clone()
    }

    fn clocks(&self) -> &dyn Clocks {
        &WIN_CLOCKS
    }

    fn displays(&self) -> &dyn Displays {
        &WIN_DISPLAYS
    }

    fn disk(&self) -> Arc<dyn Disk> {
        self.disk.clone()
    }

    fn jpeg(&self) -> Arc<dyn Jpeg> {
        self.jpeg.clone()
    }

    fn reader(&self) -> Box<dyn Reader> {
        crate::com::init_mta();
        Box::new(WinReader(ocr::Ocr::new()))
    }

    fn locked(&self) -> bool {
        session::locked()
    }

    fn frame_io(&self) -> FrameIo {
        let platform = self.clone();
        FrameIo {
            setup: self.thread_init(),
            jpeg: self.jpeg(),
            disk: self.disk(),
            reader: Box::new(move || platform.reader()),
        }
    }

    fn create_device(&self, want_video: bool, warp: bool) -> Result<D3D, PlatformError> {
        Ok(d3d::create(want_video, warp)?)
    }

    fn device_info(&self, device: &D3D) -> DeviceInfo {
        DeviceInfo {
            video: device.video,
            warp: device.warp,
        }
    }

    fn device_removed(&self, device: &D3D) -> bool {
        device.removed()
    }

    fn device_memory(&self, device: &D3D) -> Option<u64> {
        device.memory_in_use()
    }

    fn source(
        &self,
        device: D3D,
        options: &SourceOptions,
        to_main: SyncSender<Ev<Texture>>,
    ) -> Box<dyn super::Source> {
        // Windows leaves Rukoo out through Rukoo's own content protection, so the exclusions
        // are ignored here.
        match &options.test {
            Some(spec) => Box::new(synthetic::new(device, to_main, spec)),
            None => Box::new(wgc::new(device, to_main)),
        }
    }

    fn thumbs(&self, device: &D3D, width: u32, height: u32) -> Result<Thumbnailer, PlatformError> {
        Ok(Thumbnailer::new(device, width, height)?)
    }

    fn pixels(&self, device: &D3D, frame: &super::Frame<Texture>) -> Result<Pixels, PlatformError> {
        Ok(jpeg::pixels(device, frame)?)
    }

    fn video_maker(&self, device: &D3D) -> Option<Box<dyn VideoMaker<File = Self::Video>>> {
        let mf = crate::video::mf::shared()?;
        Some(Box::new(WinVideoMaker::new(mf, device.clone())))
    }
}

/// Windows' interrupt time and wall clock, as `clockwin` reads them.
struct WinClocks;

static WIN_CLOCKS: WinClocks = WinClocks;

impl Clocks for WinClocks {
    fn boot_100ns(&self) -> u64 {
        clockwin::boot_100ns()
    }

    fn awake_100ns(&self) -> u64 {
        clockwin::awake_100ns()
    }

    fn now(&self) -> Now {
        clockwin::now()
    }

    fn at_boot(&self, boot: u64) -> Now {
        clockwin::at_boot(boot)
    }
}

/// The monitors, the window Z-order and the cursor, as Windows reports them.
struct WinDisplays;

static WIN_DISPLAYS: WinDisplays = WinDisplays;

impl Displays for WinDisplays {
    fn list(&self, test: Option<&TestSpec>) -> Vec<MonitorInfo> {
        match test {
            Some(spec) => synthetic::monitors(spec),
            None => monitors_win::list(),
        }
    }

    fn z_order(&self, _exclude_pids: &[u32]) -> Vec<WindowInfo> {
        monitors_win::z_order()
    }

    fn cursor_monitor(&self) -> Option<String> {
        monitors_win::cursor_monitor()
    }

    fn can_capture(&self) -> bool {
        wgc::supported()
    }
}

/// Free space on a volume, through `GetDiskFreeSpaceExW`.
struct WinDisk;

impl Disk for WinDisk {
    fn free_bytes(&self, path: &Path, test_file: Option<&Path>) -> Option<u64> {
        crate::disk::free_bytes(path, test_file)
    }

    fn is_disk_full(&self, e: &std::io::Error) -> bool {
        crate::disk::is_disk_full(e)
    }
}

/// Frames as JPEG through WinRT's bitmap encoder.
struct WinJpeg;

impl Jpeg for WinJpeg {
    fn encode(&self, pixels: &Pixels) -> Result<(Vec<u8>, u32, u32), String> {
        jpeg::bitmap(pixels)
            .and_then(|bitmap| jpeg::encode(&bitmap))
            .map_err(|e| format!("JPEG encoding failed: {:#010x}", e.code().0))
    }
}

/// Windows' OCR, created on the thread that reads.
struct WinReader(ocr::Ocr);

impl Reader for WinReader {
    fn available(&self) -> bool {
        self.0.available()
    }

    fn language(&self) -> Option<String> {
        self.0.language()
    }

    fn read_pixels(&self, pixels: &Pixels) -> Result<Vec<String>, String> {
        self.0
            .read_pixels(pixels)
            .map_err(|e| format!("{:#010x}", e.code().0))
    }

    fn read_image(&self, file: &[u8]) -> Result<(ImageText, (u32, u32)), String> {
        let bitmap = jpeg::decode(file).map_err(|e| format!("{:#010x}", e.code().0))?;
        let size = (
            bitmap.PixelWidth().unwrap_or(0) as u32,
            bitmap.PixelHeight().unwrap_or(0) as u32,
        );
        if !self.0.available() {
            return Ok((ImageText::None, size));
        }
        match self.0.read(&bitmap) {
            Ok(lines) => Ok((ImageText::Text(lines), size)),
            Err(_) => Ok((ImageText::Failed, size)),
        }
    }
}

impl Thumbs for Thumbnailer {
    type Picture = Texture;

    fn size(&self) -> (u32, u32) {
        Thumbnailer::size(self)
    }

    fn thumb(&mut self, picture: &Texture) -> Result<Thumb, PlatformError> {
        Ok(Thumbnailer::thumb(self, &picture.0)?)
    }
}

/// COM's multithreaded apartment, which every thread that captures, encodes or reads joins.
struct WinThreadInit;

impl ThreadInit for WinThreadInit {
    fn init_thread(&self) {
        crate::com::init_mta();
    }
}
