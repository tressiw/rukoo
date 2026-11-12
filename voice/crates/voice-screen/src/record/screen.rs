//! The running screen session: the state machine the main thread runs, its journal lines, the
//! frame and drop rules and the video files, on the recorder's meeting clock.
//!
//! The main thread owns the journal, the clock, the drop state and the capture. Capture frames,
//! the JPEG/OCR worker, the video threads and the control port all talk to it through one
//! queue of `Ev`. It follows the recorder's `timeline.jsonl` every 250 ms for the clock, pause,
//! resume and stop, and stops itself when the recorder has been silent for a minute of awake
//! time with nobody asking its status: the screen is never recorded without the audio.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use voice_screen_files::clock::{Fed, ScreenClock, REANCHOR_SPLIT_MS};
use voice_screen_files::control::Request;
use voice_screen_files::frames::{own_thumb, Decision, Saver, Thumb};
use voice_screen_files::gate::{Gate, VideoGate};
use voice_screen_files::journal::{JournalWriter, Line};
use voice_screen_files::monitors::{self, MonitorInfo};
use voice_screen_files::paths;
use voice_screen_files::policy::{
    DropState, EncoderStep, Part, Reason, DISK_CHECK_MS, ENCODER_RETRY_MS,
};
use voice_screen_files::screenfile;
use voice_screen_files::timeline::{TimelineFollower, TimelineLine};

use crate::args::{Mode, TestSpec};
use crate::finalize;
use crate::frames::worker::{FrameDone, FrameJob, FrameWorker};
use crate::log;
use crate::platform::{Frame, Platform, PlatformError, Source, Thumbs, VideoError, WriterOptions};
use crate::video::thread::{self as video_thread, VideoStart, VideoThread};
use crate::video::FPS;

/// What reaches the main thread. `F` is the platform's own frame type.
pub enum Ev<F> {
    /// A captured frame.
    Frame(crate::platform::Frame<F>),
    /// The worker wrote a JPEG and read it, or failed to.
    FrameDone(FrameDone),
    /// A control port request with a good token.
    Control(Request),
    /// A video file's encoder thread failed.
    VideoError { seq: u64, error: VideoError },
    /// The capture of this generation saw the graphics device go.
    DeviceLost(u64),
    /// The monitor of this generation's capture went away.
    CaptureClosed(u64),
}

const TICK: Duration = Duration::from_millis(250);
pub(crate) const HEARTBEAT: Duration = Duration::from_secs(2);
const LOCK_CHECK: Duration = Duration::from_secs(1);
/// At most five thumbnails a second, for the saver and the video gate together.
const THUMB_GAP: Duration = Duration::from_millis(200);
/// A still screen sends no frames, so the last one is offered again this often, letting the
/// saver see that the screen settled.
const REOFFER_MS: u64 = 250;
/// `Finalize` of a video file may take this long before the file is left as it is.
const FINISH_WAIT: Duration = Duration::from_secs(5);
/// OCR still running at the stop gets this long, then frames keep only their JPEG.
const DRAIN_WAIT: Duration = Duration::from_secs(5);
const DRAIN_EXTRA: Duration = Duration::from_secs(2);
/// After the stop line, the frame thread gets this long to end; a hung OCR call doesn't keep
/// the process alive.
const WORKER_JOIN: Duration = Duration::from_secs(5);
pub(crate) const MEMORY_LOG: Duration = Duration::from_secs(60);
const CAPTURE_RETRY: Duration = Duration::from_secs(1);
const DEVICE_RETRY_PAUSE: Duration = Duration::from_millis(500);
pub(crate) const EVENT_QUEUE: usize = 64;
const MONITOR_CACHE: Duration = Duration::from_secs(2);
/// With `free-bytes-file` the file is read this often, so tests step through the thresholds
/// quickly; the decision is the same.
const TEST_DISK_CHECK: Duration = Duration::from_millis(500);

/// Why capture didn't start.
enum StartFail {
    /// The new graphics device lacks the video support the video needs.
    NoVideo,
    Platform(crate::platform::PlatformError),
}

impl From<PlatformError> for StartFail {
    fn from(e: PlatformError) -> StartFail {
        StartFail::Platform(e)
    }
}

pub(crate) struct Stopping {
    t: u64,
    reason: String,
    replies: Vec<Sender<Value>>,
}

/// The last captured frame, offered again while the screen is still.
pub(crate) struct Last<F> {
    frame: Frame<F>,
    thumb: Option<Thumb>,
    offered_t: u64,
}

pub(crate) struct Screen<P: Platform> {
    pub(crate) platform: P,
    pub(crate) dir: PathBuf,
    pub(crate) mode: Mode,
    pub(crate) test: Option<TestSpec>,
    pub(crate) journal: JournalWriter,
    pub(crate) clock: ScreenClock,
    pub(crate) follower: TimelineFollower,
    pub(crate) drops: DropState,
    pub(crate) events: SyncSender<Ev<P::Picture>>,
    pub(crate) device: Option<P::Device>,
    pub(crate) source: Option<Box<dyn Source>>,
    pub(crate) thumbs: Option<P::Thumbs>,
    pub(crate) saver: Saver,
    pub(crate) worker: FrameWorker,
    /// Names the running capture session; frames of older ones are dropped.
    pub(crate) generation: u64,
    pub(crate) capturing: bool,
    /// The next frame writes `capture`.
    pub(crate) awaiting_capture: bool,
    pub(crate) captured: bool,
    pub(crate) monitor_cache: (Instant, Vec<MonitorInfo>),
    pub(crate) monitor: MonitorInfo,
    /// `--monitor`: the only monitor this process may record (spec 2.7).
    pub(crate) pinned: Option<String>,
    pub(crate) video: Option<VideoThread<P::Video>>,
    /// Which frames of the open video file go to the encoder.
    pub(crate) gate: VideoGate,
    pub(crate) video_seq: u64,
    pub(crate) video_bytes_closed: u64,
    pub(crate) encoder: Option<(String, bool)>,
    /// The encoder failed once: a new device and a new file at this moment.
    pub(crate) rebuild_at: Option<Instant>,
    pub(crate) capture_retry_at: Option<Instant>,
    /// A `no_monitor` line is out and no `capture` has ended its wait yet.
    pub(crate) no_monitor: bool,
    pub(crate) paused: bool,
    /// Whether the timeline's latest `pause` or `resume` line is a `pause`. The timeline is what
    /// the recorder did, so a control `resume` that arrives after a newer pause doesn't resume.
    pub(crate) timeline_paused: bool,
    pub(crate) pause_from: Option<u64>,
    pub(crate) locked: bool,
    pub(crate) frames_saved: u64,
    pub(crate) last_frame_id: Option<String>,
    pub(crate) last: Option<Last<P::Picture>>,
    pub(crate) last_thumb_at: Option<Instant>,
    pub(crate) device_fault_at: Option<u64>,
    /// Device creations still to fail, from the `device-lost-at=<ms>x<n>` test switch.
    pub(crate) create_failures: u32,
    /// Devices still to come without video support, from the `no-video-device=<n>` test switch.
    pub(crate) no_video_devices: u32,
    /// The process had a graphics device with video support: see `needs_video_device`.
    pub(crate) had_video_device: bool,
    pub(crate) recovering: bool,
    pub(crate) stop: Option<Stopping>,
    pub(crate) stop_t: Option<u64>,
    pub(crate) last_activity: u64,
    pub(crate) last_status: u64,
    pub(crate) orphan_100ns: u64,
    pub(crate) next_heartbeat: Instant,
    pub(crate) next_disk_check: Instant,
    pub(crate) next_lock_check: Instant,
    /// A development copy logs the GPU memory it uses once a minute, for the measurements.
    pub(crate) next_memory_log: Instant,
    pub(crate) session: u64,
    pub(crate) mode_changes: u64,
}

impl<P: Platform> Screen<P> {
    pub(crate) fn run(mut self, rx: Receiver<Ev<P::Picture>>) {
        let mut next_tick = Instant::now();
        while self.stop.is_none() {
            let wait = next_tick.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(event) => {
                    self.handle(event);
                    handle_queued(
                        &rx,
                        EVENT_QUEUE,
                        &mut self,
                        |s| s.stop.is_none(),
                        Screen::<P>::handle,
                    );
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if self.stop.is_none() && Instant::now() >= next_tick {
                self.tick();
                next_tick = Instant::now() + TICK;
            }
        }
        self.finish(&rx);
    }

    fn handle(&mut self, event: Ev<P::Picture>) {
        match event {
            Ev::Frame(frame) => self.on_frame(frame),
            Ev::FrameDone(done) => self.on_frame_done(done),
            Ev::Control(request) => self.on_control(request),
            Ev::VideoError { seq, error } => self.on_video_error(seq, error),
            Ev::DeviceLost(generation) => {
                if generation == self.generation && self.capturing {
                    self.on_device_lost();
                }
            }
            Ev::CaptureClosed(generation) => {
                if generation == self.generation && self.capturing {
                    self.on_monitor_lost();
                }
            }
        }
    }

    fn write(&mut self, line: &Line) {
        if let Err(e) = self.journal.write(line) {
            log(format_args!("can't write the screen journal: {e}"));
        }
    }

    /// Meeting time now, never backward.
    fn now_t(&mut self) -> u64 {
        let now = self.platform.clocks().now();
        self.clock.at(now).unwrap_or(0)
    }

    // ---- capture -------------------------------------------------------------------------

    fn make_device(&mut self) -> Result<P::Device, PlatformError> {
        if self.create_failures > 0 {
            self.create_failures -= 1;
            return Err(PlatformError {
                device_lost: true,
                // `DXGI_ERROR_DEVICE_REMOVED`, as a removed device reports it.
                code: "0x887a0005".into(),
                message: "device-lost-at test switch".into(),
            });
        }
        let warp = self.test.as_ref().is_some_and(|t| t.warp);
        if self.no_video_devices > 0 {
            self.no_video_devices -= 1;
            log("no-video-device test switch: a graphics device without video support");
            return self.platform.create_device(false, warp);
        }
        self.platform
            .create_device(self.mode == Mode::Video && self.drops.video.is_none(), warp)
    }

    /// Whether a new graphics device must have video support: the video still records and
    /// this computer gave the process such a device before. `create_device` falls back to a
    /// device without it, which suits frames only, and to WARP, which never has it; after a lost
    /// device that fallback mustn't count as a recovery while the video needs the real one
    /// (spec 2.5).
    fn needs_video_device(&self) -> bool {
        self.mode == Mode::Video && self.drops.video.is_none() && self.had_video_device
    }

    fn try_start(&mut self) -> Result<(), StartFail> {
        if self.device.is_none() {
            let device = self.make_device()?;
            let info = self.platform.device_info(&device);
            let kind = match (info.warp, info.video) {
                (true, _) => "WARP",
                (false, true) => "GPU with video support",
                (false, false) => "GPU",
            };
            if self.needs_video_device() && !info.video {
                log(format_args!(
                    "graphics device: {kind}, without the video support the video needs"
                ));
                return Err(StartFail::NoVideo);
            }
            log(format_args!("graphics device: {kind}"));
            self.had_video_device |= info.video;
            self.device = Some(device);
            self.source = None;
            self.thumbs = None;
        }
        let device = self
            .device
            .clone()
            .ok_or_else(|| PlatformError::new("no graphics device"))?;
        if self.source.is_none() {
            self.source = Some(self.platform.source(
                device,
                self.test.as_ref(),
                self.events.clone(),
            ));
        }
        self.generation += 1;
        let generation = self.generation;
        let monitor = self.monitor.clone();
        if let Some(source) = self.source.as_mut() {
            source.start(&monitor, generation)?;
        }
        self.capturing = true;
        self.awaiting_capture = true;
        self.saver.reset();
        self.last = None;
        Ok(())
    }

    /// Starts capturing unless paused, locked, stopping or with nothing left to record.
    pub(crate) fn start_capture(&mut self) {
        if self.mode == Mode::Off
            || self.capturing
            || self.paused
            || self.locked
            || self.stop.is_some()
            || self.drops.all_dropped()
        {
            return;
        }
        match self.try_start() {
            Ok(()) => {}
            // Only a device without video support came: recover as from a lost device, which
            // tries again and drops the video when the retries are spent.
            Err(StartFail::NoVideo) => self.on_device_lost(),
            Err(StartFail::Platform(e)) => {
                if e.device_lost && self.device.is_some() {
                    self.on_device_lost();
                    return;
                }
                log(format_args!("can't capture {}: {e}", self.monitor.id));
                self.capture_retry_at = Some(Instant::now() + CAPTURE_RETRY);
            }
        }
    }

    fn stop_capture(&mut self) {
        if let Some(source) = self.source.as_mut() {
            source.stop();
        }
        self.capturing = false;
        self.generation += 1;
        self.last = None;
    }

    /// Drops the device and everything on it.
    fn release_device(&mut self) {
        self.stop_capture();
        self.source = None;
        self.thumbs = None;
        self.device = None;
    }

    fn change_mode(&mut self, mode: Mode, monitor: Option<&str>) -> Result<(), &'static str> {
        let next_monitor = match monitor {
            Some(id) => Some(
                self.monitors(true)
                    .into_iter()
                    .find(|m| m.id == id)
                    .ok_or("unknown_monitor")?,
            ),
            None => None,
        };
        let moved = next_monitor
            .as_ref()
            .is_some_and(|m| m.id != self.monitor.id);
        let mode_changed = self.mode != mode;
        if self.mode == mode && !moved {
            return Ok(());
        }
        if mode != Mode::Off && self.drops.frames.is_some() {
            return Err("not_available");
        }
        self.stop_capture();
        self.end_video(if mode_changed { "mode" } else { "monitor" });
        self.release_device();
        // A failed journal write must never restart the old capture mode through a retry.
        self.mode = Mode::Off;
        self.rebuild_at = None;
        self.capture_retry_at = None;
        if let Some(monitor) = next_monitor.filter(|_| moved) {
            self.monitor = monitor;
            let line = self.monitor_line("user");
            self.journal.write(&line).map_err(|_| "journal")?;
        }
        if mode_changed {
            let t = self.now_t();
            self.mode_changes += 1;
            self.journal
                .write(&Line::Mode {
                    t,
                    id: format!("screen-mode-{}-{}", self.session, self.mode_changes),
                    mode: mode.as_str().into(),
                    reason: "user".into(),
                })
                .map_err(|_| "journal")?;
        }
        self.journal.sync();
        self.mode = mode;
        self.drops.set_video(mode == Mode::Video);
        self.rebuild_at = None;
        self.capture_retry_at = None;
        if mode != Mode::Off {
            self.check_disk();
        }
        self.start_capture();
        Ok(())
    }

    pub(crate) fn write_monitor(&mut self, reason: &str) {
        let line = self.monitor_line(reason);
        self.write(&line);
    }

    fn monitor_line(&mut self, reason: &str) -> Line {
        let t = self.now_t();
        let m = self.monitor.clone();
        Line::Monitor {
            t,
            id: m.id,
            name: m.name,
            width: m.width,
            height: m.height,
            primary: m.primary,
            reason: reason.into(),
        }
    }

    /// The monitors this process may record: every one, or with `--monitor` only that one. So
    /// a pinned process never moves to another monitor: not on request, and not when its own
    /// goes away, when it waits for it to come back (spec 2.7).
    fn monitors(&mut self, fresh: bool) -> Vec<MonitorInfo> {
        if fresh || self.monitor_cache.0.elapsed() >= MONITOR_CACHE {
            let list = self.platform.displays().list(self.test.as_ref());
            self.monitor_cache = (Instant::now(), list);
        }
        let mut list = self.monitor_cache.1.clone();
        if let Some(at) = self.test.as_ref().and_then(|t| t.monitor_gone_at_ms) {
            if self.now_t() >= at && !list.is_empty() {
                list.remove(0);
            }
        }
        match &self.pinned {
            Some(id) => list.into_iter().filter(|m| &m.id == id).collect(),
            None => list,
        }
    }

    fn switch_monitor(&mut self, id: &str, reason: &str) -> Result<String, &'static str> {
        if self.stop.is_some() || self.drops.all_dropped() {
            return Err("stopped");
        }
        let list = self.monitors(true);
        let Some(next) = list.iter().find(|m| m.id == id).cloned() else {
            return Err("unknown_monitor");
        };
        if next.id == self.monitor.id && reason == "user" {
            return Ok(next.id);
        }
        self.end_video("monitor");
        self.stop_capture();
        self.monitor = next;
        log(format_args!("moved to {} ({reason})", self.monitor.id));
        self.write_monitor(reason);
        self.start_capture();
        Ok(self.monitor.id.clone())
    }

    fn on_monitor_lost(&mut self) {
        let list = self.monitors(true);
        if list.iter().any(|m| m.id == self.monitor.id) {
            // Still there, as after a mode change: capture it again.
            self.end_video("resize");
            self.stop_capture();
            self.start_capture();
            return;
        }
        let next = match &self.test {
            Some(_) => list.first().map(|m| m.id.clone()),
            None => monitors::choose(
                &self.platform.displays().z_order(),
                self.platform.displays().cursor_monitor().as_deref(),
                &list,
            ),
        };
        match next {
            Some(id) => {
                let _ = self.switch_monitor(&id, "lost");
            }
            // No monitor, or a pinned one that went away: wait until it is back. The journal
            // says so once, so Rukoo can tell the wait from a screen that didn't change.
            None => {
                if self.capturing {
                    log(format_args!("{} is gone; waiting for it", self.monitor.id));
                }
                self.end_video("monitor");
                self.stop_capture();
                if !self.no_monitor {
                    self.no_monitor = true;
                    let t = self.now_t();
                    self.write(&Line::NoMonitor { t });
                }
                self.capture_retry_at = Some(Instant::now() + CAPTURE_RETRY);
            }
        }
    }

    fn on_device_lost(&mut self) {
        if self.recovering {
            return;
        }
        self.recovering = true;
        let t = self.now_t();
        log("the graphics device was lost");
        self.end_video("error");
        self.write(&Line::DeviceLost { t });
        self.release_device();
        // The policy counts this loss and every failed recreation; after `DEVICE_RETRIES`
        // failures it drops the video, then the frames, one step per call (spec 2.6). Each loss
        // gets a count of its own, so a device that came back doesn't bring a later loss closer
        // to a drop: the episode starts from the drops made so far. A device without the video
        // support the video needs counts as a failed recreation for the video only: once the
        // video is dropped, the frames go on with such a device.
        let mut episode = DropState::new(self.mode == Mode::Video);
        episode.video = self.drops.video;
        episode.frames = self.drops.frames;
        let mut step = episode.on_device_lost();
        let mut without_video = false;
        for _ in 0..16 {
            match step {
                EncoderStep::Retry => {
                    if self.stop.is_some() || self.paused || self.locked {
                        // `resume`, `unlock` or the stop take it from here.
                        break;
                    }
                    match self.try_start() {
                        Ok(()) => break,
                        Err(StartFail::NoVideo) => {
                            without_video = true;
                            self.release_device();
                            std::thread::sleep(DEVICE_RETRY_PAUSE);
                            step = episode.on_device_without_video();
                        }
                        Err(StartFail::Platform(e)) => {
                            log(format_args!(
                                "recreating the graphics device failed: {}",
                                e.code
                            ));
                            self.release_device();
                            std::thread::sleep(DEVICE_RETRY_PAUSE);
                            step = episode.on_device_lost();
                        }
                    }
                }
                EncoderStep::Drop(part, reason) => {
                    match part {
                        Part::Video => self.drops.video = Some(reason),
                        Part::Frames => self.drops.frames = Some(reason),
                    }
                    self.apply_drop(part, reason);
                    step = if without_video {
                        episode.on_device_without_video()
                    } else {
                        episode.on_device_lost()
                    };
                }
                EncoderStep::Ignore => break,
            }
        }
        self.recovering = false;
    }

    fn rebuild_device(&mut self) {
        log("starting over with a new graphics device");
        self.release_device();
        self.start_capture();
    }

    // ---- frames --------------------------------------------------------------------------

    fn on_frame(&mut self, frame: Frame<P::Picture>) {
        if frame.generation != self.generation
            || !self.capturing
            || self.paused
            || self.locked
            || self.stop.is_some()
        {
            return;
        }
        let now = self.platform.clocks().at_boot(frame.boot);
        let Some(t) = self.clock.at(now) else {
            return;
        };
        if self.awaiting_capture {
            self.awaiting_capture = false;
            self.captured = true;
            self.no_monitor = false;
            let line = Line::Capture {
                t,
                wall: now.wall,
                monitor: self.monitor.id.clone(),
                width: frame.width,
                height: frame.height,
            };
            self.write(&line);
        }
        if let Some(at) = self.device_fault_at {
            if t >= at {
                self.device_fault_at = None;
                self.create_failures = self
                    .test
                    .as_ref()
                    .map_or(0, |spec| spec.device_create_failures);
                self.no_video_devices = self.test.as_ref().map_or(0, |spec| spec.no_video_devices);
                log("device-lost-at test switch: the graphics device is gone");
                self.on_device_lost();
                return;
            }
        }
        let video = self.open_video(&frame, t);
        let frames = self.drops.frames.is_none();
        // One thumbnail serves the video gate and the saver, at most five a second.
        let wanted = (video && self.gate.wants_thumb(frame.dirty))
            || (frames && self.saver.needs_thumb(frame.dirty));
        let now = Instant::now();
        let thumb = if wanted && self.last_thumb_at.is_none_or(|at| now - at >= THUMB_GAP) {
            self.last_thumb_at = Some(now);
            self.thumb(&frame)
        } else {
            None
        };
        // Reading it lost the graphics device: the capture starts over without this frame.
        if frame.generation != self.generation {
            return;
        }
        if video {
            self.video_frame(&frame, thumb.as_ref());
        }
        if frames {
            self.frames_frame(frame, t, thumb);
        }
    }

    /// `thumb` is this frame's thumbnail, when one was read.
    fn frames_frame(&mut self, frame: Frame<P::Picture>, t: u64, thumb: Option<Thumb>) {
        // A frame with small changes has no thumbnail of its own; a re-offer reads it, so a
        // change never stays measured on an older picture.
        let own = own_thumb(
            thumb.as_ref(),
            frame.dirty,
            self.last.as_ref().and_then(|l| l.thumb.as_ref()),
        );
        self.last = Some(Last {
            frame: frame.clone(),
            thumb: own.clone(),
            offered_t: t,
        });
        if self.saver.offer(t, thumb.as_ref(), frame.dirty) == Decision::Save {
            // Later changes are measured against the saved frame's own thumbnail.
            let own = match own {
                Some(own) => Some(own),
                None => self.thumb(&frame),
            };
            if let Some(own) = own {
                self.save(&frame, t, own);
            }
        }
    }

    fn thumb(&mut self, frame: &Frame<P::Picture>) -> Option<Thumb> {
        let device = self.device.clone()?;
        if self
            .thumbs
            .as_ref()
            .is_none_or(|t| t.size() != (frame.width, frame.height))
        {
            match self.platform.thumbs(&device, frame.width, frame.height) {
                Ok(thumbs) => self.thumbs = Some(thumbs),
                Err(e) => {
                    self.graphics_failed(&e, "thumbnail");
                    return None;
                }
            }
        }
        match self.thumbs.as_mut()?.thumb(&frame.picture) {
            Ok(thumb) => Some(thumb),
            Err(e) => {
                self.graphics_failed(&e, "thumbnail");
                None
            }
        }
    }

    fn graphics_failed(&mut self, e: &PlatformError, what: &str) {
        if e.device_lost
            || self
                .device
                .as_ref()
                .is_some_and(|d| self.platform.device_removed(d))
        {
            self.on_device_lost();
        } else {
            log(format_args!("{what} failed: {}", e.code));
        }
    }

    /// Offers the last frame again while the screen is still, so a change that settled saves.
    /// It goes with its own thumbnail, read once when the frame came without one.
    fn reoffer(&mut self) {
        if !self.capturing || self.paused || self.locked || self.drops.frames.is_some() {
            return;
        }
        let t = self.now_t();
        let Some(last) = self.last.as_ref() else {
            return;
        };
        if t < last.offered_t + REOFFER_MS {
            return;
        }
        let frame = last.frame.clone();
        let thumb = match last.thumb.clone() {
            Some(thumb) => Some(thumb),
            None => self.thumb(&frame),
        };
        let Some(thumb) = thumb else {
            return;
        };
        if let Some(last) = self.last.as_mut() {
            last.thumb = Some(thumb.clone());
            last.offered_t = t;
        }
        if self.saver.offer(t, Some(&thumb), None) == Decision::Save {
            self.save(&frame, t, thumb);
        }
    }

    fn save(&mut self, frame: &Frame<P::Picture>, t: u64, thumb: Thumb) {
        if self.drops.frames.is_some() {
            return;
        }
        let id = paths::frame_id(t);
        if self.last_frame_id.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(device) = self.device.clone() else {
            return;
        };
        // The JPEG thread is behind: the saver offers this change again, and it is saved once
        // there is room, so the frames waiting never pile up in memory.
        if !self.worker.accepts() {
            return;
        }
        match self.platform.pixels(&device, frame) {
            Ok(pixels) => {
                let job = FrameJob {
                    id: id.clone(),
                    t,
                    pixels,
                };
                if self.worker.send(job) {
                    self.saver.saved(t, thumb);
                    self.last_frame_id = Some(id);
                }
            }
            Err(e) => self.graphics_failed(&e, "reading a frame"),
        }
    }

    fn on_frame_done(&mut self, done: FrameDone) {
        match done {
            FrameDone::Saved(line) => {
                let Line::Frame { t, file, .. } = &line else {
                    return;
                };
                let after_pause = self.paused && self.pause_from.is_some_and(|p| *t >= p);
                let after_stop = self.stop_t.is_some_and(|s| *t > s);
                if self.drops.frames.is_some() || after_pause || after_stop {
                    let _ = fs::remove_file(self.dir.join(file));
                    return;
                }
                self.write(&line);
                self.frames_saved += 1;
            }
            FrameDone::Failed {
                id,
                disk_full,
                error,
                ..
            } => {
                log(format_args!("frame {id} wasn't saved: {error}"));
                if disk_full {
                    self.disk_full(Part::Frames);
                }
            }
        }
    }

    // ---- video ---------------------------------------------------------------------------

    fn video_wanted(&self) -> bool {
        self.mode == Mode::Video
            && self.drops.video.is_none()
            && self.rebuild_at.is_none()
            && !self.paused
            && !self.locked
            && self.stop.is_none()
    }

    /// Makes sure a video file at the frame's size is open when the video records; whether one
    /// is.
    fn open_video(&mut self, frame: &Frame<P::Picture>, t: u64) -> bool {
        if !self.video_wanted() {
            return false;
        }
        if let Some(video) = &self.video {
            if (video.width, video.height) != (frame.width, frame.height) {
                self.end_video("resize");
            }
        }
        if self.video.is_none() {
            self.start_video(frame, t);
        }
        self.video.is_some()
    }

    /// Sends the frame to the encoder while the screen moves, and otherwise keeps it as the
    /// newest picture for the next re-submit (`voice_screen_files::gate`).
    fn video_frame(&mut self, frame: &Frame<P::Picture>, thumb: Option<&Thumb>) {
        let gate = self.gate.frame(frame.dirty, thumb);
        if let Some(video) = &self.video {
            match gate {
                Gate::Send => video.send(frame.picture.clone(), frame.boot),
                Gate::Hold => video.hold(frame.picture.clone(), frame.boot),
            }
        }
    }

    fn next_video_seq(&self) -> u64 {
        self.video_seq.max(finalize::highest_video_seq(&self.dir)) + 1
    }

    fn start_video(&mut self, frame: &Frame<P::Picture>, t: u64) {
        let Some(device) = self.device.clone() else {
            return;
        };
        let Some(maker) = self.platform.video_maker(&device) else {
            self.drop_video_encoder("this computer has no video encoder");
            return;
        };
        let seq = self.next_video_seq();
        let file = paths::video_file(seq);
        let test = self.test.clone().unwrap_or_default();
        let options = WriterOptions {
            allow_software: test.software_encoder,
            no_hardware: test.no_hardware,
            start_t: t,
            encoder_fail_at: test.encoder_fail_at_ms,
            disk_fail_at: match test.write_fail {
                Some((Part::Video, at)) => Some(at),
                _ => None,
            },
        };
        let start = VideoStart {
            path: self.dir.join(&file),
            file: file.clone(),
            seq,
            width: frame.width,
            height: frame.height,
            options,
        };
        match video_thread::start(start, maker, self.events.clone()) {
            Ok(video) => {
                self.video_seq = seq;
                log(format_args!(
                    "video {seq}: {} ({}), {}x{}",
                    video.encoder,
                    if video.hardware {
                        "hardware"
                    } else {
                        "software"
                    },
                    video.video_width,
                    video.video_height
                ));
                let line = Line::Video {
                    t,
                    seq,
                    file,
                    width: video.video_width,
                    height: video.video_height,
                    fps: FPS,
                    encoder: video.encoder.clone(),
                    hardware: video.hardware,
                };
                self.write(&line);
                self.encoder = Some((video.encoder.clone(), video.hardware));
                self.video = Some(video);
                self.gate = VideoGate::new();
            }
            Err(VideoError::Software) => self.drop_video_encoder("no hardware H.264 encoder runs"),
            Err(VideoError::Disk(e)) => {
                log(format_args!(
                    "can't start video {seq}: the disk is full ({e})"
                ));
                self.disk_full(Part::Video);
            }
            Err(VideoError::DeviceLost) => self.on_device_lost(),
            Err(VideoError::Encoder(e)) => {
                log(format_args!("can't start video {seq}: {e}"));
                self.encoder_failed();
            }
        }
    }

    /// Ends the open video file and writes its `video_end`. A file that couldn't be finished on
    /// a full disk drops the video, unless the disk is why it ended.
    fn end_video(&mut self, reason: &str) {
        let Some(video) = self.video.take() else {
            return;
        };
        let (seq, start, path) = (video.seq, video.t, video.path.clone());
        // Read after the thread wrote the frames still queued, so `video_end` covers them.
        let (result, end) = video.end(FINISH_WAIT);
        let bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        self.write(&Line::VideoEnd {
            t: end.max(start),
            seq,
            reason: reason.into(),
            bytes,
        });
        self.video_bytes_closed += bytes;
        match result {
            Ok(_) => {}
            Err(VideoError::Disk(e)) if reason != "disk" && self.stop.is_none() => {
                log(format_args!("video {seq} ended on a full disk: {e}"));
                self.disk_full(Part::Video);
            }
            Err(e) => log(format_args!("video {seq} ended badly: {e}")),
        }
    }

    fn on_video_error(&mut self, seq: u64, error: VideoError) {
        if self.video.as_ref().is_none_or(|v| v.seq != seq) {
            return;
        }
        log(format_args!("video {seq}: {error}"));
        match error {
            VideoError::Disk(_) => {
                self.end_video("disk");
                self.disk_full(Part::Video);
            }
            VideoError::Encoder(_) => {
                self.end_video("error");
                self.encoder_failed();
            }
            VideoError::Software => {
                self.end_video("error");
                self.drop_video_encoder("only a software encoder runs");
            }
            VideoError::DeviceLost => self.on_device_lost(),
        }
    }

    fn encoder_failed(&mut self) {
        match self.drops.on_encoder_error() {
            EncoderStep::Retry => {
                self.rebuild_at = Some(Instant::now() + Duration::from_millis(ENCODER_RETRY_MS));
            }
            EncoderStep::Drop(part, reason) => self.apply_drop(part, reason),
            EncoderStep::Ignore => {}
        }
    }

    /// No encoder at all: no hardware one, or Media Foundation missing. Frames go on.
    fn drop_video_encoder(&mut self, why: &str) {
        // No encoder to retry: the policy's second encoder error drops the video.
        for _ in 0..2 {
            match self.drops.on_encoder_error() {
                EncoderStep::Retry => continue,
                EncoderStep::Drop(part, reason) => {
                    log(format_args!("recording frames only: {why}"));
                    self.apply_drop(part, reason);
                }
                EncoderStep::Ignore => {}
            }
            break;
        }
    }

    // ---- drops ---------------------------------------------------------------------------

    fn disk_full(&mut self, part: Part) {
        for (part, reason) in self.drops.on_disk_full(part) {
            self.apply_drop(part, reason);
        }
    }

    pub(crate) fn check_disk(&mut self) {
        let test_file = self.test.as_ref().and_then(|t| t.free_bytes_file.clone());
        let every = match test_file {
            Some(_) => TEST_DISK_CHECK,
            None => Duration::from_millis(DISK_CHECK_MS),
        };
        self.next_disk_check = Instant::now() + every;
        let disk = self.platform.disk();
        let Some(free) = disk.free_bytes(&self.dir, test_file.as_deref()) else {
            return;
        };
        for (part, reason) in self.drops.on_free_bytes(free) {
            self.apply_drop(part, reason);
        }
    }

    /// Journals a drop the policy decided: video first, its file ended before the line.
    fn apply_drop(&mut self, part: Part, reason: Reason) {
        match part {
            Part::Video => self.end_video(if reason == Reason::Disk {
                "disk"
            } else {
                "error"
            }),
            Part::Frames => self.worker.clear(),
        }
        let t = self.now_t();
        log(format_args!(
            "dropped {}: {}",
            part.as_str(),
            reason.as_str()
        ));
        self.write(&Line::Dropped {
            t,
            what: part.as_str().into(),
            reason: reason.as_str().into(),
        });
        if self.drops.all_dropped() {
            self.request_stop(t, "dropped", None);
        }
    }

    // ---- the recorder's timeline, pause, lock -------------------------------------------

    fn follow_timeline(&mut self) {
        let lines = match self.follower.poll() {
            Ok(lines) => lines,
            Err(e) => {
                log(format_args!("can't read the timeline: {e}"));
                return;
            }
        };
        if !lines.is_empty() {
            self.last_activity = self.platform.clocks().awake_100ns();
        }
        for line in lines {
            match &line {
                TimelineLine::Session { .. } | TimelineLine::Heartbeat { .. } => {
                    let now = self.platform.clocks().now();
                    if let Fed::Reanchored { jump_ms } = self.clock.feed(&line, now) {
                        if jump_ms.abs() > REANCHOR_SPLIT_MS {
                            log(format_args!("the recorder's clock moved {jump_ms} ms"));
                            self.end_video("clock");
                        }
                    }
                }
                TimelineLine::Pause { t } => {
                    self.timeline_paused = true;
                    self.pause(Some(*t));
                }
                TimelineLine::Resume { .. } => {
                    self.timeline_paused = false;
                    self.resume();
                }
                TimelineLine::Suspend { .. } => self.saver.reset(),
                TimelineLine::Stop { t, .. } => self.request_stop(*t, "meeting", None),
                _ => {}
            }
        }
    }

    fn pause(&mut self, t: Option<u64>) {
        let t = match t {
            Some(t) => t,
            None => self.now_t(),
        };
        self.pause_from = Some(self.pause_from.map_or(t, |p| p.min(t)));
        self.worker.cancel_from(t);
        if self.paused {
            return;
        }
        self.paused = true;
        self.end_video("pause");
        self.stop_capture();
    }

    fn resume(&mut self) {
        if !self.paused {
            return;
        }
        self.paused = false;
        self.pause_from = None;
        self.start_capture();
    }

    fn check_lock(&mut self) {
        self.next_lock_check = Instant::now() + LOCK_CHECK;
        let locked = self.platform.locked();
        if locked == self.locked {
            return;
        }
        let t = self.now_t();
        if locked {
            log("the session locked");
            self.end_video("lock");
            self.stop_capture();
            self.locked = true;
            self.write(&Line::Lock { t });
        } else {
            log("the session unlocked");
            self.locked = false;
            self.write(&Line::Unlock { t });
            // A capture bound across a lock can stay blank; a new session doesn't.
            self.source = None;
            self.start_capture();
        }
    }

    // ---- the loop ------------------------------------------------------------------------

    fn tick(&mut self) {
        self.follow_timeline();
        if self.stop.is_some() {
            return;
        }
        let now = Instant::now();
        if now >= self.next_heartbeat {
            self.next_heartbeat = now + HEARTBEAT;
            self.heartbeat();
        }
        if self.mode != Mode::Off && now >= self.next_disk_check {
            self.check_disk();
        }
        if self.test.is_none() && now >= self.next_lock_check {
            self.check_lock();
        }
        if self.rebuild_at.is_some_and(|at| now >= at) {
            self.rebuild_at = None;
            self.rebuild_device();
        }
        if self.capture_retry_at.is_some_and(|at| now >= at) {
            self.capture_retry_at = None;
            if self.monitors(true).iter().any(|m| m.id == self.monitor.id) {
                self.start_capture();
            } else {
                // The monitor went away while capture was down: choose again.
                self.on_monitor_lost();
            }
        }
        if self.capturing {
            if let Some(device) = self.device.clone() {
                if self.platform.device_removed(&device) {
                    self.on_device_lost();
                }
            }
        }
        // The capture's `Closed` notice shares the queue with frames and is lost when the queue
        // is full, so a recorded monitor that left the list counts as gone as well.
        if self.capturing && !self.monitors(false).iter().any(|m| m.id == self.monitor.id) {
            log(format_args!("{} left the monitor list", self.monitor.id));
            self.on_monitor_lost();
        }
        if let Some(video) = &self.video {
            video.tick(self.platform.clocks().boot_100ns());
        }
        self.reoffer();
        if now >= self.next_memory_log {
            self.next_memory_log = now + MEMORY_LOG;
            if crate::dev_mode() {
                let device = self.device.clone();
                if let Some(bytes) = device.and_then(|d| self.platform.device_memory(&d)) {
                    let dropped = self.video.as_ref().map_or(0, VideoThread::dropped);
                    log(format_args!(
                        "gpu memory in use: {} MB; frames saved: {}; video frames the encoder dropped: {dropped}",
                        bytes >> 20,
                        self.frames_saved
                    ));
                }
            }
        }
        let awake = self.platform.clocks().awake_100ns();
        if awake.saturating_sub(self.last_activity) >= self.orphan_100ns
            && awake.saturating_sub(self.last_status) >= self.orphan_100ns
        {
            log("the recorder has been silent too long; stopping");
            let t = self.now_t();
            self.request_stop(t, "interrupted", None);
        }
    }

    fn heartbeat(&mut self) {
        let now = self.platform.clocks().now();
        let t = self.clock.at(now).unwrap_or(0);
        let open = self
            .video
            .as_ref()
            .and_then(|v| fs::metadata(&v.path).ok())
            .map_or(0, |m| m.len());
        let line = Line::Heartbeat {
            t,
            wall: now.wall,
            boot: now.boot,
            frames: self.frames_saved,
            video_bytes: self.video_bytes_closed + open,
        };
        self.write(&line);
        self.journal.sync();
    }

    pub(crate) fn request_stop(&mut self, t: u64, reason: &str, reply: Option<Sender<Value>>) {
        match &mut self.stop {
            Some(stopping) => stopping.replies.extend(reply),
            None => {
                self.stop = Some(Stopping {
                    t,
                    reason: reason.into(),
                    replies: reply.into_iter().collect(),
                })
            }
        }
    }

    /// Stops capturing, ends the video, lets OCR finish, writes `stop`, removes `screen.json`
    /// and only then answers a `stop` request.
    fn finish(&mut self, rx: &Receiver<Ev<P::Picture>>) {
        let stopping = self.stop.take().unwrap_or(Stopping {
            t: 0,
            reason: "interrupted".into(),
            replies: Vec::new(),
        });
        let mut replies = stopping.replies;
        self.stop_t = Some(stopping.t);
        // Stopping: new frames and requests are refused from here on.
        self.stop = Some(Stopping {
            t: stopping.t,
            reason: stopping.reason.clone(),
            replies: Vec::new(),
        });
        self.release_device();
        self.end_video("stop");
        if self.drops.frames.is_none() {
            let started = Instant::now();
            while self.worker.pending() > 0 {
                let waited = started.elapsed();
                if waited >= DRAIN_WAIT {
                    // The frames OCR still holds come back with their JPEG, without text.
                    for done in self.worker.skip_ocr() {
                        self.on_frame_done(done);
                    }
                }
                if waited >= DRAIN_WAIT + DRAIN_EXTRA {
                    log("frames still waiting at the stop are left out");
                    break;
                }
                if let Ok(event) = rx.recv_timeout(Duration::from_millis(50)) {
                    self.handle_stopping(event, &mut replies);
                }
            }
            for event in rx.try_iter().collect::<Vec<_>>() {
                self.handle_stopping(event, &mut replies);
            }
        } else {
            self.worker.clear();
        }
        self.write(&Line::Stop {
            t: stopping.t,
            reason: stopping.reason.clone(),
        });
        self.journal.sync();
        screenfile::remove(&self.dir);
        log(format_args!(
            "session {}: stopped at {} ({}), {} frames",
            self.session, stopping.t, stopping.reason, self.frames_saved
        ));
        for reply in replies {
            let _ = reply.send(json!({ "ok": true, "t": stopping.t }));
        }
        // Frames still queued are left out. A JPEG the worker finishes after the stop line has
        // no line to name it, and `finalize` doesn't look at a stopped journal: remove it.
        self.worker.clear();
        self.worker.join(WORKER_JOIN);
        for event in rx.try_iter() {
            if let Ev::FrameDone(FrameDone::Saved(Line::Frame { file, .. })) = event {
                let _ = fs::remove_file(self.dir.join(file));
            }
        }
    }

    fn handle_stopping(&mut self, event: Ev<P::Picture>, replies: &mut Vec<Sender<Value>>) {
        match event {
            Ev::FrameDone(done) => self.on_frame_done(done),
            Ev::Control(request) => match request.body["cmd"].as_str() {
                Some("stop") => replies.push(request.reply),
                Some("status") => {
                    let status = self.status();
                    let _ = request.reply.send(status);
                }
                _ => {
                    let _ = request.reply.send(error("stopped"));
                }
            },
            _ => {}
        }
    }

    // ---- control port --------------------------------------------------------------------

    fn on_control(&mut self, request: Request) {
        let reply = match request.body["cmd"].as_str() {
            Some("status") => {
                self.last_status = self.platform.clocks().awake_100ns();
                self.status()
            }
            Some("monitors") => {
                let list = monitors::ordered(&self.monitors(true));
                json!({ "ok": true, "monitors": list })
            }
            Some("mode") => match request.body["mode"].as_str().and_then(Mode::parse) {
                Some(mode) => match self.change_mode(mode, request.body["monitor"].as_str()) {
                    Ok(()) => json!({ "ok": true, "mode": mode.as_str(), "t": self.now_t() }),
                    Err(code) => error(code),
                },
                None => error("bad_request"),
            },
            Some("monitor") => match request.body["id"].as_str() {
                Some(id) => match self.switch_monitor(id, "user") {
                    Ok(id) => json!({ "ok": true, "monitor": id }),
                    Err(code) => error(code),
                },
                None => error("bad_request"),
            },
            // The fast path for a pause or resume, which mustn't wait for the next timeline poll.
            // Each reads the timeline now and counts only when the timeline agrees. The supervisor
            // sends them on separate connections, so they can arrive out of order: a late resume
            // mustn't record through a newer pause, and a late pause mustn't stop the recording
            // after a newer resume. The recorder writes its line before it answers, so a genuine
            // request finds the line there.
            Some("pause") => {
                self.follow_timeline();
                if self.timeline_paused {
                    self.pause(None);
                }
                json!({ "ok": true })
            }
            Some("resume") => {
                self.follow_timeline();
                if !self.timeline_paused {
                    self.resume();
                }
                json!({ "ok": true })
            }
            Some("stop") => {
                let t = self.now_t();
                self.request_stop(t, "meeting", Some(request.reply));
                return;
            }
            Some(_) => error("unknown_command"),
            None => error("bad_request"),
        };
        let _ = request.reply.send(reply);
    }

    /// The `status` reply as `voice/RECORDING.md` "The screen control port" describes it.
    fn status(&mut self) -> Value {
        let t = self.now_t();
        let state = if self.stop.is_some() {
            "stopped"
        } else if self.mode == Mode::Off {
            "off"
        } else if self.paused {
            "paused"
        } else if self.locked {
            "locked"
        } else if !self.captured {
            "starting"
        } else {
            "recording"
        };
        let monitors = monitors::ordered(&self.monitors(false));
        json!({
            "ok": true,
            "state": state,
            "t": t,
            "mode": self.mode.as_str(),
            "video": self.mode == Mode::Video && self.drops.video.is_none(),
            "frames": self.mode != Mode::Off && self.drops.frames.is_none(),
            "monitor": self.monitor.id,
            "monitors": monitors,
            "dropped": {
                "video": self.drops.video.map(Reason::as_str),
                "frames": self.drops.frames.map(Reason::as_str),
            },
            "encoder": self.encoder.as_ref().map(|(name, _)| name.as_str()),
            "hardware": self.encoder.as_ref().map(|(_, hardware)| *hardware),
            "saved": self.frames_saved,
        })
    }
}

fn error(code: &str) -> Value {
    json!({ "ok": false, "error": code })
}

/// Handles up to `max` events still queued. `running` is checked before each one leaves the
/// queue, so an event queued behind a stop stays there for `finish` to drain: one taken after
/// the stop would be lost, a frame without its `frame` line or a request without an answer.
fn handle_queued<S, T>(
    rx: &Receiver<T>,
    max: usize,
    state: &mut S,
    running: impl Fn(&S) -> bool,
    mut handle: impl FnMut(&mut S, T),
) {
    for _ in 0..max {
        if !running(state) {
            return;
        }
        let Ok(event) = rx.try_recv() else {
            return;
        };
        handle(state, event);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn events_behind_a_stop_stay_queued() {
        let (tx, rx) = mpsc::channel();
        for event in 0..4 {
            tx.send(event).unwrap();
        }
        // Event 1 stops the loop: 2 and 3 are left for the drain.
        let mut handled: Vec<u32> = Vec::new();
        handle_queued(
            &rx,
            EVENT_QUEUE,
            &mut handled,
            |h| !h.contains(&1),
            |h, e| h.push(e),
        );
        assert_eq!(handled, [0, 1]);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [2, 3]);

        // Stopped already: nothing leaves the queue.
        tx.send(4).unwrap();
        handle_queued(&rx, EVENT_QUEUE, &mut handled, |_| false, |h, e| h.push(e));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [4]);

        // At most `max` a round, so the tick still runs under a flood of frames.
        for event in 0..10 {
            tx.send(event).unwrap();
        }
        let mut count = 0;
        handle_queued(&rx, 3, &mut count, |_| true, |c, _| *c += 1);
        assert_eq!(count, 3);
        assert_eq!(rx.try_iter().count(), 7);
    }
}
