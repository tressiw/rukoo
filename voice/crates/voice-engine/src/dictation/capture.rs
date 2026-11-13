//! Dictation's microphone outside a meeting: the default input device through cpal, converted
//! to 16 kHz mono. Opened at every press and closed at every release, as Ownkey did, so a new
//! default microphone applies from the next dictation (ownkey-dictation.md 4.3).

use crate::services::{AudioSink, MicLost, MicStream, Microphone};

/// What the user sees when there is no microphone to open. cpal's missing device and Core
/// Audio's `kAudioObjectUnknown` read the same, so a Mac and a PC say it the same way.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) const NO_MICROPHONE: &str = "No microphone is available.";

pub struct DefaultMicrophone;

impl Microphone for DefaultMicrophone {
    fn open(&self, on_audio: AudioSink) -> Result<Box<dyn MicStream>, String> {
        self.open_watched(on_audio, Box::new(|| {}))
    }

    fn open_watched(
        &self,
        on_audio: AudioSink,
        on_lost: MicLost,
    ) -> Result<Box<dyn MicStream>, String> {
        #[cfg(any(windows, target_os = "macos"))]
        {
            cpal_mic::open(on_audio, on_lost).map(|mic| Box::new(mic) as Box<dyn MicStream>)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (on_audio, on_lost);
            Err("Dictation isn't supported on this platform yet.".into())
        }
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

    use super::NO_MICROPHONE;
    use crate::services::{AudioSink, MicLost, MicStream};

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

    pub fn open(on_audio: AudioSink, on_lost: MicLost) -> Result<CpalMic, String> {
        let (opened_tx, opened_rx) = mpsc::channel::<Result<(), String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("dictation-mic".into())
            .spawn(move || {
                let stream = match build(on_audio, on_lost) {
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

    fn build(mut on_audio: AudioSink, on_lost: MicLost) -> Result<cpal::Stream, String> {
        // A Mac with no input device still gets a device from cpal; see `mac_microphone`.
        #[cfg(target_os = "macos")]
        if super::mac_microphone::is_missing(super::mac_microphone::default_input_id()) {
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
        Ok(stream)
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

/// Core Audio's default input device on macOS, which cpal can't tell Voice about.
#[cfg(target_os = "macos")]
mod mac_microphone {
    use std::ptr::NonNull;

    use objc2_core_audio::{
        kAudioHardwareNoError, kAudioHardwarePropertyDefaultInputDevice,
        kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
        AudioObjectGetPropertyData, AudioObjectPropertyAddress,
    };

    /// `kAudioObjectUnknown`: what Core Audio reports when the Mac has no default input device.
    const UNKNOWN_AUDIO_DEVICE: u32 = 0;

    /// The id Core Audio has for the default input device, or `None` when the HAL didn't answer.
    pub(super) fn default_input_id() -> Option<u32> {
        let address = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut device: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the address is valid and the HAL writes exactly one device id into `device`,
        // whose size is passed along.
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject as u32,
                NonNull::from(&address),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::from(&mut device).cast(),
            )
        };
        (status == kAudioHardwareNoError).then_some(device)
    }

    /// Whether that id means "this Mac has no input device".
    ///
    /// cpal 0.18.2's Core Audio backend keeps the unknown id and hands out a device for it; its
    /// name, its id and its format then all fail, and calling `to_string` on the device can
    /// panic. Voice reads the id itself and treats it as no microphone, so dictation
    /// says so instead of failing inside cpal.
    pub(super) fn is_missing(device: Option<u32>) -> bool {
        // A HAL that didn't answer leaves cpal's own `None` to report the missing microphone.
        device == Some(UNKNOWN_AUDIO_DEVICE)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::mac_microphone::{default_input_id, is_missing};
    use super::{DefaultMicrophone, NO_MICROPHONE};
    use crate::services::{AudioSink, Microphone};

    #[test]
    fn an_unknown_audio_device_is_no_microphone() {
        assert!(is_missing(Some(0)), "kAudioObjectUnknown");
        assert!(!is_missing(Some(1)));
        assert!(!is_missing(Some(0x4E)));
        assert!(
            !is_missing(None),
            "a HAL that can't answer is cpal's own `None`, not this check"
        );
    }

    #[test]
    fn the_missing_microphone_reads_as_a_message_for_the_user() {
        assert_eq!(NO_MICROPHONE, "No microphone is available.");
    }

    /// This Mac has no input device, which is the case the check exists for. The read opens
    /// nothing and asks for no permission.
    #[test]
    #[ignore = "needs the real machine: checks this Mac's own input devices"]
    fn this_mac_reports_no_microphone() {
        assert!(
            is_missing(default_input_id()),
            "Core Audio answered {:?}",
            default_input_id()
        );
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
