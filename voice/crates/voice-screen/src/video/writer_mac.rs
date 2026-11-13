//! One fragmented MP4 file from VideoToolbox's H.264 samples, written by
//! `voice_screen_files::fmp4`.
//!
//! The file's media time 0 is the `video` line's `t`, and every sample carries its frame's
//! capture time, so `clip.get` cuts by one offset per file. A frame is written when the next one
//! arrives, so a sample's duration is the real gap to the next: the capture rate varies. Frames
//! the screen process holds back because the screen is still aren't samples; the newest waits in
//! `held` and goes in at its own capture time with the next re-submit. After a quarter second
//! without a sample the held frame goes in, or else the last frame again, so a still screen still
//! gets a keyframe and a fragment boundary every 1 to 2 s.
//!
//! The writer asks VideoToolbox for a keyframe whenever the MP4 writer is about to close a
//! fragment (`Mp4Writer::needs_keyframe`), so the next fragment starts on a sync sample and a
//! hard kill loses at most the open one.
//!
//! The video's size is `caps::video_size`'s, which may be smaller than the captured frame (a 5K
//! display becomes 4096 wide): a frame of another size is scaled down here, on the CPU, before
//! it reaches the encoder. A frame that is already the video's size goes to VideoToolbox as it
//! is, which is what ScreenCaptureKit's bi-planar buffers will do.

use std::path::Path;

use voice_screen_files::caps::video_size;
use voice_screen_files::fmp4::{self, Mp4Writer};

use super::vt::{self, Encoder, Kind};
use crate::capture::mac::PixelBuffer;
use crate::disk;
use crate::platform::{VideoError, VideoMaker, VideoWriter as PlatformWriter, WriterOptions};

/// The longest a frame stays without a re-submit, in 100 ns units: a quarter second. With the
/// 250 ms tick, samples are at most 0.5 s apart, so a fragment ends before 1 s plus two gaps.
const RESUBMIT: u64 = 2_500_000;

pub struct MacVideoWriter {
    mp4: Option<Mp4Writer>,
    encoder: Encoder,
    /// The video's own size, as the encoder takes it.
    width: u32,
    height: u32,
    start_boot: Option<u64>,
    /// When the last frame was submitted, its picture, and whether one was submitted at all.
    last_boot: Option<u64>,
    last_picture: Option<PixelBuffer>,
    /// The newest frame held back, newer than `last_boot`, and its capture time.
    held: Option<(PixelBuffer, u64)>,
    /// The file's first sample, which must be a keyframe.
    first: bool,
    options: WriterOptions,
    encoder_name: String,
    hardware: bool,
}

impl MacVideoWriter {
    /// Creates `path` (it must not exist) for frames of `width` x `height`; the video gets the
    /// size of `caps::video_size`. On failure nothing stays at `path`.
    pub fn create(
        path: &Path,
        width: u32,
        height: u32,
        options: WriterOptions,
    ) -> Result<MacVideoWriter, VideoError> {
        let (video_width, video_height) = video_size(width, height);
        if video_width == 0 || video_height == 0 {
            return Err(VideoError::Encoder("the frame has no size".into()));
        }
        // Only the `software-encoder` switch allows Apple's software encoder; `no-hardware`
        // alone means what it says, as on Windows.
        let software = options.allow_software;
        if !software && (options.no_hardware || !vt::hardware_available()) {
            return Err(VideoError::Software);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| match disk::is_disk_full(&e) {
                true => VideoError::Disk(e.to_string()),
                false => VideoError::Encoder(format!("can't create the video folder: {e}")),
            })?;
        }
        let kind = match software {
            true => Kind::Software,
            false => Kind::Hardware,
        };
        let mut encoder =
            Encoder::create(video_width, video_height, kind).map_err(VideoError::Encoder)?;
        // The file needs the encoder's SPS and PPS, which VideoToolbox only hands out with the
        // first sample: one black frame before the file exists gives them.
        let black = PixelBuffer::new(video_width, video_height)
            .ok_or_else(|| VideoError::Encoder("can't allocate the encoder's frame".into()))?;
        black.fill([0, 0, 0, 0xff]);
        encoder
            .encode(&black, 0, true)
            .map_err(VideoError::Encoder)?;
        let (sps, pps) = encoder
            .parameter_sets()
            .ok_or_else(|| VideoError::Encoder("the encoder gave no parameter sets".into()))?;
        encoder.identify();
        // The encoder's own size is what the file declares, even when it differs from the ask.
        let (video_width, video_height) = encoder.size();
        let mp4 = Mp4Writer::create(
            path,
            fmp4::TIMESCALE,
            u16::try_from(video_width)
                .map_err(|_| VideoError::Encoder("the video is too wide for the format".into()))?,
            u16::try_from(video_height)
                .map_err(|_| VideoError::Encoder("the video is too tall for the format".into()))?,
            &sps,
            &pps,
        )
        .map_err(classify)?;
        let (encoder_name, hardware) = {
            let (name, hardware) = encoder.description();
            (name.to_string(), hardware)
        };
        Ok(MacVideoWriter {
            mp4: Some(mp4),
            encoder,
            width: video_width,
            height: video_height,
            start_boot: None,
            last_boot: None,
            last_picture: None,
            held: None,
            first: true,
            options,
            encoder_name,
            hardware,
        })
    }

    /// The frame captured at `boot` (interrupt time). It goes into the file when the next frame,
    /// a re-submit or the end gives it its duration.
    pub fn write(&mut self, picture: &PixelBuffer, boot: u64) -> Result<(), VideoError> {
        let start = *self.start_boot.get_or_insert(boot);
        let at = self.options.start_t + boot.saturating_sub(start) / 10_000;
        if self.options.encoder_fail_at.is_some_and(|fail| at >= fail) {
            return Err(VideoError::Encoder("encoder-fail-at test switch".into()));
        }
        if self.options.disk_fail_at.is_some_and(|fail| at >= fail) {
            // The next write of the file fails, as on a real full disk.
            return Err(VideoError::Disk("write-fail test switch".into()));
        }
        let input = self.encoder_input(picture)?;
        let mp4 = self
            .mp4
            .as_mut()
            .ok_or_else(|| VideoError::Encoder("the file is closed".into()))?;
        let sync = self.first || mp4.needs_keyframe(at);
        let samples = self
            .encoder
            .encode(&input, at, sync)
            .map_err(VideoError::Encoder)?;
        self.first = false;
        for sample in samples {
            mp4.add(at, sample.sync, &sample.data).map_err(classify)?;
        }
        self.last_picture = Some(picture.clone());
        self.last_boot = Some(boot);
        if self.held.as_ref().is_some_and(|(_, held)| *held <= boot) {
            self.held = None;
        }
        Ok(())
    }

    /// Keeps the frame captured at `boot` for the next re-submit, unless the last frame written
    /// is newer. A newer held frame replaces it; a written one drops it.
    pub fn hold(&mut self, picture: &PixelBuffer, boot: u64) {
        if self.last_boot.is_none_or(|at| boot > at) {
            self.held = Some((picture.clone(), boot));
        }
    }

    /// After a quarter second without a new sample, the held frame goes in at its capture time,
    /// or the last one again at `boot_now`, so a still screen still gets a keyframe and a
    /// fragment boundary every 1 to 2 s.
    pub fn tick(&mut self, boot_now: u64) -> Result<(), VideoError> {
        let (Some(_), Some(last)) = (self.start_boot, self.last_boot) else {
            return Ok(());
        };
        if boot_now.saturating_sub(last) < RESUBMIT {
            return Ok(());
        }
        if let Some((picture, boot)) = self.held.take() {
            return self.write(&picture, boot);
        }
        match self.last_picture.clone() {
            Some(picture) => self.write(&picture, boot_now),
            None => Ok(()),
        }
    }

    /// Writes the held frame, as at the end of the file, so the file ends on the newest picture.
    pub fn flush(&mut self) -> Result<(), VideoError> {
        match self.held.take() {
            Some((picture, boot)) => self.write(&picture, boot),
            None => Ok(()),
        }
    }

    /// The meeting time where the file's last sample ends.
    pub fn end_t(&self) -> u64 {
        let end = self.mp4.as_ref().map(|mp4| mp4.end()).unwrap_or(0);
        self.options.start_t.max(end)
    }

    /// Writes the open fragment, finalizes the file and returns its size. A writer that failed
    /// before is abandoned instead: the file stays readable up to its last fragment.
    pub fn finish(mut self) -> Result<u64, VideoError> {
        let Some(mp4) = self.mp4.take() else {
            return Ok(0);
        };
        mp4.finish().map_err(classify)
    }

    /// Drops the writer without finalizing, as after an error; returns the bytes on disk.
    pub fn abandon(&mut self) -> u64 {
        self.held = None;
        self.last_picture = None;
        self.mp4.take().map(|mp4| mp4.abandon()).unwrap_or(0)
    }

    /// The frame as VideoToolbox takes it: itself when it is the video's size, else scaled down
    /// to it. Only a monitor too large for the encoder gets scaled, and that goes through BGRA
    /// pixels, a bi-planar frame converted first.
    fn encoder_input(&self, picture: &PixelBuffer) -> Result<PixelBuffer, VideoError> {
        if (picture.width(), picture.height()) == (self.width, self.height) {
            return Ok(picture.clone());
        }
        let pixels = picture
            .pixels()
            .ok_or_else(|| VideoError::Encoder("can't read the frame".into()))?;
        let scaled = crate::frames::upscale::scale_to(&pixels, self.width, self.height);
        PixelBuffer::from_pixels(&scaled)
            .ok_or_else(|| VideoError::Encoder("can't make the encoder's frame".into()))
    }
}

impl PlatformWriter for MacVideoWriter {
    type Picture = PixelBuffer;

    fn write(&mut self, picture: &PixelBuffer, boot: u64) -> Result<(), VideoError> {
        MacVideoWriter::write(self, picture, boot)
    }

    fn hold(&mut self, picture: &PixelBuffer, boot: u64) {
        MacVideoWriter::hold(self, picture, boot);
    }

    fn tick(&mut self, boot_now: u64) -> Result<(), VideoError> {
        MacVideoWriter::tick(self, boot_now)
    }

    fn flush(&mut self) -> Result<(), VideoError> {
        MacVideoWriter::flush(self)
    }

    fn finish(self) -> Result<u64, VideoError> {
        MacVideoWriter::finish(self)
    }

    fn abandon(&mut self) -> u64 {
        MacVideoWriter::abandon(self)
    }

    fn encoder(&self) -> (&str, bool) {
        (&self.encoder_name, self.hardware)
    }

    fn video_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn end_t(&self) -> u64 {
        MacVideoWriter::end_t(self)
    }
}

/// Makes video files with VideoToolbox and `voice_screen_files::fmp4`.
pub struct MacVideoMaker;

impl MacVideoMaker {
    pub fn new() -> MacVideoMaker {
        MacVideoMaker
    }
}

impl Default for MacVideoMaker {
    fn default() -> Self {
        MacVideoMaker::new()
    }
}

impl VideoMaker for MacVideoMaker {
    type File = MacVideoWriter;

    fn create(
        &mut self,
        path: &Path,
        width: u32,
        height: u32,
        options: WriterOptions,
    ) -> Result<MacVideoWriter, VideoError> {
        MacVideoWriter::create(path, width, height, options)
    }
}

/// Sorts an fMP4 writer error into what the drop order needs.
fn classify(e: fmp4::Error) -> VideoError {
    match &e {
        fmp4::Error::Io(io) if disk::is_disk_full(io) => VideoError::Disk(io.to_string()),
        fmp4::Error::Io(io) => VideoError::Encoder(format!("can't write the video file: {io}")),
        other => VideoError::Encoder(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use voice_screen_files::mp4;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voice-screen-mac-writer-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn software() -> WriterOptions {
        WriterOptions {
            allow_software: true,
            start_t: 1_000,
            ..Default::default()
        }
    }

    /// A file written from frames at the video's own size: one sample per frame, the timescale
    /// RECORDING.md asks for, every fragment starting on a sync sample, and `end_t` where the
    /// file's media ends.
    #[test]
    fn the_file_holds_the_frames_and_cuts_a_clip() {
        let dir = temp_dir("frames");
        let path = dir.join("000001.mp4");
        let (width, height) = (320u32, 180u32);
        let mut writer = MacVideoWriter::create(&path, width, height, software()).unwrap();
        assert!(!writer.encoder().1, "the software encoder");
        assert!(!writer.encoder().0.is_empty());
        let picture = PixelBuffer::new(width, height).unwrap();
        picture.fill([0x30, 0x60, 0x90, 0xff]);
        let frames = 90u64;
        let mut boot = 7_000_000_000u64;
        for n in 0..frames {
            writer.write(&picture, boot).unwrap();
            boot += 666_667;
            if n % 2 == 0 {
                picture.fill([0x30, 0x60, 0x90, 0xff]);
            } else {
                picture.fill([0xa0, 0x40, 0x10, 0xff]);
            }
        }
        let end_t = writer.end_t();
        writer.finish().unwrap();
        let index = mp4::index_path(&path).unwrap();
        assert_eq!(index.timescale, fmp4::TIMESCALE);
        assert_eq!(index.complete_len, index.file_len);
        let samples: u32 = index.fragments.iter().map(|f| f.samples).sum();
        assert_eq!(samples as u64, frames, "a sample per frame");
        assert!(index.fragments.iter().all(|f| f.first_sync));
        let last = index.fragments.last().unwrap();
        assert_eq!(end_t, 1_000 + index.to_ms(last.end()));
        // The mp4 reader's cutter takes a clip out of the writer's own file.
        let clip = dir.join("clip.mp4");
        let cut = mp4::cut(&path, 1_500, 2_500, &clip).unwrap().unwrap();
        assert!(cut.start_ms <= 1_500 && cut.end_ms >= 2_500, "{cut:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A frame bigger than the video is scaled to `caps::video_size`; a frame at its size is
    /// taken as it is.
    #[test]
    fn a_large_frame_is_scaled_to_the_video() {
        let dir = temp_dir("scaled");
        let path = dir.join("000001.mp4");
        let (width, height) = (5120u32, 1440u32);
        let mut writer = MacVideoWriter::create(&path, width, height, software()).unwrap();
        assert_eq!(writer.video_size(), (4096, 1152));
        let picture = PixelBuffer::new(width, height).unwrap();
        picture.fill([0x10, 0x20, 0x30, 0xff]);
        writer.write(&picture, 5_000_000_000).unwrap();
        let end_t = writer.end_t();
        writer.finish().unwrap();
        let index = mp4::index_path(&path).unwrap();
        assert_eq!(index.complete_len, index.file_len);
        assert_eq!(
            end_t,
            1_000 + index.to_ms(index.fragments.last().unwrap().end())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Held frames: the newest goes in at its own capture time with the next re-submit or at the
    /// end, one older than a frame written is dropped, and one older than the last frame written
    /// isn't taken.
    #[test]
    fn held_frames_go_in_at_their_capture_time() {
        let dir = temp_dir("held");
        let path = dir.join("000001.mp4");
        let (width, height) = (320u32, 180u32);
        let mut writer = MacVideoWriter::create(&path, width, height, software()).unwrap();
        let picture = |v: u8| {
            let picture = PixelBuffer::new(width, height).unwrap();
            picture.fill([v, v, v, 0xff]);
            picture
        };
        let ms = |n: u64| 7_000_000_000u64 + n * 10_000;
        writer.write(&picture(10), ms(0)).unwrap();
        writer.hold(&picture(20), ms(100));
        // Not a quarter second yet: nothing goes in.
        writer.tick(ms(200)).unwrap();
        writer.tick(ms(300)).unwrap();
        writer.hold(&picture(30), ms(350));
        writer.write(&picture(40), ms(400)).unwrap();
        assert!(
            writer.held.is_none(),
            "a frame written drops an older held one"
        );
        writer.hold(&picture(50), ms(390));
        assert!(writer.held.is_none(), "older than the last frame written");
        writer.hold(&picture(60), ms(450));
        writer.flush().unwrap();
        let end_t = writer.end_t();
        writer.finish().unwrap();
        let index = mp4::index_path(&path).unwrap();
        let samples: u32 = index.fragments.iter().map(|f| f.samples).sum();
        // The first frame, the held one that went in at its capture time, the frame written at
        // 400 ms and the one flushed at 450 ms.
        assert_eq!(samples, 4, "{index:?}");
        assert_eq!(
            end_t,
            1_000 + index.to_ms(index.fragments.last().unwrap().end())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The test switches reach the writer: an encoder failure and a full disk are what the drop
    /// order decides by.
    #[test]
    fn the_test_switches_fail_as_they_say() {
        let dir = temp_dir("faults");
        let (width, height) = (320u32, 180u32);
        let picture = PixelBuffer::new(width, height).unwrap();
        let mut options = software();
        options.encoder_fail_at = Some(1_100);
        let path = dir.join("encoder.mp4");
        let mut writer = MacVideoWriter::create(&path, width, height, options).unwrap();
        writer.write(&picture, 7_000_000_000).unwrap();
        let err = writer.write(&picture, 7_003_000_000).unwrap_err();
        assert!(matches!(err, VideoError::Encoder(_)), "{err}");
        writer.abandon();

        let mut options = software();
        options.disk_fail_at = Some(1_100);
        let path = dir.join("disk.mp4");
        let mut writer = MacVideoWriter::create(&path, width, height, options).unwrap();
        writer.write(&picture, 7_000_000_000).unwrap();
        let err = writer.write(&picture, 7_003_000_000).unwrap_err();
        assert!(matches!(err, VideoError::Disk(_)), "{err}");
        writer.abandon();

        // A full disk leaves a file that reads to its last complete fragment.
        let index = mp4::index_path(&dir.join("disk.mp4")).unwrap();
        assert_eq!(index.complete_len, index.file_len);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_hardware_or_software_encoder_there_is_no_video() {
        let dir = temp_dir("no-encoder");
        let path = dir.join("000001.mp4");
        let options = WriterOptions {
            no_hardware: true,
            ..Default::default()
        };
        let Err(err) = MacVideoWriter::create(&path, 320, 180, options) else {
            panic!("a video without an encoder was created");
        };
        assert!(matches!(err, VideoError::Software), "{err}");
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
