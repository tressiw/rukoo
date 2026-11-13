//! The real screen on macOS: ScreenCaptureKit's `SCStream` on one display.
//!
//! The stream is set up once per capture session from `SCShareableContent`: the display with the
//! id the monitor list chose, and every running application that stays out of the recording
//! (Voice's own app, Rukoo, and the pids the command line names). Its filter is
//! `initWithDisplay:excludingApplications:exceptingWindows:`, so those applications' windows are
//! not in the frames and the desktop behind them is.
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
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFString, CFType, CGRect};
use objc2_core_graphics::{
    kCGDisplayStreamYCbCrMatrix_ITU_R_709_2, CGRectMakeWithDictionaryRepresentation,
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
/// How long a stopping stream may take before its frames are dropped by their generation anyway.
const STOP_WAIT: Duration = Duration::from_secs(2);

pub struct ScreenSource {
    to_main: SyncSender<Ev<PixelBuffer>>,
    /// The pids from `--exclude-pid`.
    exclude_pids: Vec<u32>,
    live: Option<Live>,
}

struct Live {
    stream: Retained<SCStream>,
    output: Retained<Output>,
    /// The filter the stream captures with, kept for a later `updateContentFilter`.
    filter: Retained<SCContentFilter>,
    /// The queue the output is called on; it must outlive the stream's output.
    _queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: ScreenCaptureKit's objects may be called from any thread, and this one is only used by
// the thread that owns the source: it starts and stops the stream. Frames and the stop notice
// reach the main thread through the channel, never through these objects.
unsafe impl Send for Live {}

impl ScreenSource {
    pub fn new(to_main: SyncSender<Ev<PixelBuffer>>, exclude_pids: Vec<u32>) -> ScreenSource {
        ScreenSource {
            to_main,
            exclude_pids,
            live: None,
        }
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

impl Source for ScreenSource {
    fn start(&mut self, monitor: &MonitorInfo, generation: u64) -> Result<(), PlatformError> {
        self.stop();
        let display_id: u32 = monitor.id.parse().map_err(|_| {
            PlatformError::new(format!(
                "display {} isn't a display this Mac has",
                monitor.id
            ))
        })?;
        let content = shareable()?;
        let displays = unsafe { content.displays() }.to_vec();
        let display = displays
            .into_iter()
            .find(|display| unsafe { display.displayID() } == display_id)
            .ok_or_else(|| PlatformError::new(format!("display {} isn't there", monitor.id)))?;
        let pids = ScreenSource::excluded_pids(&self.exclude_pids);
        let applications: Vec<Retained<SCRunningApplication>> = unsafe { content.applications() }
            .to_vec()
            .into_iter()
            .filter(|application| {
                let pid = unsafe { application.processID() };
                let bundle = unsafe { application.bundleIdentifier() }.to_string();
                excludes(pid, &bundle, &pids)
            })
            .collect();
        // SAFETY: every object lives until the filter has retained what it needs, and the empty
        // windows array is an exception list of none.
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::from_retained_slice(&applications),
                &NSArray::<SCWindow>::from_slice(&[]),
            )
        };
        let (width, height) = stream_size(monitor.width, monitor.height);
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
        }
        let output = Output::new(self.to_main.clone(), generation, width, height);
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
        unsafe {
            stream.addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Screen,
                Some(&queue),
            )
        }
        .map_err(|error| sc_error("the screen output wasn't taken", &error))?;
        wait_for("the screen capture didn't start", ANSWER_WAIT, |done| {
            // SAFETY: the handler is called once the stream started or gave up.
            unsafe { stream.startCaptureWithCompletionHandler(Some(done)) };
        })?;
        self.live = Some(Live {
            stream,
            output,
            filter,
            _queue: queue,
        });
        Ok(())
    }

    fn stop(&mut self) {
        let Some(live) = self.live.take() else {
            return;
        };
        // SAFETY: the output was added for the screen, and removing it twice never happens.
        let _ = unsafe {
            live.stream.removeStreamOutput_type_error(
                ProtocolObject::from_ref(&*live.output),
                SCStreamOutputType::Screen,
            )
        };
        let _ = wait_for("the screen capture didn't stop", STOP_WAIT, |done| {
            // SAFETY: the handler is called once the stream stopped.
            unsafe { live.stream.stopCaptureWithCompletionHandler(Some(done)) };
        });
        // The filter is the last thing `Live` holds, after the stream; dropping it here keeps
        // that order explicit.
        drop(live.filter);
    }
}

impl Drop for ScreenSource {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The size the stream delivers for a display of `width` x `height` pixels: the display's own
/// pixels, so a Retina display is captured at its native resolution and the text OCR reads is
/// sharp, unless that is more than the encoder takes (4096 on a side, 36,864 macroblocks). Then it
/// is the size `caps::video_size` gives the video, a 5K display's 5120 x 2880 becoming 4096 x
/// 2304. ScreenCaptureKit scales on the GPU, so no frame is converted and scaled on the CPU to
/// reach the video's size; the frames' own 2560 pixel limit is below every size that gets here.
pub fn stream_size(width: u32, height: u32) -> (u32, u32) {
    video_size(width, height)
}

/// Whether an application's windows stay out of the recording.
fn excludes(pid: i32, bundle: &str, exclude_pids: &[u32]) -> bool {
    pid > 0 && (exclude_pids.contains(&(pid as u32)) || EXCLUDED_BUNDLES.contains(&bundle))
}

/// The pids of the running applications that the bundle ids name, for the display choice: the
/// windows of Rukoo and Voice don't count as the window in front. Empty when the shareable
/// content can't be read.
pub fn bundle_pids() -> Vec<u32> {
    let Ok(content) = shareable() else {
        return Vec::new();
    };
    unsafe { content.applications() }
        .to_vec()
        .into_iter()
        .filter(|application| {
            let bundle = unsafe { application.bundleIdentifier() }.to_string();
            EXCLUDED_BUNDLES.contains(&bundle.as_str())
        })
        .map(|application| unsafe { application.processID() } as u32)
        .collect()
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
                    .unwrap_or_else(|| Err("the shareable content went away".into())),
                // The handler's arguments are borrowed for the call: retain what is kept.
                (None, Some(error)) => Err(unsafe { Retained::retain(error.as_ptr()) }
                    .map(|error| describe(&error))
                    .unwrap_or_else(|| "no reason given".into())),
                (None, None) => Err("no reason given".into()),
            };
            let _ = sender.send(answer);
        },
    );
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    match receiver.recv_timeout(ANSWER_WAIT) {
        Ok(Ok(content)) => Ok(content),
        Ok(Err(why)) => Err(PlatformError::new(format!(
            "the screen's content wasn't readable ({why}); Screen Recording may be off"
        ))),
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
    let (sender, receiver): (_, Receiver<Option<String>>) = mpsc::channel();
    let block: RcBlock<dyn Fn(*mut NSError)> = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: the error is a live `NSError` for the length of the call, or null.
        let answer = unsafe { Retained::retain(error) }.map(|error| describe(&error));
        let _ = sender.send(answer);
    });
    start(&block);
    match receiver.recv_timeout(wait) {
        Ok(None) => Ok(()),
        Ok(Some(reason)) => Err(PlatformError::new(format!("{why}: {reason}"))),
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
            // SAFETY: the ivars live as long as the object, which the stream holds.
            let ivars = self.ivars();
            let _ = ivars.to_main.try_send(Ev::CaptureStopped(
                ivars.generation,
                CaptureStop {
                    why: describe(error),
                    permission_lost: permission_lost(error.code()),
                },
            ));
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
        });
        // SAFETY: the ivars are set, and NSObject's `init` takes no arguments.
        unsafe { msg_send![super(this), init] }
    }

    /// One delivered sample. Idle frames, which say that nothing changed, are not frames.
    fn on_sample(&self, sample: &CMSampleBuffer) {
        let ivars = self.ivars();
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
        // The buffer's own size, which differs from the configured one when the display's mode
        // changed since the stream started; the orchestration ends the video file on a change.
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
        // An odd size loses its last row or column.
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

    /// The real screen: it needs the Screen Recording grant, so it is ignored and never runs in
    /// CI. Run it at a Mac with `--ignored`, on a desktop nobody is using.
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
        source.start(monitor, 1).expect("the capture didn't start");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut frames = 0;
        let mut seen = None;
        while frames < 2 && std::time::Instant::now() < deadline {
            if let Ok(Ev::Frame(frame)) = receiver.recv_timeout(Duration::from_millis(500)) {
                assert_eq!(frame.generation, 1);
                assert!(frame.width >= 2 && frame.height >= 2);
                assert_eq!(frame.width % 2, 0);
                assert_eq!(frame.height % 2, 0);
                // The stream asks for 420f: the encoder takes that buffer as it is.
                assert_eq!(
                    frame.picture.pixel_format(),
                    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
                );
                if seen.is_none() {
                    let pixels = frame.picture.pixels().expect("the frame's pixels");
                    assert_eq!((pixels.width, pixels.height), (frame.width, frame.height));
                    // A real screen isn't uniform: some pixels differ from the first.
                    let first = pixels.bgra[..4].to_vec();
                    assert!(
                        pixels
                            .bgra
                            .chunks_exact(4)
                            .skip(1)
                            .any(|px| px != first.as_slice()),
                        "the captured frame is one colour"
                    );
                    seen = Some(pixels);
                }
                frames += 1;
            }
        }
        source.stop();
        assert!(frames >= 2, "the real screen sent no frames");
    }

    /// The filter leaves out the applications it names. It needs the grant, so it is ignored.
    #[test]
    #[ignore = "needs the Screen Recording grant"]
    fn the_exclusions_reach_the_shareable_applications() {
        assert!(monitors_mac::has_permission());
        let content = shareable().expect("no shareable content");
        let pids = ScreenSource::excluded_pids(&[]);
        let excluded: Vec<u32> = unsafe { content.applications() }
            .to_vec()
            .into_iter()
            .filter(|application| {
                excludes(
                    unsafe { application.processID() },
                    &unsafe { application.bundleIdentifier() }.to_string(),
                    &pids,
                )
            })
            .map(|application| unsafe { application.processID() } as u32)
            .collect();
        // This test binary's own process is in the list; a Mac with Voice running has that too.
        assert!(excluded.contains(&std::process::id()));
    }
}
