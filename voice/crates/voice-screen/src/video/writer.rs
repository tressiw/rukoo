//! One fragmented MP4 file through Media Foundation's sink writer.
//!
//! Two sample paths (spec 2.5). The GPU path, when the device has video support and a hardware
//! H.264 encoder is there: the sink writer gets the device through a DXGI device manager and
//! each sample wraps the captured texture, so NVENC reads it without a copy. The system-memory
//! path otherwise, and with the `software-encoder` test switch: each frame is mapped through a
//! staging texture and copied row by row into a memory buffer, which Microsoft's software
//! encoder takes on a CI runner that has only WARP.
//!
//! A frame is written when the next one arrives, so each sample's duration is the real gap to
//! the next: the capture rate varies. Frames the screen process holds back because the screen is
//! still (`voice_screen_files::gate`) aren't samples; the newest of them waits in `held`. After a
//! quarter second without a sample the held frame goes in at its own capture time, or else the
//! last frame again, so a small change reaches the file within half a second and a still screen
//! still gets a sample every quarter second. The sample after the first one a second into a
//! fragment is a forced keyframe, which starts the next fragment, so fragments stay 1 to 2 s long
//! and a hard kill loses at most the open one.
//!
//! The file's media time is the capture time, so `video` line `t` plus media time is meeting time
//! (spec 2.4) and `clip.get` cuts by one offset per file. Two things in Media Foundation would
//! move it, and the writer stops both:
//!
//! - The video processor the sink writer puts in front of the encoder, on the system-memory path
//!   (ARGB to IYUV for the software encoder) and on the GPU path when it scales the video, converts
//!   the frame rate by default: it lays the frames on the nominal 15 fps grid, repeating a still
//!   screen's frame and dropping frames that come faster. The file then held one sample per 1/15 s
//!   and strayed up to a frame from the capture times. [`VideoWriter::open`] turns that off with
//!   `MF_XVP_DISABLE_FRC`.
//! - The sink counts media time in whole units of its timescale and rounds each gap between two
//!   sample times on its own. Gaps of 69.44 ms, every tenth refresh of a 144 Hz monitor, each
//!   gained a third of a unit, about a second an hour. Sample times sit on the timescale's grid
//!   ([`sample_time`]), so the gaps add up to the capture times exactly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use voice_screen_files::caps::video_size;
use windows::core::{Interface, GUID, HSTRING};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Variant::VARIANT;

use super::encoders;
use super::fault;
use super::mf::Mf;
use super::FPS;
use crate::capture::d3d::{is_device_lost, D3D};
use crate::platform::VideoWriter as PlatformWriter;
use crate::platform::{VideoError, VideoMaker, WriterOptions};

/// One second in 100 ns units.
const SECOND: i64 = 10_000_000;
/// `MF_MPEG4SINK_MIN_FRAGMENT_DURATION`, the shortest fragment. The sink compares it with the
/// fragment's media time when a sample arrives, puts that sample in the fragment and ends the
/// fragment at the next keyframe: the encoder's own, at a scene change, or the one the writer
/// forces on the sample after it ([`VideoWriter::emit`]). Forcing it on the sample that crosses the
/// line instead, as a plain "every second" rule does, lets the fragment run on to the keyframe
/// after that one: 2.0 to 2.4 s on a still screen in the live run.
const MIN_FRAGMENT: i64 = SECOND;
/// The longest a frame stays without a re-submit. With the 250 ms tick, samples are at most 0.5 s
/// apart, so a fragment ends before 1 s plus two gaps: under 2 s.
const RESUBMIT: i64 = SECOND / 4;
/// The timescale Media Foundation's fMP4 sink gives the track: 1,000 units for each frame of the
/// nominal rate in `MF_MT_FRAME_RATE`, so 15,000 a second.
const TIMESCALE: i64 = 1_000 * FPS as i64;
/// The duration of the last sample when the file ends, one frame at the nominal rate.
const LAST_UNITS: i64 = TIMESCALE / FPS as i64;
const LAST_DURATION: i64 = from_units(LAST_UNITS);

/// A media time in 100 ns as whole units of the file's timescale, rounded to the nearest.
const fn to_units(time: i64) -> i64 {
    (time * TIMESCALE + SECOND / 2) / SECOND
}

/// Whole units of the timescale in 100 ns, rounded to the nearest; [`to_units`] gives them back.
const fn from_units(units: i64) -> i64 {
    (units * SECOND + TIMESCALE / 2) / TIMESCALE
}

/// The media time of a frame captured `elapsed` (100 ns) after the file's first frame: on the
/// timescale's grid, and at least one unit after `last`, the sample before it.
fn sample_time(elapsed: i64, last: Option<i64>) -> i64 {
    let mut units = to_units(elapsed.max(0));
    if let Some(last) = last {
        units = units.max(to_units(last) + 1);
    }
    from_units(units)
}

/// Where a file whose last sample starts at media time `last` ends, in ms rounded down, as
/// `mp4::Index::to_ms` reads it from the file.
fn end_ms(last: i64) -> u64 {
    ((to_units(last) + LAST_UNITS) * 1_000 / TIMESCALE) as u64
}

pub struct VideoWriter {
    mf: &'static Mf,
    d3d: D3D,
    writer: Option<IMFSinkWriter>,
    stream: u32,
    codec: Option<ICodecAPI>,
    gpu: bool,
    staging: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
    video_width: u32,
    video_height: u32,
    start_boot: Option<u64>,
    /// Media time of the keyframe that started the open fragment.
    fragment_start: Option<i64>,
    /// A sample of the open fragment already lay `MIN_FRAGMENT` past its start, so the sink ends
    /// it at the next keyframe.
    fragment_full: bool,
    pending: Option<(ID3D11Texture2D, i64)>,
    last_time: Option<i64>,
    /// The last frame written, and its capture time.
    last_texture: Option<(ID3D11Texture2D, u64)>,
    /// The newest frame held back, newer than `last_texture`, and its capture time.
    held: Option<(ID3D11Texture2D, u64)>,
    path: PathBuf,
    encoder: String,
    hardware: bool,
    options: WriterOptions,
    /// Set by the `write-fail=video@` test switch: the byte stream's writes fail from now on.
    disk_failing: Arc<AtomicBool>,
}

impl VideoWriter {
    /// Creates `path` (it must not exist) for frames of `width` x `height`; the video gets the
    /// size of `caps::video_size`, which the sink writer's video processor scales to. On failure
    /// nothing stays at `path`.
    pub fn create(
        mf: &'static Mf,
        d3d: &D3D,
        path: &Path,
        width: u32,
        height: u32,
        options: WriterOptions,
    ) -> Result<VideoWriter, VideoError> {
        let hardware_encoders = if options.no_hardware {
            Vec::new()
        } else {
            encoders::hardware_h264(mf)
        };
        if hardware_encoders.is_empty() && !options.allow_software {
            return Err(VideoError::Software);
        }
        let gpu = d3d.video && !hardware_encoders.is_empty() && !options.allow_software;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| match crate::disk::is_disk_full(&e) {
                true => VideoError::Disk(e.to_string()),
                false => VideoError::Encoder(format!("can't create the video folder: {e}")),
            })?;
        }
        let (video_width, video_height) = video_size(width, height);
        let mut writer = VideoWriter {
            mf,
            d3d: d3d.clone(),
            writer: None,
            stream: 0,
            codec: None,
            gpu,
            staging: None,
            width,
            height,
            video_width,
            video_height,
            start_boot: None,
            fragment_start: None,
            fragment_full: false,
            pending: None,
            last_time: None,
            last_texture: None,
            held: None,
            path: path.to_path_buf(),
            encoder: String::new(),
            hardware: false,
            options,
            disk_failing: Arc::new(AtomicBool::new(false)),
        };
        match writer.open(&hardware_encoders) {
            Ok(()) => Ok(writer),
            Err(e) => {
                writer.abandon();
                let _ = std::fs::remove_file(path);
                Err(e)
            }
        }
    }

    fn open(
        &mut self,
        hardware_encoders: &[voice_screen_files::caps::EncoderInfo],
    ) -> Result<(), VideoError> {
        let mf = self.mf;
        let url = HSTRING::from(self.path.as_os_str());
        // Fail-if-exists, and Media Foundation shares the file for reading, so `clip.get` can
        // cut the fragments already written (spike 1).
        let bytes = mf
            .create_file(
                MF_ACCESSMODE_WRITE,
                MF_OPENMODE_FAIL_IF_EXIST,
                MF_FILEFLAGS_NONE,
                &url,
            )
            .map_err(classify)?;
        let bytes = match self.options.disk_fail_at {
            Some(_) => fault::wrap(bytes, self.disk_failing.clone()),
            None => bytes,
        };
        unsafe {
            let attributes = mf.create_attributes(6).map_err(classify)?;
            attributes
                .SetGUID(
                    &MF_TRANSCODE_CONTAINERTYPE,
                    &MFTranscodeContainerType_FMPEG4,
                )
                .map_err(classify)?;
            attributes
                .SetUINT32(
                    &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
                    u32::from(self.gpu),
                )
                .map_err(classify)?;
            if self.gpu {
                let (token, manager) = mf.create_dxgi_device_manager().map_err(classify)?;
                manager
                    .ResetDevice(&self.d3d.device, token)
                    .map_err(classify)?;
                attributes
                    .SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &manager)
                    .map_err(classify)?;
            }
            attributes
                .SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)
                .map_err(classify)?;
            // Encoded samples reach the file without waiting for more input, so a hard kill
            // loses less than the open fragment plus the encoder's queue.
            attributes.SetUINT32(&MF_LOW_LATENCY, 1).map_err(classify)?;
            attributes
                .SetUINT64(&MF_MPEG4SINK_MIN_FRAGMENT_DURATION, MIN_FRAGMENT as u64)
                .map_err(classify)?;
            let writer = mf
                .create_sink_writer(&bytes, &attributes)
                .map_err(classify)?;
            self.writer = Some(writer.clone());

            let pixels = u64::from(self.video_width) * u64::from(self.video_height);
            let output = mf.create_media_type().map_err(classify)?;
            output
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(classify)?;
            output
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
                .map_err(classify)?;
            output
                .SetUINT32(&MF_MT_AVG_BITRATE, (pixels * 2).min(8_000_000) as u32)
                .map_err(classify)?;
            output
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(classify)?;
            output
                .SetUINT64(&MF_MT_FRAME_SIZE, pack(self.video_width, self.video_height))
                .map_err(classify)?;
            output
                .SetUINT64(&MF_MT_FRAME_RATE, pack(FPS, 1))
                .map_err(classify)?;
            output
                .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))
                .map_err(classify)?;
            output
                .SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
                .map_err(classify)?;
            self.stream = writer.AddStream(&output).map_err(classify)?;

            let input = mf.create_media_type().map_err(classify)?;
            input
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(classify)?;
            input
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_ARGB32)
                .map_err(classify)?;
            input
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(classify)?;
            input
                .SetUINT64(&MF_MT_FRAME_SIZE, pack(self.width, self.height))
                .map_err(classify)?;
            input
                .SetUINT64(&MF_MT_FRAME_RATE, pack(FPS, 1))
                .map_err(classify)?;
            input
                .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))
                .map_err(classify)?;
            // Positive stride: rows run top-down, as in the texture.
            input
                .SetUINT32(&MF_MT_DEFAULT_STRIDE, self.width * 4)
                .map_err(classify)?;
            writer
                .SetInputMediaType(self.stream, &input, None)
                .map_err(classify)?;
            // The transforms are in place now; the video processor reads the switch when it
            // starts streaming.
            self.keep_frame_times(&writer);

            let mut codec = std::ptr::null_mut();
            if writer
                .GetServiceForStream(self.stream, &GUID::zeroed(), &ICodecAPI::IID, &mut codec)
                .is_ok()
                && !codec.is_null()
            {
                let codec = ICodecAPI::from_raw(codec);
                for (api, value) in [
                    (
                        CODECAPI_AVEncCommonRateControlMode,
                        eAVEncCommonRateControlMode_Quality.0 as u32,
                    ),
                    (CODECAPI_AVEncCommonQuality, 60),
                    (CODECAPI_AVLowLatencyMode, 1),
                    (CODECAPI_AVEncMPVGOPSize, 0),
                ] {
                    // An encoder that refuses one of these still records.
                    let _ = codec.SetValue(&api, &VARIANT::from(value));
                }
                self.codec = Some(codec);
            }
            writer.BeginWriting().map_err(classify)?;
            self.identify(&writer, hardware_encoders);
        }
        if !self.hardware && !self.options.allow_software {
            return Err(VideoError::Software);
        }
        if !self.gpu {
            self.staging = Some(
                self.d3d
                    .staging(self.width, self.height)
                    .map_err(classify)?,
            );
        }
        Ok(())
    }

    /// The transforms the sink writer put into the stream, in order, with their categories.
    fn transforms(&self, writer: &IMFSinkWriter) -> Vec<(GUID, IMFTransform)> {
        let Ok(ex) = writer.cast::<IMFSinkWriterEx>() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for index in 0..8 {
            let mut category = GUID::zeroed();
            let mut transform: Option<IMFTransform> = None;
            if unsafe {
                ex.GetTransformForStream(self.stream, index, Some(&mut category), &mut transform)
            }
            .is_err()
            {
                break;
            }
            if let Some(transform) = transform {
                found.push((category, transform));
            }
        }
        found
    }

    /// Turns off the frame rate conversion of every video processor in the stream, so each
    /// frame keeps its capture time (module docs). A processor that refuses still records, with
    /// samples up to a frame from their capture times.
    fn keep_frame_times(&self, writer: &IMFSinkWriter) {
        for (category, transform) in self.transforms(writer) {
            if category != MFT_CATEGORY_VIDEO_PROCESSOR {
                continue;
            }
            let set = unsafe { transform.GetAttributes() }
                .and_then(|attributes| unsafe { attributes.SetUINT32(&MF_XVP_DISABLE_FRC, 1) });
            if let Err(e) = set {
                crate::log(format_args!(
                    "the video processor keeps converting the frame rate: {e}"
                ));
            }
        }
    }

    /// Which encoder the sink writer loaded: a transform without a hardware vendor id is
    /// software, whatever was asked for.
    fn identify(
        &mut self,
        writer: &IMFSinkWriter,
        hardware_encoders: &[voice_screen_files::caps::EncoderInfo],
    ) {
        let found = self
            .transforms(writer)
            .into_iter()
            .rev()
            .find(|(category, _)| *category == MFT_CATEGORY_VIDEO_ENCODER)
            .map(|(_, transform)| transform);
        let (name, vendor) = match found.and_then(|t| unsafe { t.GetAttributes() }.ok()) {
            Some(attributes) => (
                Mf::string_attribute(&attributes, &MFT_FRIENDLY_NAME_Attribute),
                Mf::string_attribute(&attributes, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute),
            ),
            // No answer from the sink writer: trust the path it was given.
            None => (None, self.gpu.then(String::new)),
        };
        self.hardware = vendor.is_some();
        let by_vendor = vendor.as_deref().and_then(|v| {
            hardware_encoders
                .iter()
                .find(|e| !v.is_empty() && e.vendor.as_deref() == Some(v))
                .or(hardware_encoders.first())
        });
        self.encoder = name
            .or_else(|| by_vendor.map(|e| e.name.clone()))
            .unwrap_or_else(|| match self.hardware {
                true => "Hardware H.264 encoder".into(),
                false => "H264 Encoder MFT".into(),
            });
    }

    /// The encoder's name and whether it is hardware.
    pub fn encoder(&self) -> (&str, bool) {
        (&self.encoder, self.hardware)
    }

    /// The video's own size.
    pub fn video_size(&self) -> (u32, u32) {
        (self.video_width, self.video_height)
    }

    /// The frame captured at `boot` (interrupt time). It goes into the file when the next frame,
    /// a re-submit or the end gives its duration.
    pub fn write(&mut self, texture: &ID3D11Texture2D, boot: u64) -> Result<(), VideoError> {
        self.submit(texture, boot, boot)
    }

    /// Keeps the frame captured at `boot` for the next re-submit, unless the last frame written is
    /// newer. A newer held frame replaces it; a written one drops it.
    pub fn hold(&mut self, texture: &ID3D11Texture2D, boot: u64) {
        if self.last_texture.as_ref().is_none_or(|(_, at)| boot > *at) {
            self.held = Some((texture.clone(), boot));
        }
    }

    /// A sample of `texture` at media time `boot`; `captured` is when the picture was captured,
    /// earlier than `boot` for a re-submit.
    fn submit(
        &mut self,
        texture: &ID3D11Texture2D,
        boot: u64,
        captured: u64,
    ) -> Result<(), VideoError> {
        let start = *self.start_boot.get_or_insert(boot);
        let time = sample_time(boot.saturating_sub(start) as i64, self.last_time);
        let at = self.options.start_t + time as u64 / 10_000;
        if self.options.encoder_fail_at.is_some_and(|ms| at >= ms) {
            return Err(VideoError::Encoder("encoder-fail-at test switch".into()));
        }
        if self.options.disk_fail_at.is_some_and(|ms| at >= ms) {
            // The sink writer finds out when it next writes the file, as on a real full disk.
            self.disk_failing.store(true, Ordering::SeqCst);
        }
        if let Some((previous, previous_time)) = self.pending.take() {
            self.emit(&previous, previous_time, time - previous_time)?;
        }
        self.pending = Some((texture.clone(), time));
        self.last_time = Some(time);
        self.last_texture = Some((texture.clone(), captured));
        if self.held.as_ref().is_some_and(|(_, at)| *at <= captured) {
            self.held = None;
        }
        Ok(())
    }

    /// After a quarter second without a new sample, the held frame goes in at its capture time,
    /// or the last one again at `boot_now`, so a still screen still gets a keyframe and a fragment
    /// boundary every 1 to 2 s.
    pub fn tick(&mut self, boot_now: u64) -> Result<(), VideoError> {
        let (Some(start), Some(last)) = (self.start_boot, self.last_time) else {
            return Ok(());
        };
        let now = boot_now.saturating_sub(start) as i64;
        if now - last < RESUBMIT {
            return Ok(());
        }
        if let Some((texture, boot)) = self.held.take() {
            return self.write(&texture, boot);
        }
        match self.last_texture.clone() {
            Some((texture, captured)) => self.submit(&texture, boot_now, captured),
            None => Ok(()),
        }
    }

    /// Writes the held frame, as at the end of the file, so the file ends on the newest picture.
    pub fn flush(&mut self) -> Result<(), VideoError> {
        match self.held.take() {
            Some((texture, boot)) => self.write(&texture, boot),
            None => Ok(()),
        }
    }

    fn emit(
        &mut self,
        texture: &ID3D11Texture2D,
        time: i64,
        duration: i64,
    ) -> Result<(), VideoError> {
        let writer = self
            .writer
            .clone()
            .ok_or(VideoError::Encoder("closed".into()))?;
        let buffer = self.buffer(texture)?;
        unsafe {
            let sample = self.mf.create_sample().map_err(classify)?;
            sample.AddBuffer(&buffer).map_err(classify)?;
            sample.SetSampleTime(time).map_err(classify)?;
            sample
                .SetSampleDuration(duration.max(1))
                .map_err(classify)?;
            match self.fragment_start {
                Some(start) if !self.fragment_full => {
                    self.fragment_full = time - start >= MIN_FRAGMENT;
                }
                _ => {
                    if let Some(codec) = &self.codec {
                        let _ =
                            codec.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32));
                    }
                    self.fragment_start = Some(time);
                    self.fragment_full = false;
                }
            }
            writer.WriteSample(self.stream, &sample).map_err(classify)?;
        }
        Ok(())
    }

    fn buffer(&self, texture: &ID3D11Texture2D) -> Result<IMFMediaBuffer, VideoError> {
        if self.gpu {
            let buffer = self
                .mf
                .create_dxgi_surface_buffer(texture)
                .map_err(classify)?;
            unsafe {
                let len = buffer
                    .cast::<IMF2DBuffer>()
                    .and_then(|b| b.GetContiguousLength())
                    .map_err(classify)?;
                buffer.SetCurrentLength(len).map_err(classify)?;
            }
            return Ok(buffer);
        }
        let staging = self
            .staging
            .as_ref()
            .ok_or(VideoError::Encoder("no staging texture".into()))?;
        let row = self.width as usize * 4;
        let len = (row * self.height as usize) as u32;
        let buffer = self.mf.create_memory_buffer(len).map_err(classify)?;
        unsafe {
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None).map_err(classify)?;
            let copied =
                self.d3d
                    .read_rows(staging, texture, self.width, self.height, |y, bytes| {
                        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.add(y * row), row);
                    });
            let _ = buffer.Unlock();
            copied.map_err(classify)?;
            buffer.SetCurrentLength(len).map_err(classify)?;
        }
        Ok(buffer)
    }

    /// The meeting time where the file's last sample ends: the `video` line's `t` plus the media
    /// time the file holds, in the units the sink writes.
    pub fn end_t(&self) -> u64 {
        match self.last_time {
            Some(last) => self.options.start_t + end_ms(last),
            None => self.options.start_t,
        }
    }

    /// Writes the last frame, finalizes the file and returns its size. A writer that failed
    /// before is abandoned instead: the file stays readable up to its last fragment.
    pub fn finish(mut self) -> Result<u64, VideoError> {
        let mut result = Ok(());
        if let Some((texture, time)) = self.pending.take() {
            result = self.emit(&texture, time, LAST_DURATION);
        }
        if let Some(writer) = self.writer.take() {
            let finalized = unsafe { writer.Finalize() }.map_err(classify);
            result = result.and(finalized);
        }
        let bytes = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        result.map(|()| bytes)
    }

    /// Drops the sink writer without `Finalize`, as after an error; returns the bytes on disk.
    pub fn abandon(&mut self) -> u64 {
        self.pending = None;
        self.held = None;
        self.writer = None;
        self.codec = None;
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

/// The Windows sink writer behind the platform's writer trait. The methods are the writer's own;
/// this only names the frame type the orchestration hands over.
impl PlatformWriter for VideoWriter {
    type Picture = crate::capture::Texture;

    fn write(&mut self, picture: &crate::capture::Texture, boot: u64) -> Result<(), VideoError> {
        VideoWriter::write(self, &picture.0, boot)
    }

    fn hold(&mut self, picture: &crate::capture::Texture, boot: u64) {
        VideoWriter::hold(self, &picture.0, boot);
    }

    fn tick(&mut self, boot_now: u64) -> Result<(), VideoError> {
        VideoWriter::tick(self, boot_now)
    }

    fn flush(&mut self) -> Result<(), VideoError> {
        VideoWriter::flush(self)
    }

    fn finish(self) -> Result<u64, VideoError> {
        VideoWriter::finish(self)
    }

    fn abandon(&mut self) -> u64 {
        VideoWriter::abandon(self)
    }

    fn encoder(&self) -> (&str, bool) {
        VideoWriter::encoder(self)
    }

    fn video_size(&self) -> (u32, u32) {
        VideoWriter::video_size(self)
    }

    fn end_t(&self) -> u64 {
        VideoWriter::end_t(self)
    }
}

/// Makes video files with Media Foundation's sink writer, on the graphics device the capture uses.
pub struct WinVideoMaker {
    mf: &'static Mf,
    d3d: D3D,
}

impl WinVideoMaker {
    pub fn new(mf: &'static Mf, d3d: D3D) -> WinVideoMaker {
        WinVideoMaker { mf, d3d }
    }
}

impl VideoMaker for WinVideoMaker {
    type File = VideoWriter;

    fn create(
        &mut self,
        path: &Path,
        width: u32,
        height: u32,
        options: WriterOptions,
    ) -> Result<VideoWriter, VideoError> {
        VideoWriter::create(self.mf, &self.d3d, path, width, height, options)
    }
}

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// Sorts a Media Foundation or Direct3D error into what the drop order needs.
pub fn classify(e: windows::core::Error) -> VideoError {
    let code = e.code().0 as u32;
    if is_device_lost(&e) {
        VideoError::DeviceLost
    } else if matches!(code, 0x8007_0070 | 0x8003_0070 | 0x8007_0027) {
        VideoError::Disk(format!("{code:#010x}"))
    } else {
        VideoError::Encoder(format!("{code:#010x} {}", e.message()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::d3d;

    /// Gaps between frames in 100 ns: a burst faster than the nominal 15 fps, a still screen's
    /// quarter-second re-submits, and every tenth refresh of a 144 Hz monitor.
    const GAPS: [i64; 11] = [
        400_000, 400_000, 400_000, 2_500_000, 2_500_000, 694_444, 694_444, 694_444, 694_444,
        694_444, 694_444,
    ];

    /// The media time the fMP4 sink makes of a file's sample times, in units: it rounds each gap
    /// between two sample times on its own and adds the last sample's duration.
    fn sink_units(times: &[i64]) -> i64 {
        let gaps: i64 = times.windows(2).map(|w| to_units(w[1] - w[0])).sum();
        gaps + to_units(LAST_DURATION)
    }

    #[test]
    fn units_round_trip() {
        assert_eq!(
            (TIMESCALE, LAST_UNITS, LAST_DURATION),
            (15_000, 1_000, 666_667)
        );
        for units in [0, 1, 2, 999, 1_000, 1_001, 54_000_000, 432_000_000] {
            assert_eq!(to_units(from_units(units)), units);
        }
        assert_eq!(to_units(333), 0);
        assert_eq!(to_units(334), 1);
    }

    #[test]
    fn sample_times_stay_on_the_grid_and_go_forward() {
        assert_eq!(sample_time(0, None), 0);
        assert_eq!(sample_time(694_444, None), from_units(1_042));
        // A frame captured no later than the last sample still comes after it.
        let last = sample_time(10_000_000, None);
        assert_eq!(sample_time(10_000_000, Some(last)), from_units(15_001));
        assert_eq!(sample_time(9_000_000, Some(last)), from_units(15_001));
        assert_eq!(sample_time(-5, None), 0);
    }

    /// An hour of each gap: the sink's rounded gaps add up to the capture times, within half a
    /// unit, and `end_ms` is where a reader of the file sees the media end. Capture times as they
    /// are would gain a third of a unit at every 144 Hz gap, 1.2 s in the hour.
    #[test]
    fn rounded_gaps_add_up_to_the_capture_times() {
        for gap in [694_444i64, 666_683, 2_500_013, 400_000] {
            let frames = 3_600 * SECOND / gap;
            let (mut times, mut last) = (Vec::new(), None);
            for n in 0..frames {
                let time = sample_time(n * gap, last);
                times.push(time);
                last = Some(time);
            }
            let captured_end = (frames - 1) * gap + LAST_DURATION;
            let units = sink_units(&times);
            assert!(
                (units * SECOND - captured_end * TIMESCALE).abs() <= SECOND / 2,
                "gap {gap}: the file holds {units} units, the capture {captured_end} x 100 ns"
            );
            assert_eq!(units, to_units(*times.last().unwrap()) + LAST_UNITS);
            assert_eq!(
                end_ms(*times.last().unwrap()),
                (units * 1_000 / TIMESCALE) as u64
            );
        }
        let raw: Vec<i64> = (0..51_840).map(|n| n * 694_444).collect();
        let drift = sink_units(&raw) - (to_units(*raw.last().unwrap()) + LAST_UNITS);
        assert!(
            drift > 15_000,
            "capture times as they are drift by {drift} units"
        );
    }

    /// Frames faster and slower than the nominal rate, through the real sink writer: the file
    /// holds one sample per frame and its media time ends where `end_t` says, to the unit.
    /// Microsoft's software encoder on WARP, as on a CI runner, takes the frames through a video
    /// processor that would lay them on a 15 fps grid. With a hardware encoder, the GPU path runs
    /// too: at a size it scales, which also puts a video processor in front of the encoder, and at
    /// a size it keeps, where only the sink's rounding of each gap could move the media time.
    #[test]
    fn the_file_keeps_the_capture_times() {
        crate::com::init_mta();
        let mf = super::super::mf::shared().expect("Media Foundation");
        let dir = std::env::temp_dir().join(format!(
            "voice-screen-writer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cases = [
            ("software", d3d::create(false, true), (320, 180), true),
            ("gpu-scaled", d3d::create(true, false), (5120, 1440), false),
            ("gpu", d3d::create(true, false), (1920, 1080), false),
        ];
        for (name, device, (width, height), software) in cases {
            let device = device.unwrap();
            let path = dir.join(format!("{name}.mp4"));
            let options = WriterOptions {
                allow_software: software,
                start_t: 1_000,
                ..Default::default()
            };
            let mut writer = match VideoWriter::create(mf, &device, &path, width, height, options) {
                Ok(writer) => writer,
                Err(VideoError::Software) if !software => {
                    println!("{name}: skipped, no hardware H.264 encoder");
                    continue;
                }
                Err(e) => panic!("{name}: {e}"),
            };
            let textures: Vec<_> = [40u8, 200]
                .iter()
                .map(|&v| {
                    let pixels = vec![v; (width * height * 4) as usize];
                    device.upload(width, height, &pixels).unwrap()
                })
                .collect();
            let frames = 120;
            let mut boot = 7_000_000_000u64;
            for n in 0..frames {
                writer.write(&textures[n % 2], boot).unwrap();
                boot += GAPS[n % GAPS.len()] as u64;
            }
            let (end_t, last) = (writer.end_t(), writer.last_time.unwrap());
            writer.finish().unwrap();
            let index = voice_screen_files::mp4::index_path(&path).unwrap();
            let samples: u32 = index.fragments.iter().map(|f| f.samples).sum();
            let units = index.fragments.last().unwrap().end();
            assert_eq!(index.timescale as i64, TIMESCALE, "{name}");
            assert_eq!(samples as usize, frames, "{name}: a sample per frame");
            assert_eq!(units as i64, to_units(last) + LAST_UNITS, "{name}");
            assert_eq!(end_t, 1_000 + index.to_ms(units), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Held frames: the newest goes in at its own capture time with the next re-submit or at the
    /// end, one older than a frame written is dropped, and one older than the last frame written
    /// isn't taken. Software encoder on WARP, as on a CI runner.
    #[test]
    fn held_frames_go_in_at_their_capture_time() {
        crate::com::init_mta();
        let mf = super::super::mf::shared().expect("Media Foundation");
        let dir = std::env::temp_dir().join(format!(
            "voice-screen-held-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let device = d3d::create(false, true).unwrap();
        let (width, height) = (320, 180);
        let path = dir.join("held.mp4");
        let options = WriterOptions {
            allow_software: true,
            start_t: 1_000,
            ..Default::default()
        };
        let mut writer = VideoWriter::create(mf, &device, &path, width, height, options).unwrap();
        let texture = |v: u8| {
            device
                .upload(width, height, &vec![v; (width * height * 4) as usize])
                .unwrap()
        };
        let ms = |n: u64| 7_000_000_000u64 + n * 10_000;
        writer.write(&texture(10), ms(0)).unwrap();
        writer.hold(&texture(20), ms(100));
        // Not a quarter second yet: nothing goes in.
        writer.tick(ms(200)).unwrap();
        assert_eq!(writer.last_time, Some(0));
        writer.tick(ms(300)).unwrap();
        assert_eq!(writer.last_time, Some(sample_time(100 * 10_000, Some(0))));
        writer.hold(&texture(30), ms(350));
        writer.write(&texture(40), ms(400)).unwrap();
        assert!(
            writer.held.is_none(),
            "a frame written drops an older held one"
        );
        writer.hold(&texture(50), ms(390));
        assert!(writer.held.is_none(), "older than the last frame written");
        writer.hold(&texture(60), ms(450));
        writer.flush().unwrap();
        let (end_t, last) = (writer.end_t(), writer.last_time.unwrap());
        assert_eq!(last, sample_time(450 * 10_000, None));
        writer.finish().unwrap();
        let index = voice_screen_files::mp4::index_path(&path).unwrap();
        let samples: u32 = index.fragments.iter().map(|f| f.samples).sum();
        assert_eq!(samples, 4, "the first frame, two held ones and one written");
        let units = index.fragments.last().unwrap().end();
        assert_eq!(units as i64, to_units(last) + LAST_UNITS);
        assert_eq!(end_t, 1_000 + index.to_ms(units));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
