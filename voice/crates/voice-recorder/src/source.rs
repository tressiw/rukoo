//! Audio sources: real devices through cpal (Windows and macOS), and synthetic and file sources
//! for tests.
//!
//! Every source delivers interleaved `f32` blocks and failures through the same [`Sink`], so a
//! test source's simulated device loss takes exactly the path a real one does.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::record::Event;

#[derive(Debug, Clone, PartialEq)]
pub enum SourceSpec {
    /// The default microphone, following the default when it changes.
    DefaultInput,
    /// Loopback of the default output device, following the default when it changes.
    DefaultOutput,
    /// One microphone by device id, as `devices` prints it.
    Input(String),
    /// Loopback of one output device by device id.
    Output(String),
    /// A sine tone, for tests.
    Tone { hz: f32, faults: Faults },
    /// A healthy loopback endpoint with nothing playing: it delivers no packets, for tests of
    /// the call-track padding.
    Idle { faults: Faults },
    /// A macOS process tap, for tests: it delivers blocks of exact zeros all the time, as a tap
    /// does while nothing plays or when macOS doesn't let Voice record system audio. With
    /// `sound` it plays a tone between those two times, in ms since the recorder started.
    /// `playing` says another app plays sound the whole time.
    Tap {
        faults: Faults,
        sound: Option<(u64, Option<u64>)>,
        playing: bool,
    },
    /// A WAV file played in real time, then silence, or looped.
    File { path: PathBuf, looped: bool },
}

/// What happens to a synthetic device, in ms since the recorder started: it disappears at
/// `fail_at` and comes back at `back_at`, and at `switch_at` another device becomes the default.
/// Opening it takes `open_ms`, as a slow Bluetooth endpoint can. From `skip_at` it delivers
/// nothing for `skip_ms` and then goes on in time, the audio of that stretch lost, as a device
/// that glitches does.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Faults {
    pub fail_at: Option<u64>,
    pub back_at: Option<u64>,
    pub switch_at: Option<u64>,
    pub open_ms: Option<u64>,
    pub skip_at: Option<u64>,
    pub skip_ms: Option<u64>,
}

impl Faults {
    /// Whether the device delivers nothing at this moment, without failing.
    fn skipping(&self, elapsed_ms: u64) -> bool {
        match (self.skip_at, self.skip_ms) {
            (Some(at), Some(ms)) => elapsed_ms >= at && elapsed_ms < at + ms,
            _ => false,
        }
    }

    pub fn unavailable(&self, elapsed_ms: u64) -> bool {
        self.fail_at.is_some_and(|f| elapsed_ms >= f) && self.back_at.is_none_or(|b| elapsed_ms < b)
    }

    /// 1 for the first default device, 2 once the default switched.
    fn device_number(&self, elapsed_ms: u64) -> u32 {
        if self.switch_at.is_some_and(|s| elapsed_ms >= s) {
            2
        } else {
            1
        }
    }
}

impl SourceSpec {
    /// Loopback endpoints deliver nothing while nothing plays, so their track is padded.
    pub fn is_loopback(&self) -> bool {
        matches!(
            self,
            SourceSpec::DefaultOutput
                | SourceSpec::Output(_)
                | SourceSpec::Idle { .. }
                | SourceSpec::Tap { .. }
        )
    }

    /// Sources that move to the new default device; a synthetic one with `switch-at` plays a
    /// default device that changes.
    pub fn follows_default(&self) -> bool {
        match self {
            SourceSpec::DefaultInput | SourceSpec::DefaultOutput => true,
            SourceSpec::Tone { faults, .. }
            | SourceSpec::Idle { faults }
            | SourceSpec::Tap { faults, .. } => faults.switch_at.is_some(),
            _ => false,
        }
    }
}

impl FromStr for SourceSpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "default-input" => return Ok(SourceSpec::DefaultInput),
            "default-output" => return Ok(SourceSpec::DefaultOutput),
            _ => {}
        }
        if let Some(id) = s.strip_prefix("input:").filter(|id| !id.is_empty()) {
            return Ok(SourceSpec::Input(id.to_string()));
        }
        if let Some(id) = s.strip_prefix("output:").filter(|id| !id.is_empty()) {
            return Ok(SourceSpec::Output(id.to_string()));
        }
        if let Some(rest) = s.strip_prefix("file:") {
            let (path, looped) = match rest.strip_suffix(",loop") {
                Some(path) => (path, true),
                None => (rest, false),
            };
            if path.is_empty() {
                return Err(format!("source {s:?} names no file"));
            }
            return Ok(SourceSpec::File {
                path: PathBuf::from(path),
                looped,
            });
        }
        if let Some(rest) = s.strip_prefix("synthetic:") {
            let mut parts = rest.split(',');
            let kind = parts.next().unwrap_or_default();
            let mut faults = Faults::default();
            let (mut sound_at, mut sound_until, mut playing) = (None, None, false);
            for option in parts {
                if option == "playing" && kind == "tap" {
                    playing = true;
                    continue;
                }
                let (key, value) = option
                    .split_once('=')
                    .ok_or_else(|| format!("source {s:?}: expected key=value, got {option:?}"))?;
                let ms: u64 = value
                    .parse()
                    .map_err(|_| format!("source {s:?}: {key} needs milliseconds"))?;
                match key {
                    "fail-at" => faults.fail_at = Some(ms),
                    "back-at" => faults.back_at = Some(ms),
                    "switch-at" => faults.switch_at = Some(ms),
                    "open-ms" => faults.open_ms = Some(ms),
                    "skip-at" => faults.skip_at = Some(ms),
                    "skip-ms" => faults.skip_ms = Some(ms),
                    "sound-at" if kind == "tap" => sound_at = Some(ms),
                    "sound-until" if kind == "tap" => sound_until = Some(ms),
                    _ => return Err(format!("source {s:?}: unknown option {key:?}")),
                }
            }
            if let Some(hz) = kind.strip_prefix("tone:") {
                let hz: f32 = hz
                    .parse()
                    .ok()
                    .filter(|hz: &f32| hz.is_finite() && *hz > 0.0 && *hz < 24_000.0)
                    .ok_or_else(|| format!("source {s:?}: tone needs a frequency in Hz"))?;
                return Ok(SourceSpec::Tone { hz, faults });
            }
            if kind == "idle" {
                return Ok(SourceSpec::Idle { faults });
            }
            if kind == "tap" {
                if sound_until.is_some() && sound_at.is_none() {
                    return Err(format!("source {s:?}: sound-until needs sound-at"));
                }
                return Ok(SourceSpec::Tap {
                    faults,
                    sound: sound_at.map(|at| (at, sound_until)),
                    playing,
                });
            }
        }
        Err(format!(
            "unknown source {s:?}; use default-input, default-output, input:<id>, output:<id>, \
             synthetic:tone:<hz>, synthetic:idle, synthetic:tap or file:<path>"
        ))
    }
}

/// The device a track opened.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub id: String,
}

/// Why a running stream stopped delivering.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// The device is gone; retry until it, or a new default, is available.
    Lost(String),
    /// The stream must be rebuilt, usually because the default device changed.
    Rebuild(String),
}

/// State one open stream shares with the recorder thread.
#[derive(Default)]
pub struct StreamFlags {
    /// Set when a block was dropped because the recorder fell behind.
    pub overrun: AtomicBool,
    /// The first failure the stream reported, and when it reported it.
    pub failure: Mutex<Option<(Failure, Instant)>>,
}

impl StreamFlags {
    pub fn take_failure(&self) -> Option<(Failure, Instant)> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// Where a stream's callbacks send their audio and failures. Nothing here blocks a callback,
/// so the recorder can always drop a stream, whose callback thread it may have to wait for.
#[derive(Clone)]
pub struct Sink {
    tx: SyncSender<Event>,
    track: usize,
    generation: u64,
    flags: Arc<StreamFlags>,
}

impl Sink {
    pub(crate) fn new(
        tx: SyncSender<Event>,
        track: usize,
        generation: u64,
        flags: Arc<StreamFlags>,
    ) -> Self {
        Self {
            tx,
            track,
            generation,
            flags,
        }
    }

    /// When the recorder has fallen this far behind, the block is dropped and the track
    /// starts a new segment, so the loss can't hide inside one.
    pub fn audio(&self, data: Vec<f32>) {
        let event = Event::Audio {
            track: self.track,
            generation: self.generation,
            at: Instant::now(),
            data,
        };
        if let Err(TrySendError::Full(_)) = self.tx.try_send(event) {
            self.flags.overrun.store(true, Ordering::Relaxed);
        }
    }

    /// Records the failure where the recorder's next tick finds it, and wakes the recorder. The
    /// moment of the report is when the track lost its device, however late the recorder gets
    /// to it.
    pub fn failed(&self, failure: Failure) {
        let at = Instant::now();
        {
            let mut slot = self
                .flags
                .failure
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if slot.is_none() {
                *slot = Some((failure, at));
            }
        }
        let _ = self.tx.try_send(Event::Failed {
            track: self.track,
            generation: self.generation,
        });
    }
}

/// A running stream; dropping it stops the capture. Streams open on a thread of their own and
/// move to the recorder thread, so they are `Send`.
pub trait Capture: Send {}

pub struct Opened {
    pub capture: Box<dyn Capture>,
    pub device: DeviceInfo,
    pub rate: u32,
    pub channels: u16,
    /// The source records exact zeros, without an error, when the system doesn't let Voice record
    /// it: a macOS process tap without the System Audio Recording grant. The silence check
    /// watches such a track (`silence.rs`).
    pub silent_when_denied: bool,
}

/// Opens sources. Synthetic faults count from `started`, when the backend was ready; the
/// recorder starts the session's clock right after.
///
/// Opening a device can block for long: macOS holds the first open of a process tap until the
/// user answers the System Audio Recording prompt, and a Bluetooth headset can take seconds to
/// switch profile. The recorder therefore opens on a thread of its own and shares the backend.
pub struct Backend {
    started: Instant,
    #[cfg(any(windows, target_os = "macos"))]
    host: cpal::Host,
}

impl Backend {
    pub fn prepare() -> Self {
        #[cfg(any(windows, target_os = "macos"))]
        let host = {
            use cpal::traits::{DeviceTrait, HostTrait};
            let host = cpal::default_host();
            // The first device query in a process sets up COM and the device enumerator on
            // Windows and Core Audio's device list on macOS; do it before the meeting clock
            // starts instead of inside the first track's open. A Mac without a microphone has
            // nothing to ask.
            if !no_microphone() {
                if let Some(device) = host.default_input_device() {
                    let _ = device.default_input_config();
                }
            }
            host
        };
        Self {
            started: Instant::now(),
            #[cfg(any(windows, target_os = "macos"))]
            host,
        }
    }

    pub fn open(&self, spec: &SourceSpec, sink: Sink) -> Result<Opened, String> {
        match spec {
            SourceSpec::Tone { hz, faults } => {
                self.open_synthetic(Signal::Tone(*hz), *faults, spec, sink)
            }
            SourceSpec::Idle { faults } => self.open_synthetic(Signal::Idle, *faults, spec, sink),
            SourceSpec::Tap { faults, sound, .. } => {
                let mut opened = self.open_synthetic(Signal::Tap(*sound), *faults, spec, sink)?;
                opened.silent_when_denied = true;
                Ok(opened)
            }
            SourceSpec::File { path, looped } => {
                let wav = voice_audio::read_wav(path)
                    .map_err(|e| format!("can't read {}: {e}", path.display()))?;
                let signal = Signal::File {
                    channels: wav.channels,
                    samples: wav
                        .samples
                        .iter()
                        .map(|&s| voice_audio::i16_to_f32(s))
                        .collect(),
                    looped: *looped,
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(Synthetic::start(
                    signal,
                    wav.rate,
                    wav.channels,
                    Faults::default(),
                    self.started,
                    DeviceInfo {
                        name: format!("File {name}"),
                        id: format!("file:{}", path.display()),
                    },
                    sink,
                ))
            }
            _ => self.open_device(spec, sink),
        }
    }

    fn open_synthetic(
        &self,
        signal: Signal,
        faults: Faults,
        spec: &SourceSpec,
        sink: Sink,
    ) -> Result<Opened, String> {
        if let Some(ms) = faults.open_ms {
            std::thread::sleep(Duration::from_millis(ms));
        }
        let elapsed = self.started.elapsed().as_millis() as u64;
        if faults.unavailable(elapsed) {
            return Err("The synthetic device is unplugged.".into());
        }
        let number = faults.device_number(elapsed);
        let mut name = match &signal {
            Signal::Tone(hz) => format!("Synthetic {hz} Hz tone"),
            Signal::Tap(_) => "Synthetic process tap".into(),
            _ => "Synthetic idle output".into(),
        };
        if number > 1 {
            name.push_str(&format!(" {number}"));
        }
        Ok(Synthetic::start(
            signal,
            SYNTHETIC_RATE,
            SYNTHETIC_CHANNELS,
            faults,
            self.started,
            DeviceInfo {
                name,
                id: synthetic_id(spec, number),
            },
            sink,
        ))
    }

    /// The id of the current default device, for following it.
    pub fn default_device_id(&self, spec: &SourceSpec) -> Option<String> {
        match spec {
            SourceSpec::Tone { faults, .. }
            | SourceSpec::Idle { faults }
            | SourceSpec::Tap { faults, .. } => {
                let elapsed = self.started.elapsed().as_millis() as u64;
                faults
                    .switch_at
                    .map(|_| synthetic_id(spec, faults.device_number(elapsed)))
            }
            _ => self.default_cpal_id(spec),
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn default_cpal_id(&self, spec: &SourceSpec) -> Option<String> {
        use cpal::traits::{DeviceTrait, HostTrait};
        let device = match spec {
            SourceSpec::DefaultInput if no_microphone() => None,
            SourceSpec::DefaultInput => self.host.default_input_device(),
            SourceSpec::DefaultOutput => self.host.default_output_device(),
            _ => None,
        }?;
        device.id().ok().map(|id| id.to_string())
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn default_cpal_id(&self, _spec: &SourceSpec) -> Option<String> {
        None
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn open_device(&self, spec: &SourceSpec, sink: Sink) -> Result<Opened, String> {
        cpal_capture::open(&self.host, spec, sink)
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn open_device(&self, _spec: &SourceSpec, _sink: Sink) -> Result<Opened, String> {
        Err(UNSUPPORTED.into())
    }
}

/// Whether another app plays sound now, for a track whose source records silence when it isn't
/// allowed to record (`Opened::silent_when_denied`). `None` when nobody can tell.
pub fn others_playing(spec: &SourceSpec) -> Option<bool> {
    match spec {
        SourceSpec::Tap { playing: true, .. } => Some(true),
        SourceSpec::Tap { .. } => None,
        #[cfg(target_os = "macos")]
        SourceSpec::DefaultOutput | SourceSpec::Output(_) => crate::coreaudio::others_playing(),
        _ => None,
    }
}

/// The message for a computer without a microphone. Dictation says the same
/// (`voice-engine/src/dictation/capture.rs`).
pub const NO_MICROPHONE: &str = "No microphone is available.";

/// Whether this computer has no microphone at all. On a Mac, cpal 0.18.2 hands out a device for
/// Core Audio's `kAudioObjectUnknown` then, and every call on it fails.
pub fn no_microphone() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::coreaudio::no_microphone()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// How a loopback source on macOS records an output device. cpal turns an input stream on an
/// output device into a process tap only when the device has no inputs of its own; on a device
/// that has inputs, such as a USB headset or an audio interface, it opens those inputs instead,
/// which would put the user's microphone on the call track. The recorder makes the tap itself
/// then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapPath {
    /// cpal's own tap, for a device without inputs.
    Cpal,
    /// The recorder's tap (`coreaudio::Tap`), opened through cpal as an input device.
    Own,
}

pub fn tap_path(device_has_inputs: bool) -> TapPath {
    if device_has_inputs {
        TapPath::Own
    } else {
        TapPath::Cpal
    }
}

fn synthetic_id(spec: &SourceSpec, number: u32) -> String {
    if number > 1 {
        format!("{spec}#{number}")
    } else {
        spec.to_string()
    }
}

pub const UNSUPPORTED: &str =
    "Recording from audio devices is only supported on Windows and macOS in this version.";

/// Whether this build can open the source at all. Platform limits are refused at start, not
/// retried every second.
pub fn supported(spec: &SourceSpec) -> bool {
    cfg!(any(windows, target_os = "macos"))
        || matches!(
            spec,
            SourceSpec::Tone { .. }
                | SourceSpec::Idle { .. }
                | SourceSpec::Tap { .. }
                | SourceSpec::File { .. }
        )
}

impl fmt::Display for SourceSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn faults(f: &mut fmt::Formatter<'_>, faults: &Faults) -> fmt::Result {
            if let Some(ms) = faults.fail_at {
                write!(f, ",fail-at={ms}")?;
            }
            if let Some(ms) = faults.back_at {
                write!(f, ",back-at={ms}")?;
            }
            if let Some(ms) = faults.switch_at {
                write!(f, ",switch-at={ms}")?;
            }
            if let Some(ms) = faults.open_ms {
                write!(f, ",open-ms={ms}")?;
            }
            if let Some(ms) = faults.skip_at {
                write!(f, ",skip-at={ms}")?;
            }
            if let Some(ms) = faults.skip_ms {
                write!(f, ",skip-ms={ms}")?;
            }
            Ok(())
        }
        match self {
            SourceSpec::DefaultInput => f.write_str("default-input"),
            SourceSpec::DefaultOutput => f.write_str("default-output"),
            SourceSpec::Input(id) => write!(f, "input:{id}"),
            SourceSpec::Output(id) => write!(f, "output:{id}"),
            SourceSpec::Tone { hz, faults: x } => {
                write!(f, "synthetic:tone:{hz}")?;
                faults(f, x)
            }
            SourceSpec::Idle { faults: x } => {
                f.write_str("synthetic:idle")?;
                faults(f, x)
            }
            SourceSpec::Tap {
                faults: x,
                sound,
                playing,
            } => {
                f.write_str("synthetic:tap")?;
                faults(f, x)?;
                if let Some((at, until)) = sound {
                    write!(f, ",sound-at={at}")?;
                    if let Some(until) = until {
                        write!(f, ",sound-until={until}")?;
                    }
                }
                if *playing {
                    f.write_str(",playing")?;
                }
                Ok(())
            }
            SourceSpec::File { path, looped } => {
                write!(f, "file:{}", path.display())?;
                if *looped {
                    f.write_str(",loop")?;
                }
                Ok(())
            }
        }
    }
}

/// Synthetic devices run at a typical shared-mode format, so tests go through the resampler.
const SYNTHETIC_RATE: u32 = 48_000;
const SYNTHETIC_CHANNELS: u16 = 2;
/// Test sources deliver 10 ms blocks, like WASAPI's device period.
const BLOCK: Duration = Duration::from_millis(10);
const TONE_LEVEL: f32 = 0.5;

enum Signal {
    Tone(f32),
    Idle,
    /// Exact zeros, with a tone between the two times (ms since the recorder started).
    Tap(Option<(u64, Option<u64>)>),
    File {
        channels: u16,
        samples: Vec<f32>,
        looped: bool,
    },
}

/// A test source: a thread that delivers real-time paced blocks from the wall clock.
struct Synthetic {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Capture for Synthetic {}

impl Drop for Synthetic {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Synthetic {
    fn start(
        signal: Signal,
        rate: u32,
        channels: u16,
        faults: Faults,
        started: Instant,
        device: DeviceInfo,
        sink: Sink,
    ) -> Opened {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("synthetic-source".into())
                .spawn(move || run_synthetic(signal, rate, channels, faults, started, &stop, sink))
                .ok()
        };
        Opened {
            capture: Box::new(Synthetic { stop, thread }),
            device,
            rate,
            channels,
            silent_when_denied: false,
        }
    }
}

/// The tone a synthetic tap plays while it has sound.
const TAP_TONE_HZ: f32 = 600.0;

fn tone_block(hz: f32, produced: u64, frames: usize, rate: u32, channels: usize) -> Vec<f32> {
    let mut block = Vec::with_capacity(frames * channels);
    for i in 0..frames as u64 {
        let n = produced + i;
        // The phase from an exact integer position, so long runs don't drift.
        let cycles = (n as f64 * hz as f64 / rate as f64).fract();
        let v = TONE_LEVEL * (std::f64::consts::TAU * cycles).sin() as f32;
        block.extend(std::iter::repeat_n(v, channels));
    }
    block
}

fn run_synthetic(
    signal: Signal,
    rate: u32,
    channels: u16,
    faults: Faults,
    started: Instant,
    stop: &AtomicBool,
    sink: Sink,
) {
    let opened = Instant::now();
    let block_frames = (rate as u64 * BLOCK.as_millis() as u64 / 1000) as usize;
    let channels = channels as usize;
    let mut produced: u64 = 0;
    let mut position: usize = 0;
    while !stop.load(Ordering::Relaxed) {
        if faults.unavailable(started.elapsed().as_millis() as u64) {
            sink.failed(Failure::Lost("The synthetic device was unplugged.".into()));
            return;
        }
        let due = opened.elapsed().as_micros() as u64 * rate as u64 / 1_000_000;
        while produced + block_frames as u64 <= due {
            let block = match &signal {
                Signal::Idle => None,
                Signal::Tap(sound) => {
                    let now = started.elapsed().as_millis() as u64;
                    let sounding = sound.is_some_and(|(at, until)| {
                        now >= at && until.is_none_or(|until| now < until)
                    });
                    Some(if sounding {
                        tone_block(TAP_TONE_HZ, produced, block_frames, rate, channels)
                    } else {
                        vec![0.0; block_frames * channels]
                    })
                }
                Signal::Tone(hz) => Some(tone_block(*hz, produced, block_frames, rate, channels)),
                Signal::File {
                    channels: file_channels,
                    samples,
                    looped,
                } => {
                    let width = *file_channels as usize;
                    let mut block = vec![0.0f32; block_frames * width];
                    for frame in block.chunks_exact_mut(width) {
                        if position + width > samples.len() && *looped && !samples.is_empty() {
                            position = 0;
                        }
                        if position + width <= samples.len() {
                            frame.copy_from_slice(&samples[position..position + width]);
                            position += width;
                        }
                    }
                    Some(block)
                }
            };
            let skipping = faults.skipping(started.elapsed().as_millis() as u64);
            if let Some(block) = block.filter(|_| !skipping) {
                sink.audio(block);
            }
            produced += block_frames as u64;
        }
        std::thread::sleep(BLOCK / 2);
    }
}

#[cfg(any(windows, target_os = "macos"))]
mod cpal_capture {
    use super::{Capture, DeviceInfo, Failure, Opened, Sink, SourceSpec, NO_MICROPHONE};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{Data, ErrorKind, SampleFormat};

    /// The stream, then on macOS the tap it reads, if the recorder made one: fields drop in
    /// order, so the stream stops before its device goes.
    struct CpalStream {
        _stream: cpal::Stream,
        #[cfg(target_os = "macos")]
        _tap: Option<crate::coreaudio::Tap>,
    }

    impl Capture for CpalStream {}

    pub fn open(host: &cpal::Host, spec: &SourceSpec, sink: Sink) -> Result<Opened, String> {
        let loopback = spec.is_loopback();
        let device = match spec {
            SourceSpec::DefaultInput if super::no_microphone() => return Err(NO_MICROPHONE.into()),
            SourceSpec::DefaultInput => host.default_input_device().ok_or(NO_MICROPHONE)?,
            SourceSpec::DefaultOutput => host
                .default_output_device()
                .ok_or("No audio output device is available.")?,
            SourceSpec::Input(id) | SourceSpec::Output(id) => {
                let id: cpal::DeviceId = id.parse().map_err(|e| format!("bad device id: {e}"))?;
                host.device_by_id(&id)
                    .ok_or("The audio device is not connected.")?
            }
            _ => return Err("not a device source".into()),
        };
        // `to_string` on a device whose name can't be read panics in cpal 0.18.2 on macOS; the
        // id, then a plain word, stand in.
        let id = device.id().map(|id| id.to_string()).unwrap_or_default();
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .ok()
            .or_else(|| (!id.is_empty()).then(|| id.clone()))
            .unwrap_or_else(|| "unnamed device".into());
        // An input stream on an output device is WASAPI loopback on Windows and a Core Audio
        // process tap on macOS. Capture in the device's own format: cpal doesn't ask the system
        // to convert capture streams, so only native formats are reliable. Voice resamples
        // itself.
        let config = if loopback {
            device.default_output_config()
        } else {
            device.default_input_config()
        }
        .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        let (device, config, tap) = if loopback {
            let has_inputs = device
                .supported_input_configs()
                .is_ok_and(|mut configs| configs.next().is_some());
            match super::tap_path(has_inputs) {
                super::TapPath::Cpal => (device, config, None),
                super::TapPath::Own => own_tap(host, &id)?,
            }
        } else {
            (device, config, None)
        };
        let format = config.sample_format();
        if !decodable(format) {
            return Err(format!("unsupported sample format {format:?}"));
        }
        let data_sink = sink.clone();
        let on_data = move |data: &Data, _: &cpal::InputCallbackInfo| {
            if let Some(samples) = to_f32(data) {
                data_sink.audio(samples);
            }
        };
        let on_error = move |e: cpal::Error| match e.kind() {
            // Glitches and refused real-time priority don't stop the stream.
            ErrorKind::Xrun | ErrorKind::RealtimeDenied => {}
            // The default device changed, or on macOS its sample rate did; cpal doesn't reroute
            // capture streams, so rebuild.
            ErrorKind::StreamInvalidated | ErrorKind::DeviceChanged => {
                sink.failed(Failure::Rebuild(e.to_string()))
            }
            _ => sink.failed(Failure::Lost(e.to_string())),
        };
        let stream_config = config.config();
        let stream = device
            .build_input_stream_raw(stream_config, format, on_data, on_error, None)
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Opened {
            capture: Box::new(CpalStream {
                _stream: stream,
                #[cfg(target_os = "macos")]
                _tap: tap,
            }),
            device: DeviceInfo { name, id },
            rate: config.sample_rate(),
            channels: config.channels(),
            // A tap without the System Audio Recording grant records exact zeros (spike S2).
            silent_when_denied: cfg!(target_os = "macos") && loopback,
        })
    }

    /// Taps an output device that has inputs of its own, and finds the tap's private aggregate
    /// among this process's devices to open it as an input. The track keeps the output device's
    /// name and id, so following the default and the timeline see the device the user chose.
    #[cfg(target_os = "macos")]
    fn own_tap(
        host: &cpal::Host,
        output_id: &str,
    ) -> Result<
        (
            cpal::Device,
            cpal::SupportedStreamConfig,
            Option<crate::coreaudio::Tap>,
        ),
        String,
    > {
        let uid = output_id
            .strip_prefix("coreaudio:")
            .ok_or("The output device has no Core Audio id.")?;
        let tap = crate::coreaudio::Tap::of_output(uid)?;
        let aggregate_id = format!("coreaudio:{}", tap.uid);
        let aggregate = host
            .devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.id().is_ok_and(|id| id.to_string() == aggregate_id))
            .ok_or("macOS made the device for the process tap, but it isn't in the device list.")?;
        let config = aggregate
            .default_input_config()
            .map_err(|e| e.to_string())?;
        Ok((aggregate, config, Some(tap)))
    }

    fn decodable(format: SampleFormat) -> bool {
        matches!(
            format,
            SampleFormat::F32
                | SampleFormat::F64
                | SampleFormat::I16
                | SampleFormat::I32
                | SampleFormat::I8
                | SampleFormat::U16
                | SampleFormat::U8
        )
    }

    fn to_f32(data: &Data) -> Option<Vec<f32>> {
        Some(match data.sample_format() {
            SampleFormat::F32 => data.as_slice::<f32>()?.to_vec(),
            SampleFormat::F64 => data.as_slice::<f64>()?.iter().map(|&s| s as f32).collect(),
            SampleFormat::I16 => data
                .as_slice::<i16>()?
                .iter()
                .map(|&s| s as f32 / 32_768.0)
                .collect(),
            SampleFormat::I32 => data
                .as_slice::<i32>()?
                .iter()
                .map(|&s| s as f32 / 2_147_483_648.0)
                .collect(),
            SampleFormat::I8 => data
                .as_slice::<i8>()?
                .iter()
                .map(|&s| s as f32 / 128.0)
                .collect(),
            SampleFormat::U16 => data
                .as_slice::<u16>()?
                .iter()
                .map(|&s| (s as f32 - 32_768.0) / 32_768.0)
                .collect(),
            SampleFormat::U8 => data
                .as_slice::<u8>()?
                .iter()
                .map(|&s| (s as f32 - 128.0) / 128.0)
                .collect(),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_source_kind() {
        let cases = [
            ("default-input", SourceSpec::DefaultInput),
            ("default-output", SourceSpec::DefaultOutput),
            (
                "input:wasapi:{0.0.1.00000000}.{e113}",
                SourceSpec::Input("wasapi:{0.0.1.00000000}.{e113}".into()),
            ),
            ("output:wasapi:x", SourceSpec::Output("wasapi:x".into())),
            (
                "synthetic:tone:440",
                SourceSpec::Tone {
                    hz: 440.0,
                    faults: Faults::default(),
                },
            ),
            (
                "synthetic:tone:440,fail-at=2000,back-at=4000",
                SourceSpec::Tone {
                    hz: 440.0,
                    faults: Faults {
                        fail_at: Some(2000),
                        back_at: Some(4000),
                        ..Faults::default()
                    },
                },
            ),
            (
                "synthetic:idle,fail-at=10,switch-at=30,open-ms=5000",
                SourceSpec::Idle {
                    faults: Faults {
                        fail_at: Some(10),
                        switch_at: Some(30),
                        open_ms: Some(5000),
                        ..Faults::default()
                    },
                },
            ),
            (
                "synthetic:tap",
                SourceSpec::Tap {
                    faults: Faults::default(),
                    sound: None,
                    playing: false,
                },
            ),
            (
                "synthetic:tap,open-ms=7000,skip-at=800,skip-ms=300,sound-at=0,sound-until=500,playing",
                SourceSpec::Tap {
                    faults: Faults {
                        open_ms: Some(7000),
                        skip_at: Some(800),
                        skip_ms: Some(300),
                        ..Faults::default()
                    },
                    sound: Some((0, Some(500))),
                    playing: true,
                },
            ),
            (
                r"file:C:\audio\ami.wav",
                SourceSpec::File {
                    path: PathBuf::from(r"C:\audio\ami.wav"),
                    looped: false,
                },
            ),
            (
                "file:/tmp/a,b.wav,loop",
                SourceSpec::File {
                    path: PathBuf::from("/tmp/a,b.wav"),
                    looped: true,
                },
            ),
        ];
        for (text, spec) in cases {
            assert_eq!(text.parse::<SourceSpec>().unwrap(), spec, "{text}");
            assert_eq!(spec.to_string(), text);
        }
    }

    #[test]
    fn refuses_unknown_or_broken_sources() {
        for text in [
            "",
            "mic",
            "input:",
            "file:",
            "synthetic:tone:",
            "synthetic:tone:-5",
            "synthetic:tone:440,fail-at=soon",
            "synthetic:tone:440,explode=1",
            "synthetic:noise",
            "synthetic:tone:440,playing",
            "synthetic:idle,sound-at=5",
            "synthetic:tap,sound-until=5",
        ] {
            assert!(text.parse::<SourceSpec>().is_err(), "{text}");
        }
    }

    #[test]
    fn only_loopback_sources_are_padded() {
        assert!(SourceSpec::DefaultOutput.is_loopback());
        assert!(SourceSpec::Output("x".into()).is_loopback());
        assert!(SourceSpec::Idle {
            faults: Faults::default()
        }
        .is_loopback());
        assert!(!SourceSpec::DefaultInput.is_loopback());
        assert!(!SourceSpec::Tone {
            hz: 1.0,
            faults: Faults::default()
        }
        .is_loopback());
    }

    #[test]
    fn a_stream_keeps_its_first_failure_and_when_it_came() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let flags = Arc::new(StreamFlags::default());
        let sink = Sink::new(tx, 0, 1, flags.clone());
        let before = Instant::now();
        sink.failed(Failure::Lost("gone".into()));
        let after = Instant::now();
        sink.failed(Failure::Rebuild("moved".into()));

        let (failure, at) = flags.take_failure().unwrap();
        assert_eq!(failure, Failure::Lost("gone".into()));
        assert!(before <= at && at <= after);
        assert!(flags.take_failure().is_none());
        assert!(matches!(
            rx.try_recv(),
            Ok(Event::Failed {
                track: 0,
                generation: 1
            })
        ));
    }

    #[test]
    fn a_process_tap_is_a_padded_loopback_that_follows_the_default() {
        let tap: SourceSpec = "synthetic:tap,switch-at=10".parse().unwrap();
        assert!(tap.is_loopback() && tap.follows_default());
        assert!(SourceSpec::DefaultOutput.is_loopback());
    }

    /// cpal records an output device's own inputs when it has any, which would put the user's
    /// microphone on the call track; the recorder taps such a device itself.
    #[test]
    fn an_output_device_with_inputs_gets_the_recorders_own_tap() {
        assert_eq!(tap_path(false), TapPath::Cpal);
        assert_eq!(tap_path(true), TapPath::Own);
    }

    #[test]
    fn only_a_tap_says_whether_other_apps_play() {
        let playing: SourceSpec = "synthetic:tap,playing".parse().unwrap();
        assert_eq!(others_playing(&playing), Some(true));
        let unknown: SourceSpec = "synthetic:tap".parse().unwrap();
        assert_eq!(others_playing(&unknown), None);
        assert_eq!(others_playing(&SourceSpec::DefaultInput), None);
    }

    #[test]
    fn a_skip_delivers_nothing_for_its_length() {
        let f = Faults {
            skip_at: Some(800),
            skip_ms: Some(300),
            ..Faults::default()
        };
        assert!(!f.skipping(799));
        assert!(f.skipping(800) && f.skipping(1_099));
        assert!(!f.skipping(1_100));
        assert!(!f.unavailable(900), "a skip isn't a failure");
    }

    #[test]
    fn faults_cover_the_window_between_fail_and_back() {
        let f = Faults {
            fail_at: Some(2000),
            back_at: Some(4000),
            ..Faults::default()
        };
        assert!(!f.unavailable(1999));
        assert!(f.unavailable(2000));
        assert!(f.unavailable(3999));
        assert!(!f.unavailable(4000));
        let forever = Faults {
            fail_at: Some(10),
            ..Faults::default()
        };
        assert!(forever.unavailable(1_000_000));
        assert!(!Faults::default().unavailable(5));
    }

    #[test]
    fn a_switching_synthetic_device_follows_the_default() {
        let switching: SourceSpec = "synthetic:tone:440,switch-at=1500".parse().unwrap();
        assert!(switching.follows_default());
        assert!(!"synthetic:tone:440"
            .parse::<SourceSpec>()
            .unwrap()
            .follows_default());
        let SourceSpec::Tone { faults, .. } = switching else {
            unreachable!()
        };
        assert_eq!(faults.device_number(1499), 1);
        assert_eq!(faults.device_number(1500), 2);
        assert_eq!(
            synthetic_id(&switching, 1),
            "synthetic:tone:440,switch-at=1500"
        );
        assert_eq!(
            synthetic_id(&switching, 2),
            "synthetic:tone:440,switch-at=1500#2"
        );
    }
}
