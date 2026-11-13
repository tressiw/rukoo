//! What the screen process needs from the computer it records, as traits with plain Rust types.
//!
//! The orchestration in `record` never names a graphics API, an encoder or an operating system:
//! it captures frames, encodes video, reads text and asks for free space through these traits.
//! `platform/windows.rs` implements them with Direct3D, Media Foundation and WinRT; a macOS
//! backend implements the same ones with ScreenCaptureKit, VideoToolbox and Vision.

use std::fmt;
use std::path::Path;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use voice_screen_files::clock::Now;
use voice_screen_files::frames::Thumb;
use voice_screen_files::monitors::{MonitorInfo, WindowInfo};

use crate::args::TestSpec;
use crate::record::Ev;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

/// A captured frame: the platform's own picture, and what it says about it. `width` and `height`
/// are even (an odd last column or row is cropped). `boot` is when it was composed, in interrupt
/// time. `dirty` is the fraction of the screen its dirty regions cover, where the platform
/// reports them. `generation` names the capture session, so frames of a stopped one are dropped.
#[derive(Clone)]
pub struct Frame<F> {
    pub picture: F,
    pub boot: u64,
    pub width: u32,
    pub height: u32,
    pub dirty: Option<f32>,
    pub generation: u64,
}

/// Why a platform call failed, in the terms the orchestration decides by: whether the graphics
/// device went, and what to write in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformError {
    /// The graphics device was removed, reset or hung: capture and video start over on a new one.
    pub device_lost: bool,
    /// The platform's own code, ready for the log (`0x887a0005`).
    pub code: String,
    pub message: String,
}

impl PlatformError {
    pub fn new(message: impl Into<String>) -> PlatformError {
        PlatformError {
            device_lost: false,
            code: String::new(),
            message: message.into(),
        }
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.code.is_empty() {
            return f.write_str(&self.message);
        }
        write!(f, "{} {}", self.code, self.message)
    }
}

/// A capture of one monitor at a time.
pub trait Source: Send {
    /// Starts capturing `monitor`; frames carry `generation`.
    fn start(&mut self, monitor: &MonitorInfo, generation: u64) -> Result<(), PlatformError>;
    /// Stops capturing. Frames already sent may still arrive; their generation is old.
    fn stop(&mut self);
}

/// What a source needs besides the device it captures on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceOptions {
    /// The test's synthetic screen instead of the real one.
    pub test: Option<TestSpec>,
    /// The processes whose windows stay out of the recording; a platform that can't filter by
    /// process ignores it.
    pub exclude_pids: Vec<u32>,
}

/// What ended a video file early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    /// The disk is full (`0x80070070`, `STG_E_MEDIUMFULL`).
    Disk(String),
    /// The encoder failed to start or to encode.
    Encoder(String),
    /// Only a software encoder would run, and the test switch doesn't allow one.
    Software,
    /// The graphics device was removed or reset.
    DeviceLost,
}

impl fmt::Display for VideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VideoError::Disk(e) => write!(f, "the disk is full ({e})"),
            VideoError::Encoder(e) => write!(f, "the encoder failed ({e})"),
            VideoError::Software => f.write_str("no hardware H.264 encoder runs on this computer"),
            VideoError::DeviceLost => f.write_str("the graphics device was lost"),
        }
    }
}

/// How a file is made, including the test faults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriterOptions {
    /// The `software-encoder` test switch: the system-memory path into the software encoder.
    pub allow_software: bool,
    /// The `no-hardware` test switch: as if no hardware encoder were installed.
    pub no_hardware: bool,
    /// Meeting time of media time 0, from the file's `video` line.
    pub start_t: u64,
    /// `encoder-fail-at`: from this meeting time every write fails as an encoder error.
    pub encoder_fail_at: Option<u64>,
    /// `write-fail=video@`: from this meeting time the MP4 byte stream's writes fail as on a
    /// full disk (`video::fault`).
    pub disk_fail_at: Option<u64>,
}

/// One video file, as the encoder thread drives it: created on that thread, fed the frames the
/// orchestration decided on, and finished or abandoned when the file ends.
///
/// A frame is written when the next one arrives, so a sample's duration is the real gap to the
/// next: the capture rate varies. Frames held back because the screen is still aren't samples;
/// the newest of them waits for the next re-submit.
pub trait VideoWriter: 'static {
    /// The captured picture this writer takes, the platform's own frame type. It travels to the
    /// encoder's thread, so it must be sendable.
    type Picture: Send;

    /// Writes the frame captured at `boot` (interrupt time). It goes into the file when the next
    /// frame, a re-submit or the end gives its duration.
    fn write(&mut self, picture: &Self::Picture, boot: u64) -> Result<(), VideoError>;
    /// Keeps the frame captured at `boot` for the next re-submit, unless the last frame written is
    /// newer. A newer held frame replaces it; a written one drops it.
    fn hold(&mut self, picture: &Self::Picture, boot: u64);
    /// After a quarter second without a new sample, the held frame goes in at its capture time,
    /// or the last one again at `boot_now`, so a still screen still gets a keyframe and a fragment
    /// boundary every 1 to 2 s.
    fn tick(&mut self, boot_now: u64) -> Result<(), VideoError>;
    /// Writes the held frame, as at the end of the file, so the file ends on the newest picture.
    fn flush(&mut self) -> Result<(), VideoError>;
    /// Writes the last frame, finalizes the file and returns its size. A writer that failed
    /// before is abandoned instead: the file stays readable up to its last fragment.
    fn finish(self) -> Result<u64, VideoError>
    where
        Self: Sized;
    /// Drops the writer without finalizing, as after an error; returns the bytes on disk.
    fn abandon(&mut self) -> u64;
    /// The encoder's name and whether it is hardware.
    fn encoder(&self) -> (&str, bool);
    /// The video's own size, which may differ from the captured frame's.
    fn video_size(&self) -> (u32, u32);
    /// The meeting time where the file's last sample ends.
    fn end_t(&self) -> u64;
}

/// Makes a video file: the platform's encoders and container, for one file at a time.
pub trait VideoMaker: Send + 'static {
    type File: VideoWriter;

    /// Creates `path` (it must not exist) for frames of `width` x `height`, as `options` say. On
    /// failure nothing stays at `path`.
    fn create(
        &mut self,
        path: &Path,
        width: u32,
        height: u32,
        options: WriterOptions,
    ) -> Result<Self::File, VideoError>;
}

/// A frame's pixels, BGRA, rows top-down without padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// What the platform says about a graphics device: whether Media Foundation's GPU path can use it
/// (`video`), and whether it is a software rasterizer (`warp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub video: bool,
    pub warp: bool,
}

/// The text on an image file a crash left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageText {
    /// The lines read on it.
    Text(Vec<String>),
    /// This computer reads no text at all.
    None,
    /// The reader failed.
    Failed,
}

/// The clocks the screen process reads: interrupt time in 100 ns units, which counts sleep and
/// every process shares, the awake-only counter of the orphan rule, and the wall clock.
pub trait Clocks: Send + Sync {
    /// Interrupt time in 100 ns units: counts sleep.
    fn boot_100ns(&self) -> u64;
    /// Unbiased interrupt time in 100 ns units: awake time only, so a computer that slept doesn't
    /// count as a dead recorder.
    fn awake_100ns(&self) -> u64;
    /// Both clocks now, for the meeting clock.
    fn now(&self) -> Now;
    /// The clocks at an earlier boot time.
    fn at_boot(&self, boot: u64) -> Now;
}

/// The monitors to record and the windows that say which one shows the call.
pub trait Displays: Send + Sync {
    /// Every monitor, synthetic ones when a test spec asks for them.
    fn list(&self, test: Option<&TestSpec>) -> Vec<MonitorInfo>;
    /// Top-level windows from the foreground window down the Z-order. A window of a process in
    /// `exclude_pids` counts as excluded from the recording, like one that hides itself from
    /// capture, so a meeting started from that app picks the display behind it.
    fn z_order(&self, exclude_pids: &[u32]) -> Vec<WindowInfo>;
    /// The cursor's monitor.
    fn cursor_monitor(&self) -> Option<String>;
    /// Whether this computer can capture the screen at all: the platform has a capture.
    fn can_capture(&self) -> bool;
    /// Why this process may not capture the screen, when the computer asks the user for a grant
    /// that isn't there: macOS' Screen Recording. The words name the setting and where to switch it
    /// on. `None` where there is no such grant, and when it is given.
    fn missing_permission(&self) -> Option<String> {
        None
    }
}

/// Free space on the meeting folder's volume, for the drop floors.
pub trait Disk: Send + Sync {
    /// Bytes free for this user on the volume of `path`. With `test_file` (the `free-bytes-file`
    /// test switch) the number written in that file instead, so tests can step through the drop
    /// thresholds. `None` when neither can be read.
    fn free_bytes(&self, path: &Path, test_file: Option<&Path>) -> Option<u64>;
    /// Whether a write failed because the disk is full.
    fn is_disk_full(&self, e: &std::io::Error) -> bool;
}

/// The JPEG of a saved frame.
pub trait Jpeg: Send + Sync {
    /// The JPEG at `voice_screen_files::frames`'s quality, scaled so its longest side is at most
    /// its maximum, and the size it got.
    fn encode(&self, pixels: &Pixels) -> Result<(Vec<u8>, u32, u32), String>;
}

/// The text on a frame, read on this computer.
pub trait Reader: Send {
    /// Whether this computer has a text recognizer.
    fn available(&self) -> bool;
    /// The BCP-47 tag of the language it reads.
    fn language(&self) -> Option<String>;
    /// The text lines on a saved frame, read the way the frame was saved: upscaled when small
    /// text reads better that way.
    fn read_pixels(&self, pixels: &Pixels) -> Result<Vec<String>, String>;
    /// An image file a crash left without a `frame` line: its pixel size and its text.
    fn read_image(&self, file: &[u8]) -> Result<(ImageText, (u32, u32)), String>;
}

/// The 1/8-scale luma thumbnail that decides which frames are saved and which go to the encoder.
pub trait Thumbs {
    /// The captured picture this thumbnailer takes.
    type Picture;

    /// The frame size this thumbnailer takes.
    fn size(&self) -> (u32, u32);
    /// The thumbnail of a frame of this thumbnailer's size.
    fn thumb(&mut self, picture: &Self::Picture) -> Result<Thumb, PlatformError>;
}

/// Per-thread setup the platform needs, such as COM's multithreaded apartment. Called on every
/// thread that captures, encodes or reads before it uses the platform.
pub trait ThreadInit: Send + Sync + 'static {
    fn init_thread(&self);
}

/// What the frame worker's threads need from the platform.
pub struct FrameIo {
    pub setup: Arc<dyn ThreadInit>,
    pub jpeg: Arc<dyn Jpeg>,
    pub disk: Arc<dyn Disk>,
    /// Makes the text reader; the OCR thread calls it, so the engine is created on that thread.
    pub reader: Box<dyn FnOnce() -> Box<dyn Reader> + Send>,
}

/// The platform behind the screen process: the graphics device, the capture, thumbnails, pixels
/// and the video writer, plus the stateless parts above.
pub trait Platform: Clone + Send + Sync + Sized + 'static {
    /// The platform's own captured picture, which travels between the capture, the main thread
    /// and the encoder.
    type Picture: Clone + Send + 'static;
    /// The graphics device, or the platform's equivalent, shared by capture, thumbnails, pixels
    /// and video.
    type Device: Clone + Send;
    type Thumbs: Thumbs<Picture = Self::Picture>;
    type Video: VideoWriter<Picture = Self::Picture>;

    /// The main thread's setup, before anything else: COM's apartment, DPI awareness.
    fn init(&self);
    fn thread_init(&self) -> Arc<dyn ThreadInit>;
    fn clocks(&self) -> &dyn Clocks;
    fn displays(&self) -> &dyn Displays;
    fn disk(&self) -> Arc<dyn Disk>;
    fn jpeg(&self) -> Arc<dyn Jpeg>;
    /// A new text reader, on the calling thread.
    fn reader(&self) -> Box<dyn Reader>;
    /// Whether the session is locked or on another desktop, so the screen shows the lock screen.
    fn locked(&self) -> bool;
    /// Everything the frame worker's threads need.
    fn frame_io(&self) -> FrameIo;

    /// Creates the graphics device: with video support when `want_video`, else without, else the
    /// platform's software fallback.
    fn create_device(&self, want_video: bool, warp: bool) -> Result<Self::Device, PlatformError>;
    fn device_info(&self, device: &Self::Device) -> DeviceInfo;
    /// Whether the device was removed, even when no call has failed yet.
    fn device_removed(&self, device: &Self::Device) -> bool;
    /// The graphics memory this process uses, for the measurements.
    fn device_memory(&self, device: &Self::Device) -> Option<u64>;
    /// A capture of one monitor, real or the test's synthetic screen.
    fn source(
        &self,
        device: Self::Device,
        options: &SourceOptions,
        to_main: SyncSender<Ev<Self::Picture>>,
    ) -> Box<dyn Source>;
    fn thumbs(
        &self,
        device: &Self::Device,
        width: u32,
        height: u32,
    ) -> Result<Self::Thumbs, PlatformError>;
    /// Reads a captured frame back from the GPU, or its equivalent.
    fn pixels(
        &self,
        device: &Self::Device,
        frame: &Frame<Self::Picture>,
    ) -> Result<Pixels, PlatformError>;
    /// A maker for video files on this device; `None` when this computer has no encoder at all.
    fn video_maker(&self, device: &Self::Device)
        -> Option<Box<dyn VideoMaker<File = Self::Video>>>;
}
