//! The real screen on macOS: ScreenCaptureKit's `SCStream` on one display.
//!
//! The stream is set up once per capture session, on a thread of its own, from one read of
//! `SCShareableContent`: the display with the id the monitor list chose, and every running
//! application that stays out of the recording (Voice's own app, Rukoo, and the pids the command
//! line names). Its filter is `initWithDisplay:excludingApplications:exceptingWindows:`, so those
//! applications' windows are not in the frames and the desktop behind them is. The stream's size
//! is the filter's content in pixels. The main thread, which answers the control port, never
//! waits for a stream to start.
//!
//! Frames arrive on a dispatch queue as `CMSampleBuffer`s backed by an IOSurface in `420f`
//! (bi-planar full-range YCbCr), which is the format VideoToolbox encodes without a conversion.
//! The picture travels as it is; the thumbnail reads its luma plane, the JPEG and OCR paths
//! convert it to BGRA when they need pixels, and the encoder takes the buffer itself. Idle
//! frames, which ScreenCaptureKit sends when nothing changed, are not frames: the orchestration
//! treats the screen as still from its own timer.
//!
//! A stream that stops by itself, as it does when the display goes or the user revokes Screen
//! Recording, sends `Ev::CaptureStopped` with the stream's own reason; the orchestration decides
//! from the permission whether to record again or to end the screen part.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFString, CFType, CGRect};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, kCGDisplayStreamYCbCrMatrix_ITU_R_709_2,
    CGRectMakeWithDictionaryRepresentation,
};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_core_video::kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;
use objc2_foundation::{NSArray, NSError, NSString, NSValue};
use objc2_screen_capture_kit::{
    SCContentFilter, SCFrameStatus, SCRunningApplication, SCShareableContent, SCStream,
    SCStreamConfiguration, SCStreamDelegate, SCStreamErrorCode, SCStreamFrameInfoDirtyRects,
    SCStreamFrameInfoStatus, SCStreamOutput, SCStreamOutputType, SCWindow,
};
use voice_screen_files::monitors::MonitorInfo;

use super::mac::PixelBuffer;
#[cfg(test)]
use super::monitors_mac;
use crate::clock_mac;
use crate::platform::{Frame, PlatformError, Source};
use crate::record::{CaptureStop, Ev};
use crate::video::size::video_size;
use crate::video::FPS;

/// The apps whose windows stay out of the recording, by bundle id: Rukoo, and Voice itself, whose
/// pill, Settings and dialogs would otherwise be in the meeting's frames and video. A development
/// Rukoo runs from Electron, which only its pid tells apart, so `--exclude-pid` covers that.
pub const EXCLUDED_BUNDLES: [&str; 3] = [
    "nl.bvdm.rukoo-mail",
    "nl.bvdm.rukoo.voice",
    "nl.bvdm.rukoo.voice.dev",
];

/// How long the shareable content and a starting stream may take.
const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// How long a start that gave no answer within `ANSWER_WAIT` is still waited for, off the main
/// thread, so a stream that starts late is stopped instead of capturing with nobody holding it.
const LATE_START_WAIT: Duration = Duration::from_secs(60);
/// How long a stopping stream may take before its frames are dropped by their generation anyway.
const STOP_WAIT: Duration = Duration::from_secs(2);
/// A running stream that hasn't called its output for this long, idle samples included, counts
/// as stalled and is started again. The header promises nothing about idle samples: a real
/// capture at the Mac has to confirm how often they arrive on a still screen and across display
/// sleep. If a still screen sends none at all, this only restarts the stream inside the source
/// every 10 s, without a journal line; a restarted stream that delivers nothing either is
/// reported as stopped.
const STALL_AFTER: Duration = Duration::from_secs(10);
/// How often the running stream's exclusions are read again: an app that starts, restarts or
/// connects after the stream began must stay out of the recording too.
const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// The real screen. `start` hands the work to a thread of its own, so the main thread, which
/// answers the control port, never waits on ScreenCaptureKit to start a stream: the shareable
/// content, the filter and the stream's start all happen there, from one read of the shareable
/// content. A start that fails reaches the main thread as `Ev::CaptureStopped`, as a stream that
/// stops by itself does.
pub struct ScreenSource {
    to_main: SyncSender<Ev<PixelBuffer>>,
    /// The pids from `--exclude-pid`.
    exclude_pids: Vec<u32>,
    shared: Arc<Mutex<Shared>>,
    /// The display and size of the last start, for starting a stalled stream again.
    last_start: Option<(u32, (u32, u32))>,
    /// When the exclusions are read again next, on the awake clock.
    next_refresh: u64,
}

/// What the main thread and a starting thread share.
#[derive(Default)]
struct Shared {
    /// The generation whose capture is wanted; 0 once it was stopped. A start that finishes for
    /// another generation stops its stream again.
    wanted: u64,
    live: Option<Live>,
    /// A start that failed, kept for `health` too: its event can be lost in a full queue. A
    /// re-read of the exclusions that ScreenCaptureKit refused for the permission puts its stop
    /// here as well.
    failed: Option<(u64, CaptureStop)>,
    /// A re-read of the exclusions runs.
    refreshing: bool,
    /// The last reason a re-read failed, so the log says it once.
    refresh_error: Option<String>,
}

/// A running stream and what it needs to keep running.
struct Live {
    generation: u64,
    /// When the stream started, on the awake clock.
    started: u64,
    /// The stream was started again after the one before it stalled.
    restarted: bool,
    display: u32,
    /// What the stream's filter leaves out.
    applied: Matched,
    stream: Retained<SCStream>,
    /// The stream's output and delegate. `Live` holds it until the stream stopped: whether a
    /// stream retains its delegate isn't in the header.
    output: Retained<Output>,
    /// The queue the output is called on; it must outlive the stream's output.
    _queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: ScreenCaptureKit's objects may be called from any thread. A `Live` is made on a
// starting thread and from then on used under the `Shared` lock or by the one thread that took it
// out to stop it. Frames and the stop notice reach the main thread through the channel.
unsafe impl Send for Live {}

impl ScreenSource {
    pub fn new(to_main: SyncSender<Ev<PixelBuffer>>, exclude_pids: Vec<u32>) -> ScreenSource {
        ScreenSource {
            to_main,
            exclude_pids,
            shared: Arc::new(Mutex::new(Shared::default())),
            last_start: None,
            next_refresh: 0,
        }
    }

    /// Reads the shareable content again on a thread of its own, at most every `REFRESH_EVERY`
    /// and one read at a time, and gives the running stream a new filter when what it must leave
    /// out changed.
    fn refresh(&mut self) {
        let now = clock_mac::awake_100ns();
        if now < self.next_refresh {
            return;
        }
        self.next_refresh = now + REFRESH_EVERY.as_nanos() as u64 / 100;
        let generation = {
            let mut shared = lock(&self.shared);
            let Some(live) = shared.live.as_ref() else {
                return;
            };
            if shared.refreshing {
                return;
            }
            let generation = live.generation;
            shared.refreshing = true;
            generation
        };
        let job = RefreshJob {
            generation,
            pids: ScreenSource::excluded_pids(&self.exclude_pids),
            shared: self.shared.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("screen-filter".into())
            .spawn(move || job.run());
        if spawned.is_err() {
            lock(&self.shared).refreshing = false;
        }
    }

    /// Starts `generation`'s stream on a thread of its own.
    fn spawn(
        &mut self,
        display: u32,
        fallback: (u32, u32),
        generation: u64,
        restarted: bool,
    ) -> Result<(), PlatformError> {
        self.last_start = Some((display, fallback));
        let job = StartJob {
            display,
            fallback,
            pids: ScreenSource::excluded_pids(&self.exclude_pids),
            generation,
            restarted,
            to_main: self.to_main.clone(),
            shared: self.shared.clone(),
        };
        std::thread::Builder::new()
            .name("screen-start".into())
            .spawn(move || job.run())
            .map(|_| ())
            .map_err(|e| PlatformError::new(format!("can't start the capture's thread: {e}")))
    }

    /// The pids whose windows stay out of the recording: this process, its parent (the Voice app
    /// that started it), and the ones the command line names.
    pub fn excluded_pids(from_args: &[u32]) -> Vec<u32> {
        // SAFETY: the call takes no arguments and only reads this process's parent.
        let parent = unsafe { libc::getppid() };
        let mut pids = vec![std::process::id()];
        if parent > 1 {
            pids.push(parent as u32);
        }
        pids.extend(from_args.iter().copied().filter(|&pid| pid != 0));
        pids.sort_unstable();
        pids.dedup();
        pids
    }
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

impl Source for ScreenSource {
    fn start(&mut self, monitor: &MonitorInfo, generation: u64) -> Result<(), PlatformError> {
        self.stop();
        let display: u32 = monitor.id.parse().map_err(|_| {
            PlatformError::new(format!(
                "display {} isn't a display this Mac has",
                monitor.id
            ))
        })?;
        lock(&self.shared).wanted = generation;
        self.spawn(display, (monitor.width, monitor.height), generation, false)
    }

    fn stop(&mut self) {
        let live = {
            let mut shared = lock(&self.shared);
            shared.wanted = 0;
            shared.failed = None;
            shared.live.take()
        };
        if let Some(live) = live {
            close(live);
        }
    }

    fn exclude(&mut self, pids: &[u32]) {
        self.exclude_pids = pids.to_vec();
        // The next tick reads the exclusions again.
        self.next_refresh = 0;
    }

    fn health(&mut self) -> Option<CaptureStop> {
        self.refresh();
        let mut shared = lock(&self.shared);
        let wanted = shared.wanted;
        if let Some((generation, stop)) = shared.failed.take() {
            if generation == wanted {
                return Some(stop);
            }
        }
        let live = shared.live.as_ref()?;
        if let Some(stop) = live.output.take_stop() {
            return Some(stop);
        }
        let seen = live.output.last_callback();
        if !stalled(seen.max(live.started), clock_mac::awake_100ns()) {
            return None;
        }
        let live = shared.live.take()?;
        drop(shared);
        let (generation, restarted) = (live.generation, live.restarted);
        close(live);
        if restarted && seen == 0 {
            return Some(CaptureStop {
                why: format!(
                    "the stream was started again after {} s without a sample and sent none either",
                    STALL_AFTER.as_secs()
                ),
                permission_lost: false,
                by_user: false,
            });
        }
        crate::log(format_args!(
            "the stream sent nothing for {} s; starting it again",
            STALL_AFTER.as_secs()
        ));
        let (display, fallback) = self.last_start?;
        match self.spawn(display, fallback, generation, true) {
            Ok(()) => None,
            Err(e) => Some(CaptureStop {
                why: e.to_string(),
                permission_lost: false,
                by_user: false,
            }),
        }
    }
}

/// One re-read of the exclusions of a running stream.
struct RefreshJob {
    generation: u64,
    pids: Vec<u32>,
    shared: Arc<Mutex<Shared>>,
}

impl RefreshJob {
    fn run(self) {
        self.refresh();
        lock(&self.shared).refreshing = false;
    }

    fn refresh(&self) {
        let content = match shareable() {
            Ok(content) => content,
            Err(e) => {
                let mut shared = lock(&self.shared);
                if e.permission && shared.wanted == self.generation {
                    // The grant is gone: a running stream doesn't stop by itself when it is
                    // revoked, so this ends it.
                    shared.failed = Some((
                        self.generation,
                        CaptureStop {
                            why: e.to_string(),
                            permission_lost: true,
                            by_user: false,
                        },
                    ));
                } else if shared.refresh_error.as_deref() != Some(e.message.as_str()) {
                    crate::log(format_args!(
                        "can't read the screen's applications again: {e}"
                    ));
                    shared.refresh_error = Some(e.message.clone());
                }
                return;
            }
        };
        let exclusions = Exclusions::of(&content, &self.pids);
        let (stream, display) = {
            let mut shared = lock(&self.shared);
            shared.refresh_error = None;
            let Some(live) = shared
                .live
                .as_ref()
                .filter(|live| live.generation == self.generation)
            else {
                return;
            };
            if live.applied == exclusions.matched {
                return;
            }
            (live.stream.clone(), live.display)
        };
        let Some(display) = unsafe { content.displays() }
            .to_vec()
            .into_iter()
            .find(|d| unsafe { d.displayID() } == display)
        else {
            // The display went; the monitor rules take it from here.
            return;
        };
        // SAFETY: as at the start: the objects live until the filter retained them.
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::from_retained_slice(&exclusions.applications),
                &NSArray::<SCWindow>::from_slice(&[]),
            )
        };
        let updated = wait_for("the capture's filter wasn't updated", ANSWER_WAIT, |done| {
            // SAFETY: the handler is called once the filter is in place or refused.
            unsafe { stream.updateContentFilter_completionHandler(&filter, Some(done)) };
        });
        match updated {
            Ok(()) => {
                crate::log(format_args!("the capture now {}", exclusions.describe()));
                let mut shared = lock(&self.shared);
                if let Some(live) = shared
                    .live
                    .as_mut()
                    .filter(|live| live.generation == self.generation)
                {
                    live.applied = exclusions.matched;
                }
            }
            Err(e) => {
                let mut shared = lock(&self.shared);
                if e.permission && shared.wanted == self.generation {
                    shared.failed = Some((
                        self.generation,
                        CaptureStop {
                            why: e.to_string(),
                            permission_lost: true,
                            by_user: false,
                        },
                    ));
                } else {
                    crate::log(format_args!("{e}"));
                }
            }
        }
    }
}

/// Whether a stream whose last callback, or start, was at `last` on the awake clock has gone
/// quiet for longer than `STALL_AFTER` at `now`.
fn stalled(last: u64, now: u64) -> bool {
    now.saturating_sub(last) > STALL_AFTER.as_nanos() as u64 / 100
}

impl Drop for ScreenSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One start of the stream, on a thread of its own.
struct StartJob {
    display: u32,
    /// The monitor's pixels, for a filter that doesn't say its own size.
    fallback: (u32, u32),
    pids: Vec<u32>,
    generation: u64,
    /// A stream started again after the one before it stalled.
    restarted: bool,
    to_main: SyncSender<Ev<PixelBuffer>>,
    shared: Arc<Mutex<Shared>>,
}

impl StartJob {
    fn run(self) {
        match self.open() {
            Opened::Running(live) => {
                let mut shared = lock(&self.shared);
                if shared.wanted == self.generation {
                    shared.live = Some(live);
                } else {
                    // Stopped, or started again, while this start ran.
                    drop(shared);
                    close(live);
                }
            }
            Opened::Failed(stop) => self.report(stop),
            Opened::Late(stop, pending) => {
                self.report(stop);
                pending.clean_up();
            }
        }
    }

    /// A start that failed, for the main thread to decide on. A start nobody wants any more
    /// says nothing.
    fn report(&self, stop: CaptureStop) {
        let mut shared = lock(&self.shared);
        if shared.wanted != self.generation {
            return;
        }
        shared.failed = Some((self.generation, stop.clone()));
        drop(shared);
        // `health` hands it over too when the queue is full; the main thread takes it once.
        let _ = self
            .to_main
            .try_send(Ev::CaptureStopped(self.generation, stop));
    }

    fn open(&self) -> Opened {
        let failed = |e: PlatformError| {
            Opened::Failed(CaptureStop {
                why: e.to_string(),
                permission_lost: e.permission,
                by_user: false,
            })
        };
        // One read of the shareable content serves the display and the filter.
        let content = match shareable() {
            Ok(content) => content,
            Err(e) => return failed(e),
        };
        let Some(display) = unsafe { content.displays() }
            .to_vec()
            .into_iter()
            .find(|display| unsafe { display.displayID() } == self.display)
        else {
            return failed(PlatformError::new(format!(
                "display {} isn't there",
                self.display
            )));
        };
        let exclusions = Exclusions::of(&content, &self.pids);
        crate::log(format_args!("the capture {}", exclusions.describe()));
        // SAFETY: every object lives until the filter has retained what it needs, and the empty
        // windows array is an exception list of none.
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::from_retained_slice(&exclusions.applications),
                &NSArray::<SCWindow>::from_slice(&[]),
            )
        };
        // The display's pixels as the filter sees them, at its current mode; the monitor list's
        // only when the filter doesn't say.
        let (width, height) = filter_pixels(&filter).unwrap_or(self.fallback);
        let (width, height) = stream_size(width, height);
        let configuration = unsafe { SCStreamConfiguration::new() };
        // SAFETY: the configuration is a fresh object, and the pixel format is one
        // ScreenCaptureKit documents for it.
        unsafe {
            configuration.setWidth(width as usize);
            configuration.setHeight(height as usize);
            configuration.setPixelFormat(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange);
            configuration.setMinimumFrameInterval(CMTime::new(1, FPS as i32));
            configuration.setShowsCursor(false);
            configuration.setCapturesAudio(false);
            // The matrix the stream encodes with, so the JPEG and OCR paths convert the way the
            // encoder assumes.
            configuration.setColorMatrix(kCGDisplayStreamYCbCrMatrix_ITU_R_709_2);
            // sRGB, not the display's own space (P3 on a MacBook), which the stream would use
            // otherwise: the JPEG says sRGB and the video's BT.709 has sRGB's primaries, so the
            // frames, the OCR input and the video agree.
            configuration.setColorSpaceName(kCGColorSpaceSRGB);
        }
        let output = Output::new(self.to_main.clone(), self.generation, width, height);
        let queue = DispatchQueue::new("nl.bvdm.rukoo.voice.screen", None);
        // SAFETY: the delegate and the output outlive the stream, which `Live` holds.
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &configuration,
                Some(ProtocolObject::from_ref(&*output)),
            )
        };
        if let Err(error) = unsafe {
            stream.addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Screen,
                Some(&queue),
            )
        } {
            return failed(sc_error("the screen output wasn't taken", &error));
        }
        let (sender, answer) = mpsc::channel();
        let block: RcBlock<dyn Fn(*mut NSError)> = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: the error is a live `NSError` for the length of the call, or null.
            let error = unsafe { Retained::retain(error) }
                .map(|error| (describe(&error), permission_lost(error.code())));
            let _ = sender.send(error);
        });
        // SAFETY: the handler is called once the stream started or gave up.
        unsafe { stream.startCaptureWithCompletionHandler(Some(&block)) };
        let live = Live {
            generation: self.generation,
            started: clock_mac::awake_100ns(),
            restarted: self.restarted,
            display: self.display,
            applied: exclusions.matched.clone(),
            stream,
            output,
            _queue: queue,
        };
        match answer.recv_timeout(ANSWER_WAIT) {
            Ok(None) => {
                crate::log(format_args!(
                    "capturing display {} at {width}x{height}",
                    self.display
                ));
                Opened::Running(live)
            }
            Ok(Some((why, permission))) => {
                remove_output(&live);
                Opened::Failed(CaptureStop {
                    why: format!("the screen capture didn't start: {why}"),
                    permission_lost: permission,
                    by_user: false,
                })
            }
            Err(_) => Opened::Late(
                CaptureStop {
                    why: format!(
                        "the screen capture didn't start: no answer within {} s",
                        ANSWER_WAIT.as_secs()
                    ),
                    permission_lost: false,
                    by_user: false,
                },
                Pending { live, answer },
            ),
        }
    }
}

/// How a start ended.
enum Opened {
    Running(Live),
    Failed(CaptureStop),
    /// No answer in time; the stream may still start.
    Late(CaptureStop, Pending),
}

/// A start that gave no answer in time.
struct Pending {
    live: Live,
    answer: Receiver<Option<(String, bool)>>,
}

impl Pending {
    /// Takes the output off, waits a while longer for the start's answer and stops the stream
    /// if it did start, so no capture runs that nobody holds.
    fn clean_up(self) {
        remove_output(&self.live);
        match self.answer.recv_timeout(LATE_START_WAIT) {
            Ok(None) => {
                crate::log("a late capture start answered; stopping it");
                close(self.live);
            }
            Ok(Some(_)) => {}
            Err(_) => crate::log(format_args!(
                "a capture start never answered within {} s",
                (ANSWER_WAIT + LATE_START_WAIT).as_secs()
            )),
        }
    }
}

/// Takes the output off a stream, so its frames stop reaching the main thread.
fn remove_output(live: &Live) {
    // SAFETY: the output was added for the screen; a second removal fails and is ignored.
    let _ = unsafe {
        live.stream.removeStreamOutput_type_error(
            ProtocolObject::from_ref(&*live.output),
            SCStreamOutputType::Screen,
        )
    };
}

/// Stops a running stream and waits a moment for it.
fn close(live: Live) {
    remove_output(&live);
    if let Err(e) = wait_for("the screen capture didn't stop", STOP_WAIT, |done| {
        // SAFETY: the handler is called once the stream stopped.
        unsafe { live.stream.stopCaptureWithCompletionHandler(Some(done)) };
    }) {
        crate::log(format_args!("generation {}: {e}", live.generation));
    }
}

/// The pixels the filter's content has: its rectangle in points times its point-to-pixel scale
/// (macOS 14), which follows the display's current mode. `None` when the filter gives no size.
fn filter_pixels(filter: &SCContentFilter) -> Option<(u32, u32)> {
    // SAFETY: both are plain getters of the filter.
    let (rect, scale) = unsafe { (filter.contentRect(), filter.pointPixelScale()) };
    pixels_of(rect.size.width, rect.size.height, f64::from(scale))
}

fn pixels_of(width: f64, height: f64, scale: f64) -> Option<(u32, u32)> {
    let (width, height) = ((width * scale).round(), (height * scale).round());
    (width.is_finite() && height.is_finite() && width >= 2.0 && height >= 2.0)
        .then_some((width as u32, height as u32))
}

/// The size the stream delivers for a display of `width` x `height` pixels: the display's own
/// pixels, so a Retina display is captured at its native resolution and the text OCR reads is
/// sharp, unless that is more than the encoder takes (4096 on a side, 36,864 macroblocks). Then it
/// is the size `caps::video_size` gives the video, a 5K display's 5120 x 2880 becoming 4096 x
/// 2304. ScreenCaptureKit scales on the GPU, so no frame is converted and scaled on the CPU to
/// reach the video's size; the frames' own 2560 pixel limit is below every size that gets here.
/// Both sides are even, so an odd one, rare on a Mac, is scaled by less than a pixel.
pub fn stream_size(width: u32, height: u32) -> (u32, u32) {
    video_size(width, height)
}

/// Whether an application's windows stay out of the recording.
fn excludes(pid: i32, bundle: &str, exclude_pids: &[u32]) -> bool {
    pid > 0 && (exclude_pids.contains(&(pid as u32)) || EXCLUDED_BUNDLES.contains(&bundle))
}

/// What a filter leaves out, from one read of the shareable content.
struct Exclusions {
    applications: Vec<Retained<SCRunningApplication>>,
    /// The pids and bundle ids that matched, and the pids asked for that no shareable
    /// application has, such as a process without a window.
    matched: Matched,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Matched {
    /// Sorted.
    pids: Vec<u32>,
    bundles: Vec<String>,
    unmatched: Vec<u32>,
}

impl Exclusions {
    fn of(content: &SCShareableContent, pids: &[u32]) -> Exclusions {
        let all = unsafe { content.applications() }.to_vec();
        let listed: Vec<(i32, String)> = all
            .iter()
            .map(|application| (unsafe { application.processID() }, bundle_id(application)))
            .collect();
        let (chosen, matched) = matching(&listed, pids);
        Exclusions {
            applications: chosen.into_iter().map(|n| all[n].clone()).collect(),
            matched,
        }
    }

    fn describe(&self) -> String {
        self.matched.describe()
    }
}

impl Matched {
    /// For the log: pids and bundle ids only, never window titles.
    fn describe(&self) -> String {
        let mut line = format!(
            "leaves out pids {:?} (bundles {:?})",
            self.pids, self.bundles
        );
        if !self.unmatched.is_empty() {
            line.push_str(&format!(
                "; no shareable application has pids {:?}",
                self.unmatched
            ));
        }
        line
    }
}

/// Which of the `listed` applications (pid, bundle id) a filter leaves out for `pids`, by index,
/// and what matched.
fn matching(listed: &[(i32, String)], pids: &[u32]) -> (Vec<usize>, Matched) {
    let mut chosen = Vec::new();
    let mut matched = Matched::default();
    for (n, (pid, bundle)) in listed.iter().enumerate() {
        if !excludes(*pid, bundle, pids) {
            continue;
        }
        chosen.push(n);
        matched.pids.push(*pid as u32);
        if EXCLUDED_BUNDLES.contains(&bundle.as_str()) && !matched.bundles.contains(bundle) {
            matched.bundles.push(bundle.clone());
        }
    }
    matched.pids.sort_unstable();
    matched.pids.dedup();
    matched.bundles.sort();
    matched.unmatched = pids
        .iter()
        .copied()
        .filter(|pid| !matched.pids.contains(pid))
        .collect();
    (chosen, matched)
}

/// An application's bundle id, or `""` for a process without one. The generated binding says the
/// property is never nil and panics when it is, which a window of a process without a bundle (a
/// Python or Java window, a binary run from a terminal) may well be: read it as optional.
fn bundle_id(application: &SCRunningApplication) -> String {
    // SAFETY: `bundleIdentifier` takes no arguments and returns an `NSString` or nil.
    let bundle: Option<Retained<NSString>> = unsafe { msg_send![application, bundleIdentifier] };
    bundle.map(|bundle| bundle.to_string()).unwrap_or_default()
}

/// The shareable content, waited for: ScreenCaptureKit only answers through its completion
/// handler.
fn shareable() -> Result<Retained<SCShareableContent>, PlatformError> {
    let (sender, receiver) = mpsc::channel();
    let block = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let answer = match (NonNull::new(content), NonNull::new(error)) {
                (Some(content), _) => unsafe { Retained::retain(content.as_ptr()) }
                    .map(Ok)
                    .unwrap_or_else(|| Err(("the shareable content went away".into(), false))),
                // The handler's arguments are borrowed for the call: retain what is kept.
                (None, Some(error)) => Err(unsafe { Retained::retain(error.as_ptr()) }
                    .map(|error| (describe(&error), permission_lost(error.code())))
                    .unwrap_or_else(|| ("no reason given".into(), false))),
                (None, None) => Err(("no reason given".into(), false)),
            };
            let _ = sender.send(answer);
        },
    );
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    match receiver.recv_timeout(ANSWER_WAIT) {
        Ok(Ok(content)) => Ok(content),
        Ok(Err((why, permission))) => Err(PlatformError {
            permission,
            ..PlatformError::new(format!(
                "the screen's content wasn't readable ({why}); Screen Recording may be off"
            ))
        }),
        Err(_) => Err(PlatformError::new(
            "ScreenCaptureKit didn't answer within 5 s",
        )),
    }
}

/// Calls `start` with a completion handler and waits for it, up to `wait`.
fn wait_for(
    why: &str,
    wait: Duration,
    start: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>),
) -> Result<(), PlatformError> {
    let (sender, receiver): (_, Receiver<Option<(String, bool)>>) = mpsc::channel();
    let block: RcBlock<dyn Fn(*mut NSError)> = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: the error is a live `NSError` for the length of the call, or null.
        let answer = unsafe { Retained::retain(error) }
            .map(|error| (describe(&error), permission_lost(error.code())));
        let _ = sender.send(answer);
    });
    start(&block);
    match receiver.recv_timeout(wait) {
        Ok(None) => Ok(()),
        Ok(Some((reason, permission))) => Err(PlatformError {
            permission,
            ..PlatformError::new(format!("{why}: {reason}"))
        }),
        Err(_) => Err(PlatformError::new(format!(
            "{why}: no answer within {} s",
            wait.as_secs()
        ))),
    }
}

/// An `NSError` as a line for the log: its code and its own words.
fn describe(error: &NSError) -> String {
    format!("{} {}", error.code(), error.localizedDescription())
}

/// The screen process's own error for a failed ScreenCaptureKit call.
fn sc_error(what: &str, error: &NSError) -> PlatformError {
    PlatformError {
        device_lost: false,
        code: format!("SCError {}", error.code()),
        message: format!("{what}: {}", error.localizedDescription()),
        permission: permission_lost(error.code()),
    }
}

/// What one frame's attachment dictionary says about it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Info {
    status: isize,
    /// The share of the picture its dirty rectangles cover, when it reports them.
    dirty: Option<f32>,
}

/// The frame's status and dirty regions from the first dictionary of its sample attachments, as
/// ScreenCaptureKit documents them.
fn info(sample: &CMSampleBuffer, width: u32, height: u32) -> Option<Info> {
    // SAFETY: the call answers with the attachments array, which the sample owns.
    let list = unsafe { sample.sample_attachments_array(false) }?;
    // SAFETY: the array holds one dictionary per sample, and a screen sample has one.
    let list =
        unsafe { &*(&*list as *const CFArray as *const CFArray<CFDictionary<CFString, CFType>>) };
    let values = list.get(0)?;
    let number = |key: &CFString| -> Option<f64> {
        let value = values.get(key)?;
        value.downcast_ref::<CFNumber>()?.as_f64()
    };
    let status = number(cf(unsafe { SCStreamFrameInfoStatus }))?.round() as isize;
    let dirty = values
        .get(cf(unsafe { SCStreamFrameInfoDirtyRects }))
        .and_then(|rects| dirty_fraction(&rects, width, height));
    Some(Info { status, dirty })
}

/// A `SCStreamFrameInfo` key as the `CFString` a CF dictionary takes.
fn cf(key: &'static NSString) -> &'static CFString {
    // SAFETY: the keys of the frame info are `NSString` constants, which are `CFString`s
    // (toll-free bridged), and the statics outlive the process.
    unsafe { &*(key as *const NSString as *const CFString) }
}

/// The share of a `width` x `height` picture the dirty rectangles cover, capped at 1: they may
/// overlap. `None` when the value isn't an array of rectangles this can read, so the frame counts
/// as changed everywhere.
fn dirty_fraction(rects: &CFType, width: u32, height: u32) -> Option<f32> {
    let rects = rects.downcast_ref::<CFArray>()?;
    // SAFETY: the elements are only inspected through their own type.
    let rects = unsafe { &*(rects as *const CFArray as *const CFArray<CFType>) };
    let mut area = 0f64;
    for rect in rects.iter() {
        let rect = rect_of(&rect)?;
        area += rect.size.width.max(0.0) * rect.size.height.max(0.0);
    }
    let total = f64::from(width) * f64::from(height);
    (total > 0.0).then(|| (area / total).min(1.0) as f32)
}

/// One dirty rectangle: Core Graphics' dictionary form, or an `NSValue` holding a `CGRect`.
fn rect_of(value: &CFType) -> Option<CGRect> {
    if let Some(dict) = value.downcast_ref::<CFDictionary>() {
        let mut rect = CGRect::default();
        // SAFETY: the call writes only `rect`.
        return unsafe { CGRectMakeWithDictionaryRepresentation(Some(dict), &mut rect) }
            .then_some(rect);
    }
    // SAFETY: an array element that isn't a dictionary is an `NSValue`, which this checks with
    // its class before reading a rectangle from it.
    let value: &NSObject = unsafe { &*(value as *const CFType as *const NSObject) };
    let value = value.downcast_ref::<NSValue>()?;
    if !value.contains_encoding::<CGRect>() {
        return None;
    }
    // SAFETY: the value holds a `CGRect`, which the encoding check just said.
    Some(unsafe { value.get::<CGRect>() })
}

/// The stream's output and its stop notice, in one object.
struct OutputIvars {
    to_main: SyncSender<Ev<PixelBuffer>>,
    generation: u64,
    width: u32,
    height: u32,
    /// The dirty regions of frames that never reached the main thread; they go with the next one,
    /// so a change is never lost.
    carried: Mutex<f32>,
    /// When the stream last called the output, idle samples included, on the awake clock; 0
    /// before the first call.
    seen: AtomicU64,
    /// The delegate's stop, kept besides the event that may be lost in a full queue.
    stop: Mutex<Option<CaptureStop>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class has no `Drop`.
    #[unsafe(super(NSObject))]
    #[ivars = OutputIvars]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn output(
            &self,
            _stream: &SCStream,
            sample: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind == SCStreamOutputType::Screen {
                self.on_sample(sample);
            }
        }
    }

    unsafe impl SCStreamDelegate for Output {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn stopped(&self, _stream: &SCStream, error: &NSError) {
            // The ivars live as long as the object, which `Live` holds until the stream stopped.
            self.on_stop(CaptureStop {
                why: describe(error),
                permission_lost: permission_lost(error.code()),
                by_user: error.code() == SCStreamErrorCode::UserStopped.0,
            });
        }
    }
);

impl Output {
    fn new(
        to_main: SyncSender<Ev<PixelBuffer>>,
        generation: u64,
        width: u32,
        height: u32,
    ) -> Retained<Output> {
        let this = Output::alloc().set_ivars(OutputIvars {
            to_main,
            generation,
            width,
            height,
            carried: Mutex::new(0.0),
            seen: AtomicU64::new(0),
            stop: Mutex::new(None),
        });
        // SAFETY: the ivars are set, and NSObject's `init` takes no arguments.
        unsafe { msg_send![super(this), init] }
    }

    /// The stream stopped by itself: kept for `health`, and sent to the main thread when the
    /// queue has room.
    fn on_stop(&self, stop: CaptureStop) {
        let ivars = self.ivars();
        *ivars.stop.lock().unwrap_or_else(|e| e.into_inner()) = Some(stop.clone());
        let _ = ivars
            .to_main
            .try_send(Ev::CaptureStopped(ivars.generation, stop));
    }

    /// The stop the delegate kept, once.
    fn take_stop(&self) -> Option<CaptureStop> {
        self.ivars()
            .stop
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// When the stream last called the output, on the awake clock; 0 before the first call.
    fn last_callback(&self) -> u64 {
        self.ivars().seen.load(Ordering::Relaxed)
    }

    /// One delivered sample. Idle frames, which say that nothing changed, are not frames, but
    /// they show that the stream lives.
    fn on_sample(&self, sample: &CMSampleBuffer) {
        let ivars = self.ivars();
        ivars
            .seen
            .store(clock_mac::awake_100ns().max(1), Ordering::Relaxed);
        // A sample whose attachments can't be read counts as a frame with new content, of which
        // nothing is known about what changed.
        let info = info(sample, ivars.width, ivars.height).unwrap_or(Info {
            status: SCFrameStatus::Complete.0,
            dirty: None,
        });
        let status = SCFrameStatus(info.status);
        if status == SCFrameStatus::Idle
            || status == SCFrameStatus::Blank
            || status == SCFrameStatus::Suspended
            || status == SCFrameStatus::Stopped
        {
            return;
        }
        // SAFETY: the sample is a screen sample, so it holds a pixel buffer.
        let Some(image) = (unsafe { sample.image_buffer() }) else {
            return;
        };
        let Some(boot) = unsafe { sample.presentation_time_stamp() }.boot_time() else {
            return;
        };
        // The share of the screen that changed since the last frame the main thread got. The
        // first frame is all of it; a frame that doesn't say what changed leaves it unknown.
        let carried = *ivars.carried.lock().unwrap_or_else(|e| e.into_inner());
        let dirty = if status == SCFrameStatus::Started {
            Some(1.0)
        } else {
            info.dirty.map(|d| (carried + d).min(1.0))
        };
        // The buffer has the configured size: `SCStream.h` says a screen sample's width and height
        // are the configuration's. A display whose mode changes keeps it, with the new picture
        // scaled in, until the orchestration sees the monitor's new size and starts again.
        let picture = PixelBuffer::from_image(image);
        let frame = Frame {
            width: picture.width() & !1,
            height: picture.height() & !1,
            picture,
            boot,
            dirty,
            generation: ivars.generation,
        };
        // A full queue means the main thread is behind: drop this frame rather than block the
        // stream's queue, and its changes go with the next one.
        let sent = ivars.to_main.try_send(Ev::Frame(frame));
        let mut carried = ivars.carried.lock().unwrap_or_else(|e| e.into_inner());
        *carried = if sent.is_ok() {
            0.0
        } else {
            dirty.unwrap_or(1.0)
        };
    }
}

/// A sample's presentation time on this process's boot clock, when the sample has one. Its own
/// clock is the host clock, `mach_absolute_time`, which stops while the computer sleeps, so only
/// the frame's age is carried over to the boot clock the meeting runs on.
trait BootTime {
    fn boot_time(self) -> Option<u64>;
}

impl BootTime for CMTime {
    fn boot_time(self) -> Option<u64> {
        if !self.flags.contains(CMTimeFlags::Valid) {
            return None;
        }
        // SAFETY: the call only reads the time value.
        let seconds = unsafe { self.seconds() };
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        Some(clock_mac::frame_boot((seconds * 1e7) as u64))
    }
}

/// Whether a stream error says that this process may not capture the screen any more: the grant
/// was refused or revoked, the user pressed the system's stop button, or the entitlements are
/// missing. Other errors, such as a display that went away, leave the choice to the monitor rules.
fn permission_lost(code: isize) -> bool {
    [
        SCStreamErrorCode::UserDeclined,
        SCStreamErrorCode::UserStopped,
        SCStreamErrorCode::MissingEntitlements,
    ]
    .iter()
    .any(|known| known.0 == code)
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::CFRetained;

    use super::*;

    /// The stream takes a display's native pixels, within what the encoder takes, so the video
    /// writer never has to scale a frame itself.
    #[test]
    fn the_stream_takes_native_pixels_within_the_encoders_limits() {
        // Native: a 16-inch MacBook Pro, a 4K display, a portrait one and a 14-inch MacBook Pro.
        assert_eq!(stream_size(3456, 2234), (3456, 2234));
        assert_eq!(stream_size(3840, 2160), (3840, 2160));
        assert_eq!(stream_size(1440, 2560), (1440, 2560));
        assert_eq!(stream_size(3024, 1964), (3024, 1964));
        // Over what the encoder takes: a 5K display, and a 5K ultrawide.
        assert_eq!(stream_size(5120, 2880), (4096, 2304));
        assert_eq!(stream_size(5120, 1440), (4096, 1152));
        // An odd size is scaled into the even one below it.
        assert_eq!(stream_size(1441, 901), (1440, 900));
        // The video writer sizes its file from the frame, so a frame at the stream's size must be
        // a video of that same size.
        for (w, h) in [
            (3456, 2234),
            (5120, 2880),
            (6016, 3384),
            (5120, 1440),
            (1441, 901),
        ] {
            let (sw, sh) = stream_size(w, h);
            assert_eq!(video_size(sw, sh), (sw, sh), "{w}x{h}");
        }
    }

    /// The filter leaves out the applications with the pids asked for and Voice's and Rukoo's
    /// bundles, and says which asked-for pids no application has.
    #[test]
    fn the_filter_leaves_out_pids_and_bundles() {
        let listed = vec![
            (10, "com.apple.finder".to_string()),
            (20, "nl.bvdm.rukoo-mail".to_string()),
            (30, String::new()),
            (40, "nl.bvdm.rukoo.voice.dev".to_string()),
            (50, "com.github.Electron".to_string()),
            (0, "nl.bvdm.rukoo.voice".to_string()),
        ];
        let (chosen, matched) = matching(&listed, &[50, 30, 77]);
        assert_eq!(chosen, [1, 2, 3, 4]);
        assert_eq!(matched.pids, [20, 30, 40, 50]);
        assert_eq!(
            matched.bundles,
            ["nl.bvdm.rukoo-mail", "nl.bvdm.rukoo.voice.dev"]
        );
        assert_eq!(matched.unmatched, [77]);
        assert!(
            matched.describe().contains("[77]"),
            "{}",
            matched.describe()
        );
        // A restarted Voice is a new pid with the same bundle: the set the filter needs changes.
        let mut restarted = listed.clone();
        restarted[3].0 = 41;
        assert_ne!(matching(&restarted, &[50, 30, 77]).1, matched);
    }

    /// The stream's size is the filter's content in pixels: points times the scale.
    #[test]
    fn the_filters_pixels_are_points_times_the_scale() {
        assert_eq!(pixels_of(1728.0, 1117.0, 2.0), Some((3456, 2234)));
        assert_eq!(pixels_of(1920.0, 1080.0, 1.0), Some((1920, 1080)));
        assert_eq!(pixels_of(0.0, 0.0, 2.0), None);
        assert_eq!(pixels_of(1728.0, 1117.0, 0.0), None);
        assert_eq!(pixels_of(f64::NAN, 1117.0, 2.0), None);
    }

    /// The exclusions are this process, its parent and the command line's, without repeats and
    /// without the 0 a bad caller could pass.
    #[test]
    fn the_exclusions_name_this_process_and_the_command_line() {
        let pids = ScreenSource::excluded_pids(&[7, 7, 0, 42]);
        assert!(pids.contains(&std::process::id()));
        assert!(pids.contains(&42) && pids.contains(&7));
        assert!(!pids.contains(&0));
        let mut sorted = pids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, pids, "not sorted and unique");
    }

    /// Voice, a development Voice and Rukoo are left out by bundle id, and anything else by pid.
    #[test]
    fn the_excluded_applications_are_voice_rukoo_and_the_named_pids() {
        assert!(excludes(10, "nl.bvdm.rukoo-mail", &[]));
        assert!(excludes(11, "nl.bvdm.rukoo.voice", &[]));
        assert!(excludes(12, "nl.bvdm.rukoo.voice.dev", &[]));
        assert!(excludes(13, "com.github.Electron", &[13]));
        assert!(!excludes(14, "com.github.Electron", &[13]));
        assert!(!excludes(15, "com.apple.Safari", &[]));
        // A process without a bundle id, as some helpers have, is still excluded by pid.
        assert!(excludes(16, "", &[16]));
        assert!(!excludes(0, "nl.bvdm.rukoo-mail", &[]));
    }

    /// A screen sample as ScreenCaptureKit hands one over: `picture` with a presentation time on
    /// the host clock and the frame info in its attachments.
    fn sample(
        picture: &PixelBuffer,
        pts: CMTime,
        info: &[(&CFString, &CFType)],
    ) -> CFRetained<CMSampleBuffer> {
        use objc2_core_foundation::CFMutableDictionary;
        use objc2_core_media::{
            CMSampleTimingInfo, CMVideoFormatDescription,
            CMVideoFormatDescriptionCreateForImageBuffer,
        };
        let image: &objc2_core_video::CVImageBuffer = picture.buffer();
        let mut format: *const CMVideoFormatDescription = std::ptr::null();
        // SAFETY: the out pointer is valid, and the call returns a retained description.
        let status = unsafe {
            CMVideoFormatDescriptionCreateForImageBuffer(None, image, NonNull::from(&mut format))
        };
        assert_eq!(status, 0);
        let format = unsafe { CFRetained::from_raw(NonNull::new(format.cast_mut()).unwrap()) };
        let mut timing = CMSampleTimingInfo {
            duration: unsafe { CMTime::new(1, FPS as i32) },
            presentationTimeStamp: pts,
            decodeTimeStamp: unsafe { objc2_core_media::kCMTimeInvalid },
        };
        let mut out: *mut CMSampleBuffer = std::ptr::null_mut();
        // SAFETY: every pointer is valid for the call, which returns a retained sample.
        let status = unsafe {
            CMSampleBuffer::create_ready_with_image_buffer(
                None,
                image,
                &format,
                NonNull::from(&mut timing),
                NonNull::from(&mut out),
            )
        };
        assert_eq!(status, 0);
        let sample = unsafe { CFRetained::from_raw(NonNull::new(out).unwrap()) };
        if !info.is_empty() {
            let list = unsafe { sample.sample_attachments_array(true) }.unwrap();
            // SAFETY: a sample's attachments are one mutable dictionary per sample.
            let dict = unsafe { CFArray::value_at_index(&list, 0) } as *const CFMutableDictionary;
            for (key, value) in info {
                unsafe {
                    CFMutableDictionary::set_value(
                        dict.as_ref(),
                        (*key as *const CFString).cast(),
                        (*value as *const CFType).cast(),
                    )
                };
            }
        }
        sample
    }

    /// A presentation time on the host clock, `ms` ago.
    fn host_time(ms_ago: u64) -> CMTime {
        let now = clock_mac::awake_100ns().saturating_sub(ms_ago * 10_000);
        unsafe { CMTime::new(now as i64, 10_000_000) }
    }

    fn status(status: SCFrameStatus) -> CFRetained<CFNumber> {
        CFNumber::new_isize(status.0)
    }

    fn picture() -> PixelBuffer {
        let slide = super::super::synthetic_mac::slide("Slide 1", 1, 64, 48).unwrap();
        PixelBuffer::from_pixels_420f(&slide).unwrap()
    }

    /// An `NSValue` holding a rectangle, as ScreenCaptureKit documents the dirty rectangles.
    fn ns_rect(x: f64, y: f64, w: f64, h: f64) -> Retained<NSValue> {
        NSValue::new(CGRect::new(
            objc2_core_foundation::CGPoint::new(x, y),
            objc2_core_foundation::CGSize::new(w, h),
        ))
    }

    /// Core Graphics' dictionary form of a rectangle.
    fn dict_rect(x: f64, y: f64, w: f64, h: f64) -> CFRetained<CFDictionary<CFString, CFNumber>> {
        let key = CFString::from_static_str;
        CFDictionary::from_slices(
            &[&*key("X"), &*key("Y"), &*key("Width"), &*key("Height")],
            &[
                &*CFNumber::new_f64(x),
                &*CFNumber::new_f64(y),
                &*CFNumber::new_f64(w),
                &*CFNumber::new_f64(h),
            ],
        )
    }

    fn as_cf(value: &NSObject) -> &CFType {
        // SAFETY: Foundation objects are Core Foundation objects (toll-free bridged).
        unsafe { &*(value as *const NSObject as *const CFType) }
    }

    /// The frame info: its status, and the share of the picture its dirty rectangles cover, in
    /// either form, overlaps capped at the whole picture.
    #[test]
    fn the_frame_info_reads_status_and_dirty_rectangles() {
        let picture = picture();
        let complete = status(SCFrameStatus::Complete);
        let key_status = cf(unsafe { SCStreamFrameInfoStatus });
        let key_dirty = cf(unsafe { SCStreamFrameInfoDirtyRects });

        // A quarter of 64 x 48 as NSValues, and as a dictionary plus an NSValue.
        let rects = NSArray::from_retained_slice(&[ns_rect(0.0, 0.0, 32.0, 24.0)]);
        let s = sample(
            &picture,
            host_time(5),
            &[(key_status, &complete), (key_dirty, as_cf(&rects))],
        );
        let read = info(&s, 64, 48).unwrap();
        assert_eq!(read.status, SCFrameStatus::Complete.0);
        assert!((read.dirty.unwrap() - 0.25).abs() < 1e-6, "{read:?}");

        let both = CFArray::<CFType>::from_objects(&[
            &*dict_rect(0.0, 0.0, 64.0, 12.0),
            as_cf(&ns_rect(0.0, 12.0, 64.0, 12.0)),
        ]);
        let s = sample(
            &picture,
            host_time(5),
            &[(key_status, &complete), (key_dirty, &both)],
        );
        assert!((info(&s, 64, 48).unwrap().dirty.unwrap() - 0.5).abs() < 1e-6);

        // Overlapping rectangles cover the picture at most once.
        let over = NSArray::from_retained_slice(&[
            ns_rect(0.0, 0.0, 64.0, 48.0),
            ns_rect(0.0, 0.0, 64.0, 48.0),
        ]);
        let s = sample(
            &picture,
            host_time(5),
            &[(key_status, &complete), (key_dirty, as_cf(&over))],
        );
        assert_eq!(info(&s, 64, 48).unwrap().dirty, Some(1.0));

        // No rectangles is no change; a value that isn't an array of rectangles is unknown.
        let none = NSArray::<NSValue>::from_slice(&[]);
        let s = sample(
            &picture,
            host_time(5),
            &[(key_status, &complete), (key_dirty, as_cf(&none))],
        );
        assert_eq!(info(&s, 64, 48).unwrap().dirty, Some(0.0));
        let s = sample(
            &picture,
            host_time(5),
            &[(key_status, &complete), (key_dirty, &complete)],
        );
        assert_eq!(info(&s, 64, 48).unwrap().dirty, None);
        let odd = CFArray::<CFType>::from_objects(&[&*complete]);
        let s = sample(
            &picture,
            host_time(5),
            &[(key_status, &complete), (key_dirty, &odd)],
        );
        assert_eq!(info(&s, 64, 48).unwrap().dirty, None);
        // A rectangle of a picture without size reads as unknown, not as a division by zero.
        assert_eq!(dirty_fraction(as_cf(&rects), 0, 0), None);

        // No status, or no attachments at all: nothing is known.
        let s = sample(&picture, host_time(5), &[(key_dirty, as_cf(&rects))]);
        assert_eq!(info(&s, 64, 48), None);
        let s = sample(&picture, host_time(5), &[]);
        assert_eq!(info(&s, 64, 48), None);
    }

    /// A sample's age crosses to the boot clock; a time without the valid flag, or before the
    /// clock began, has none.
    #[test]
    fn a_samples_time_crosses_to_the_boot_clock() {
        let boot = host_time(40).boot_time().unwrap();
        let age = clock_mac::boot_100ns().saturating_sub(boot);
        assert!((350_000..=1_500_000).contains(&age), "{age}");
        assert_eq!(
            unsafe { objc2_core_media::kCMTimeInvalid }.boot_time(),
            None
        );
        assert_eq!(unsafe { CMTime::new(-5, 1) }.boot_time(), None);
        // A time in the future counts as now.
        let ahead =
            unsafe { CMTime::new(clock_mac::awake_100ns() as i64 + 50_000_000, 10_000_000) };
        assert!(ahead.boot_time().unwrap() <= clock_mac::boot_100ns());
    }

    /// A stream is stalled once it called nothing for longer than `STALL_AFTER`.
    #[test]
    fn a_quiet_stream_is_stalled_after_ten_seconds() {
        let ten_s = 100_000_000;
        assert!(!stalled(5_000, 5_000 + ten_s));
        assert!(stalled(5_000, 5_000 + ten_s + 1));
        // A clock read before the callback's own time is no stall.
        assert!(!stalled(5_000, 4_000));
    }

    /// The delegate's stop reaches the main thread through the queue, and is kept for `health`
    /// when the queue is full, as when the main thread is busy finishing a video file.
    #[test]
    fn a_stop_is_kept_when_the_queue_is_full() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let output = Output::new(sender.clone(), 3, 64, 48);
        let stop = CaptureStop {
            why: "-3821 the system stopped the stream".into(),
            permission_lost: false,
            by_user: false,
        };
        output.on_stop(stop.clone());
        assert!(matches!(receiver.try_recv(), Ok(Ev::CaptureStopped(3, s)) if s == stop));
        assert_eq!(output.take_stop(), Some(stop.clone()));
        assert_eq!(output.take_stop(), None, "taken once");
        // A full queue loses the event; `health` still finds the stop.
        sender
            .try_send(Ev::DeviceLost(0))
            .expect("room for one event");
        output.on_stop(stop.clone());
        assert_eq!(output.take_stop(), Some(stop));
        assert!(matches!(receiver.try_recv(), Ok(Ev::DeviceLost(0))));
        assert!(receiver.try_recv().is_err());
        // Any sample, an idle one too, shows the stream lives.
        assert_eq!(output.last_callback(), 0);
        output.on_sample(&sample(&picture(), host_time(5), &[]));
        assert!(output.last_callback() > 0);
    }

    /// The codes that end the screen part, and two that don't by themselves.
    #[test]
    fn the_permission_codes() {
        assert!(permission_lost(SCStreamErrorCode::UserDeclined.0));
        assert!(permission_lost(SCStreamErrorCode::UserStopped.0));
        assert!(permission_lost(SCStreamErrorCode::MissingEntitlements.0));
        assert!(!permission_lost(
            SCStreamErrorCode::FailedApplicationConnectionInterrupted.0
        ));
        assert!(!permission_lost(SCStreamErrorCode::SystemStoppedStream.0));
        assert!(!permission_lost(0));
    }

    /// The output class registers, which in a debug build checks its methods against the
    /// `SCStreamOutput` and `SCStreamDelegate` protocols, and hands a sample to the main thread
    /// the way a stream's queue calls it: idle samples aren't frames, the first frame is all
    /// changed, and a frame the full queue refused carries its change to the next one.
    #[test]
    fn the_output_registers_and_passes_frames_on() {
        use objc2::{sel, ProtocolType};
        let (sender, receiver) = mpsc::sync_channel(1);
        let output = Output::new(sender, 7, 64, 48);
        // A Rust process has no `SCStreamOutput` or `SCStreamDelegate` in its runtime: the
        // protocols' metadata lives in the binaries that adopt them, and ScreenCaptureKit doesn't
        // register them itself. So the class can't declare them and objc2's debug check of the
        // method signatures against them doesn't run; ScreenCaptureKit calls the methods by
        // selector, which the real capture at the Mac showed. Should a system register them, the
        // class must conform.
        for protocol in [
            <dyn SCStreamOutput>::protocol(),
            <dyn SCStreamDelegate>::protocol(),
        ]
        .into_iter()
        .flatten()
        {
            assert!(
                output.conformsToProtocol(protocol),
                "{}",
                protocol.name().to_string_lossy()
            );
        }
        for selector in [
            sel!(stream:didOutputSampleBuffer:ofType:),
            sel!(stream:didStopWithError:),
        ] {
            assert!(output.respondsToSelector(selector), "{selector:?}");
        }
        let picture = picture();
        let key_status = cf(unsafe { SCStreamFrameInfoStatus });
        let key_dirty = cf(unsafe { SCStreamFrameInfoDirtyRects });
        let frame = |status_: SCFrameStatus, rects: &[Retained<NSValue>], ms_ago: u64| {
            let rects = NSArray::from_retained_slice(rects);
            let status_ = status(status_);
            sample(
                &picture,
                host_time(ms_ago),
                &[(key_status, &status_), (key_dirty, as_cf(&rects))],
            )
        };
        let take = || match receiver.try_recv() {
            Ok(Ev::Frame(frame)) => Some(frame),
            Ok(_) => panic!("not a frame"),
            Err(_) => None,
        };

        output.on_sample(&frame(SCFrameStatus::Started, &[], 10));
        let first = take().expect("the first frame");
        assert_eq!((first.generation, first.width, first.height), (7, 64, 48));
        assert_eq!(first.dirty, Some(1.0));
        assert!(first.picture.is_biplanar());

        for idle in [
            SCFrameStatus::Idle,
            SCFrameStatus::Blank,
            SCFrameStatus::Suspended,
        ] {
            output.on_sample(&frame(idle, &[], 5));
            assert!(take().is_none(), "{idle:?} is a frame");
        }

        // One frame fills the queue of one; the next is refused and its change carried.
        output.on_sample(&frame(
            SCFrameStatus::Complete,
            &[ns_rect(0.0, 0.0, 64.0, 12.0)],
            5,
        ));
        output.on_sample(&frame(
            SCFrameStatus::Complete,
            &[ns_rect(0.0, 12.0, 64.0, 12.0)],
            4,
        ));
        assert!((take().unwrap().dirty.unwrap() - 0.25).abs() < 1e-6);
        output.on_sample(&frame(
            SCFrameStatus::Complete,
            &[ns_rect(0.0, 24.0, 64.0, 12.0)],
            3,
        ));
        let carried = take().unwrap();
        assert!(
            (carried.dirty.unwrap() - 0.5).abs() < 1e-6,
            "{:?}",
            carried.dirty
        );

        // A sample without a valid time isn't a frame.
        let untimed = sample(&picture, unsafe { objc2_core_media::kCMTimeInvalid }, &[]);
        output.on_sample(&untimed);
        assert!(take().is_none());
    }

    /// The real screen: it needs the Screen Recording grant, so it is ignored and never runs in
    /// CI. Run it at a Mac with `--ignored`. A still desktop sends one frame with a picture and
    /// then idle samples, so one frame is all it asks for; it prints how many more came in 3 s,
    /// which says how often a real screen sends frames.
    #[test]
    #[ignore = "needs the Screen Recording grant and captures the real screen"]
    fn captures_the_real_screen() {
        assert!(
            monitors_mac::has_permission(),
            "grant Screen Recording to this test binary first"
        );
        let monitors = monitors_mac::list();
        let Some(monitor) = monitors.first() else {
            eprintln!("no display to record");
            return;
        };
        let (sender, receiver) = mpsc::sync_channel(8);
        let mut source = ScreenSource::new(sender, Vec::new());
        // The start runs on a thread of its own; a failure comes back as an event.
        source.start(monitor, 1).expect("the capture didn't start");
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        let first = loop {
            assert!(
                std::time::Instant::now() < deadline,
                "the real screen sent no frame"
            );
            match receiver.recv_timeout(Duration::from_millis(500)) {
                Ok(Ev::Frame(frame)) => break frame,
                Ok(Ev::CaptureStopped(_, stop)) => panic!("the capture stopped: {}", stop.why),
                _ => {}
            }
        };
        assert_eq!(first.generation, 1);
        assert!(first.width >= 2 && first.height >= 2);
        assert_eq!((first.width % 2, first.height % 2), (0, 0));
        // The stream asks for 420f: the encoder takes that buffer as it is.
        assert_eq!(
            first.picture.pixel_format(),
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        );
        let pixels = first.picture.pixels().expect("the frame's pixels");
        assert_eq!((pixels.width, pixels.height), (first.width, first.height));
        // A real screen isn't uniform: some pixels differ from the first.
        let corner = pixels.bgra[..4].to_vec();
        assert!(
            pixels
                .bgra
                .chunks_exact(4)
                .skip(1)
                .any(|px| px != corner.as_slice()),
            "the captured frame is one colour"
        );
        eprintln!(
            "the first frame is {}x{} for a {}x{} monitor",
            first.width, first.height, monitor.width, monitor.height
        );
        let watch = std::time::Instant::now() + Duration::from_secs(3);
        let mut more = 0;
        while std::time::Instant::now() < watch {
            if let Ok(Ev::Frame(_)) = receiver.recv_timeout(Duration::from_millis(100)) {
                more += 1;
            }
        }
        eprintln!("{more} more frames with a change in 3 s");
        assert_eq!(
            source.health(),
            None,
            "the stream counts as stopped or stalled"
        );
        source.stop();
    }

    /// The filter as the stream gets it, from the real shareable content: every application it
    /// leaves out matches a pid asked for or one of Voice's and Rukoo's bundle ids, no
    /// application with such a bundle id is missed, and the filter knows its display's size. A
    /// windowless process, such as this test, isn't among the shareable applications, so its pid
    /// is reported as matching nothing rather than asserted present. Whether a left-out window is
    /// missing from a frame is a check at the Mac with Voice's Settings open over a window. It
    /// needs the grant, so it is ignored.
    #[test]
    #[ignore = "needs the Screen Recording grant"]
    fn the_exclusions_reach_the_shareable_applications() {
        assert!(monitors_mac::has_permission());
        let content = shareable().expect("no shareable content");
        let pids = ScreenSource::excluded_pids(&[]);
        let exclusions = Exclusions::of(&content, &pids);
        eprintln!("the filter {}", exclusions.describe());
        let all = unsafe { content.applications() }.to_vec();
        for application in &exclusions.applications {
            let pid = unsafe { application.processID() };
            assert!(
                excludes(pid, &bundle_id(application), &pids),
                "pid {pid} is left out without a reason"
            );
        }
        for application in &all {
            if EXCLUDED_BUNDLES.contains(&bundle_id(application).as_str()) {
                let pid = unsafe { application.processID() } as u32;
                assert!(
                    exclusions.matched.pids.contains(&pid),
                    "pid {pid} is missed"
                );
            }
        }
        // A pid asked for that matches nothing is named, not lost.
        for pid in &exclusions.matched.unmatched {
            assert!(pids.contains(pid));
        }
        let Some(display) = unsafe { content.displays() }.to_vec().into_iter().next() else {
            eprintln!("no display");
            return;
        };
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::from_retained_slice(&exclusions.applications),
                &NSArray::<SCWindow>::from_slice(&[]),
            )
        };
        let pixels = filter_pixels(&filter).expect("the filter's size");
        let id = unsafe { display.displayID() }.to_string();
        if let Some(monitor) = monitors_mac::list().into_iter().find(|m| m.id == id) {
            assert_eq!(
                pixels,
                (monitor.width, monitor.height),
                "the filter's pixels and the monitor list's differ"
            );
        }
    }
}
