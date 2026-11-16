//! Dictation's microphone outside a meeting: the default input device through cpal, converted
//! to 16 kHz mono. Opened at every press and closed at every release, as Ownkey did, so a new
//! default microphone applies from the next dictation (ownkey-dictation.md 4.3).

use crate::services::{AudioSink, MicLost, MicOpen, MicStream, Microphone};

/// What the user sees when there is no microphone to open. cpal's missing device and Core
/// Audio's `kAudioObjectUnknown` read the same, so a Mac and a PC say it the same way, and a
/// meeting's microphone track says it too.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) const NO_MICROPHONE: &str = voice_recorder::source::NO_MICROPHONE;

/// What one open of a dictation's capture found: the device, and the format its stream delivers.
#[cfg(any(windows, target_os = "macos", test))]
pub(crate) struct OpenedInput<'a> {
    pub name: &'a str,
    pub rate: u32,
    pub channels: u16,
    /// The sample format's name, as cpal writes it: `f32`, `i16` and so on.
    pub format: &'a str,
}

/// The one line a dictation's capture logs when it opens an input stream: the device it opened,
/// the format the stream runs in and the rate it converts to, under the dictation's id, so an
/// empty transcript can be read against what the microphone delivered and at which rate. The
/// device's name is the one the person chose and stays in `app.log` in the data folder, as the
/// recorder's journal names the microphone of a meeting.
#[cfg(any(windows, target_os = "macos", test))]
fn open_line(about: MicOpen, input: OpenedInput) -> String {
    let which = match about.reopen {
        0 => "first".to_string(),
        reopen => format!("reopen {reopen}"),
    };
    let channels = match input.channels {
        1 => "1 channel".to_string(),
        channels => format!("{channels} channels"),
    };
    format!(
        "dictation {} capture open ({which}): {:?} at {} Hz, {channels}, {}, resampled to {} Hz",
        about.dictation,
        input.name,
        input.rate,
        input.format,
        voice_audio::SAMPLE_RATE
    )
}

pub struct DefaultMicrophone;

impl Microphone for DefaultMicrophone {
    fn open(&self, on_audio: AudioSink) -> Result<Box<dyn MicStream>, String> {
        open_capture(on_audio, Box::new(|| {}), None)
    }

    fn open_watched(
        &self,
        on_audio: AudioSink,
        on_lost: MicLost,
        about: MicOpen,
    ) -> Result<Box<dyn MicStream>, String> {
        open_capture(on_audio, on_lost, Some(about))
    }
}

/// Opens the default microphone, logging what it opened when a dictation asked for it. The plain
/// [`Microphone::open`] logs nothing: only dictation's opens carry a dictation to report.
fn open_capture(
    on_audio: AudioSink,
    on_lost: MicLost,
    about: Option<MicOpen>,
) -> Result<Box<dyn MicStream>, String> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        cpal_mic::open(on_audio, on_lost, about).map(|mic| Box::new(mic) as Box<dyn MicStream>)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (on_audio, on_lost, about);
        Err("Dictation isn't supported on this platform yet.".into())
    }
}

/// Whether a stream error means the stream delivers nothing more: the device went away, or
/// changed under it. cpal 0.18.2 pauses a Core Audio stream for both, a sample rate change
/// included. Anything else passes: an overrun, or a default device that changed and that cpal
/// followed on its own.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn ends_stream(kind: cpal::ErrorKind) -> bool {
    matches!(
        kind,
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated
    )
}

#[cfg(any(windows, target_os = "macos"))]
mod cpal_mic {
    use std::sync::{mpsc, Arc, Mutex, PoisonError};
    use std::thread::{self, JoinHandle};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{Data, SampleFormat};
    use voice_audio::Resampler;

    use super::{open_line, OpenedInput, NO_MICROPHONE};
    use crate::services::{AudioSink, MicLost, MicOpen, MicStream};

    /// The stream lives on its own thread, which drops it when the microphone closes.
    pub struct CpalMic {
        stop: Option<mpsc::Sender<()>>,
        thread: Option<JoinHandle<()>>,
    }

    impl MicStream for CpalMic {}

    impl Drop for CpalMic {
        fn drop(&mut self) {
            drop(self.stop.take());
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub fn open(
        on_audio: AudioSink,
        on_lost: MicLost,
        about: Option<MicOpen>,
    ) -> Result<CpalMic, String> {
        let (opened_tx, opened_rx) = mpsc::channel::<Result<(), String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("dictation-mic".into())
            .spawn(move || {
                let stream = match build(on_audio, on_lost, about) {
                    Ok(stream) => stream,
                    Err(e) => {
                        let _ = opened_tx.send(Err(e));
                        return;
                    }
                };
                let _ = opened_tx.send(Ok(()));
                // Until the release drops the sender.
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| format!("Couldn't start the microphone: {e}"))?;
        match opened_rx.recv() {
            Ok(Ok(())) => Ok(CpalMic {
                stop: Some(stop_tx),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("The microphone stopped while it opened.".into())
            }
        }
    }

    fn build(
        mut on_audio: AudioSink,
        on_lost: MicLost,
        about: Option<MicOpen>,
    ) -> Result<cpal::Stream, String> {
        // A Mac with no input device still gets a device from cpal 0.18.2, for Core Audio's
        // `kAudioObjectUnknown`; the recorder's check is the one meetings use.
        if voice_recorder::source::no_microphone() {
            return Err(NO_MICROPHONE.into());
        }
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(NO_MICROPHONE)?;
        // The device's own format: cpal doesn't ask Windows to convert capture streams.
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let format = config.sample_format();
        let mut resampler =
            Resampler::new(config.sample_rate(), config.channels()).ok_or_else(|| {
                format!(
                    "The microphone's format ({} Hz) isn't supported.",
                    config.sample_rate()
                )
            })?;
        let on_data = move |data: &Data, _: &cpal::InputCallbackInfo| {
            if let Some(samples) = to_f32(data) {
                let pcm = resampler.process(&samples);
                if !pcm.is_empty() {
                    on_audio(&pcm);
                }
            }
        };
        // A lost or changed device stops delivering. The dictation hears about it once, on a
        // thread of its own: it replaces this stream, which joins the thread that holds it.
        let on_lost = Arc::new(Mutex::new(Some(on_lost)));
        let on_error = move |error: cpal::Error| {
            if !super::ends_stream(error.kind()) {
                return;
            }
            let taken = on_lost
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(on_lost) = taken {
                let _ = thread::Builder::new()
                    .name("dictation-mic-lost".into())
                    .spawn(on_lost);
            }
        };
        let stream = device
            .build_input_stream_raw(config.config(), format, on_data, on_error, None)
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        // Only after the stream plays, so a failed open never reads as one that delivered nothing.
        if let Some(about) = about {
            let name = device_name(&device);
            let line = open_line(
                about,
                OpenedInput {
                    name: &name,
                    rate: config.sample_rate(),
                    channels: config.channels(),
                    format: &format.to_string(),
                },
            );
            log::info!("{line}");
        }
        Ok(stream)
    }

    /// The device's name for the log. Reading a name can fail, and `to_string` on a device whose
    /// name can't be read panics in cpal 0.18.2 on macOS; the id, then a plain word, stand in, as
    /// in the recorder's own device lookup.
    fn device_name(device: &cpal::Device) -> String {
        let id = device.id().map(|id| id.to_string()).unwrap_or_default();
        device
            .description()
            .map(|d| d.name().to_string())
            .ok()
            .or_else(|| (!id.is_empty()).then(|| id.clone()))
            .unwrap_or_else(|| "unnamed device".into())
    }

    fn to_f32(data: &Data) -> Option<Vec<f32>> {
        Some(match data.sample_format() {
            SampleFormat::F32 => data.as_slice::<f32>()?.to_vec(),
            SampleFormat::F64 => data.as_slice::<f64>()?.iter().map(|&s| s as f32).collect(),
            SampleFormat::I16 => data
                .as_slice::<i16>()?
                .iter()
                .map(|&s| f32::from(s) / 32_768.0)
                .collect(),
            SampleFormat::I32 => data
                .as_slice::<i32>()?
                .iter()
                .map(|&s| s as f32 / 2_147_483_648.0)
                .collect(),
            SampleFormat::U16 => data
                .as_slice::<u16>()?
                .iter()
                .map(|&s| (f32::from(s) - 32_768.0) / 32_768.0)
                .collect(),
            SampleFormat::U8 => data
                .as_slice::<u8>()?
                .iter()
                .map(|&s| (f32::from(s) - 128.0) / 128.0)
                .collect(),
            SampleFormat::I8 => data
                .as_slice::<i8>()?
                .iter()
                .map(|&s| f32::from(s) / 128.0)
                .collect(),
            _ => return None,
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{DefaultMicrophone, NO_MICROPHONE};
    use crate::services::{AudioSink, Microphone};

    #[test]
    fn the_missing_microphone_reads_as_a_message_for_the_user() {
        assert_eq!(NO_MICROPHONE, "No microphone is available.");
    }

    /// Opens the real microphone, so it needs one and a desktop nobody is using.
    #[test]
    #[ignore = "needs a microphone: opens and closes the real input device"]
    fn opens_the_real_microphone() {
        let seen = Arc::new(Mutex::new(0usize));
        let counter = Arc::clone(&seen);
        let sink: AudioSink = Box::new(move |pcm: &[i16]| {
            *counter.lock().unwrap() += pcm.len();
        });
        let mic = DefaultMicrophone
            .open(sink)
            .expect("a microphone on this machine");
        std::thread::sleep(Duration::from_millis(300));
        drop(mic);
        assert!(
            *seen.lock().unwrap() > 0,
            "the microphone delivered no samples"
        );
    }
}

#[cfg(test)]
mod opened_lines {
    use super::{open_line, OpenedInput};
    use crate::services::MicOpen;

    /// A first open says the device, its native format and the rate the capture converts to, so an
    /// empty transcript can be read against what the microphone delivered.
    #[test]
    fn the_first_open_names_the_device_and_its_format() {
        let line = open_line(
            MicOpen::first(7),
            OpenedInput {
                name: "Sony WH-1000XM3",
                rate: 48_000,
                channels: 1,
                format: "f32",
            },
        );
        assert_eq!(
            line,
            "dictation 7 capture open (first): \"Sony WH-1000XM3\" at 48000 Hz, 1 channel, f32, resampled to 16000 Hz"
        );
    }

    /// A stream that stopped is opened again under the same dictation, numbered, so the rate a
    /// Bluetooth headset switched to reads next to the one before it.
    #[test]
    fn a_reopen_carries_its_number() {
        let line = open_line(
            MicOpen::reopen(7, 2),
            OpenedInput {
                name: "MacBook Pro Microphone",
                rate: 44_100,
                channels: 2,
                format: "i16",
            },
        );
        assert_eq!(
            line,
            "dictation 7 capture open (reopen 2): \"MacBook Pro Microphone\" at 44100 Hz, 2 channels, i16, resampled to 16000 Hz"
        );
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod stream_errors {
    use super::ends_stream;

    #[test]
    fn only_a_device_that_went_or_changed_ends_the_stream() {
        assert!(ends_stream(cpal::ErrorKind::DeviceNotAvailable));
        assert!(ends_stream(cpal::ErrorKind::StreamInvalidated));
        assert!(!ends_stream(cpal::ErrorKind::BackendError));
        assert!(!ends_stream(cpal::ErrorKind::Xrun));
        assert!(
            !ends_stream(cpal::ErrorKind::DeviceChanged),
            "cpal rerouted the stream itself"
        );
    }
}
