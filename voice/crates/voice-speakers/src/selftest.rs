//! `--self-test`: labels a WAV file once, the way the app's follower feeds a stream, and reports
//! what came out. The runtime build workflow runs it as its smoke test on the AMI excerpt.

use std::ffi::OsString;
use std::path::PathBuf;

use serde::Serialize;

/// How the report reads, one JSON line on stdout and optionally in a file.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Distinct speakers in the runtime's own segments.
    pub speakers: usize,
    /// Frames the stream returned over all pushes and the finish.
    pub frames: u64,
    pub segments: usize,
    pub load_ms: u64,
    /// Seconds of audio per second of pushing and finishing, without the load.
    pub realtime_factor: f64,
}

#[derive(Debug, PartialEq)]
pub struct Options {
    pub runtime: PathBuf,
    pub model: PathBuf,
    pub wav: PathBuf,
    pub report: Option<PathBuf>,
}

pub const USAGE: &str =
    "rukoo-voice-speakers --self-test --runtime <dir> --model <gguf> --wav <16 kHz mono wav> [--report <json>]";

pub fn parse(args: &[OsString]) -> Result<Options, String> {
    let (mut runtime, mut model, mut wav, mut report) = (None, None, None, None);
    let mut args = args.iter();
    while let Some(name) = args.next() {
        let slot = match name.to_str() {
            Some("--runtime") => &mut runtime,
            Some("--model") => &mut model,
            Some("--wav") => &mut wav,
            Some("--report") => &mut report,
            _ => return Err(format!("unknown argument {}", name.to_string_lossy())),
        };
        let Some(value) = args.next() else {
            return Err(format!("{} needs a value", name.to_string_lossy()));
        };
        *slot = Some(PathBuf::from(value));
    }
    let need =
        |value: Option<PathBuf>, name: &str| value.ok_or_else(|| format!("{name} is missing"));
    Ok(Options {
        runtime: need(runtime, "--runtime")?,
        model: need(model, "--model")?,
        wav: need(wav, "--wav")?,
        report,
    })
}

/// The samples of a 16 kHz mono 16-bit PCM WAV file. Chunks other than `fmt ` and `data` are
/// skipped; a `data` chunk that claims more than the file holds is read up to the end.
pub fn read_wav(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let mut format = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(at + 4) as usize;
        let body = at + 8;
        if id == b"fmt " {
            if size < 16 || body + 16 > bytes.len() {
                return Err("the fmt chunk is too short".into());
            }
            format = Some((
                u16_at(body),
                u16_at(body + 2),
                u32_at(body + 4),
                u16_at(body + 14),
            ));
        } else if id == b"data" {
            let Some((tag, channels, rate, bits)) = format else {
                return Err("the data chunk comes before the fmt chunk".into());
            };
            if tag != 1 || channels != 1 || rate != 16_000 || bits != 16 {
                return Err(format!(
                    "the audio must be 16 kHz mono 16-bit PCM, not format {tag} with {channels} channels at {rate} Hz and {bits} bits"
                ));
            }
            let end = body.saturating_add(size).min(bytes.len());
            return Ok(bytes[body..end]
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect());
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    Err("the file has no data chunk".into())
}

/// Loads the runtime and the model, pushes the audio in 1 s pieces, finishes and counts.
#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
pub fn run(options: &Options) -> Result<Report, String> {
    use std::time::Instant;

    use crate::backend::{CHUNK_FRAMES, PRESET, RIGHT_CONTEXT_FRAMES, SAMPLE_RATE};
    use crate::nemo::{Model, Runtime, Stream};

    let bytes = std::fs::read(&options.wav)
        .map_err(|error| format!("could not read {}: {error}", options.wav.display()))?;
    let pcm = read_wav(&bytes).map_err(|error| format!("{}: {error}", options.wav.display()))?;
    let failed = |failure: crate::protocol::Failure| failure.message;

    let started = Instant::now();
    let runtime = Runtime::load(&options.runtime, &[]).map_err(failed)?;
    let model = Model::create(
        runtime,
        &options.model,
        PRESET,
        CHUNK_FRAMES,
        RIGHT_CONTEXT_FRAMES,
    )
    .map_err(failed)?;
    let load_ms = started.elapsed().as_millis() as u64;
    let model: &'static Model = Box::leak(Box::new(model));
    let speakers = model.speakers();

    let started = Instant::now();
    let mut stream = Stream::open(model).map_err(failed)?;
    let mut frames = 0u64;
    let mut expected_first = 0u64;
    let mut take = |got: crate::backend::Frames| -> Result<(), String> {
        if got.first != expected_first {
            return Err(format!(
                "frames resumed at {} instead of {expected_first}",
                got.first
            ));
        }
        let new = (got.values.len() / speakers) as u64;
        frames += new;
        expected_first = got.first + new;
        Ok(())
    };
    for piece in pcm.chunks(SAMPLE_RATE as usize) {
        take(stream.push(piece).map_err(failed)?)?;
    }
    take(stream.finish().map_err(failed)?)?;
    let elapsed = started.elapsed().as_secs_f64();
    let segments = stream.segments().map_err(failed)?;

    let mut distinct: Vec<i32> = segments.iter().map(|segment| segment.speaker).collect();
    distinct.sort_unstable();
    distinct.dedup();
    let seconds = pcm.len() as f64 / f64::from(SAMPLE_RATE);
    let realtime_factor = if elapsed > 0.0 {
        seconds / elapsed
    } else {
        0.0
    };
    Ok(Report {
        speakers: distinct.len(),
        frames,
        segments: segments.len(),
        load_ms,
        realtime_factor: (realtime_factor * 100.0).round() / 100.0,
    })
}

#[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
pub fn run(options: &Options) -> Result<Report, String> {
    let bytes = std::fs::read(&options.wav)
        .map_err(|error| format!("could not read {}: {error}", options.wav.display()))?;
    read_wav(&bytes).map_err(|error| format!("{}: {error}", options.wav.display()))?;
    Err("speaker labels run on Windows on x86-64 and macOS on Apple Silicon only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, rate: u32, samples: &[i16], extra_chunk: bool) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2 * u32::from(channels)).to_le_bytes());
        out.extend_from_slice(&(2 * channels).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        if extra_chunk {
            out.extend_from_slice(b"LIST");
            out.extend_from_slice(&3u32.to_le_bytes());
            out.extend_from_slice(b"abc\0");
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn reads_the_canonical_header_and_skips_other_chunks() {
        let canonical = wav(1, 16_000, &[1, -2, 32767], false);
        assert_eq!(canonical.len(), 44 + 6);
        assert_eq!(read_wav(&canonical).unwrap(), [1, -2, 32767]);
        assert_eq!(read_wav(&wav(1, 16_000, &[5], true)).unwrap(), [5]);
    }

    #[test]
    fn refuses_other_formats() {
        assert!(read_wav(&wav(2, 16_000, &[0, 0], false))
            .unwrap_err()
            .contains("2 channels"));
        assert!(read_wav(&wav(1, 48_000, &[0], false))
            .unwrap_err()
            .contains("48000 Hz"));
        assert!(read_wav(b"not a wav file").is_err());
    }

    #[test]
    fn a_data_chunk_longer_than_the_file_is_read_to_the_end() {
        let mut file = wav(1, 16_000, &[7, 8], false);
        let len = file.len();
        file[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read_wav(&file[..len - 1]).unwrap(), [7]);
    }

    #[test]
    fn parses_the_options() {
        let args: Vec<OsString> = ["--runtime", "rt", "--model", "m.gguf", "--wav", "a.wav"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(
            parse(&args).unwrap(),
            Options {
                runtime: "rt".into(),
                model: "m.gguf".into(),
                wav: "a.wav".into(),
                report: None
            }
        );
        assert!(parse(&args[..4]).unwrap_err().contains("--wav"));
        assert!(parse(&[OsString::from("--runtime")])
            .unwrap_err()
            .contains("needs a value"));
        assert!(parse(&[OsString::from("--bogus")]).is_err());
    }
}
