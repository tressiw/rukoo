//! The Rukoo Voice meeting recorder: captures the microphone and the call audio into the
//! meeting folder described in `voice/RECORDING.md`, and nothing else.
//!
//! The binary `rukoo-voice-recorder` runs as its own process, so transcription, the app and
//! Rukoo can all crash while the recording goes on.

pub mod clock;
pub mod control;
#[cfg(target_os = "macos")]
mod coreaudio;
pub mod devices;
pub mod meeting;
pub mod record;
pub mod silence;
pub mod source;
pub mod timeline;
mod token;

use std::fmt;
use std::path::PathBuf;

pub use record::{record, RecordOptions, DEFAULT_LIMIT_MS, DEFAULT_SEGMENT_MS};

/// The recorder's own version, written into `session` lines.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exit codes of `rukoo-voice-recorder`.
pub mod exit {
    /// The arguments are wrong, or a source can't run on this platform.
    pub const USAGE: i32 = 2;
    /// The meeting folder's state doesn't allow it: a new recording into a meeting that has
    /// one, or a resume after `stop`.
    pub const CONFLICT: i32 = 3;
    /// Another recorder is recording into this meeting.
    pub const BUSY: i32 = 4;
    /// Anything else, such as a disk error at start.
    pub const FAILED: i32 = 1;
}

#[derive(Debug)]
pub struct ExitError {
    pub code: i32,
    pub message: String,
}

impl ExitError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: exit::USAGE,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            code: exit::CONFLICT,
            message: message.into(),
        }
    }

    pub fn busy() -> Self {
        Self {
            code: exit::BUSY,
            message: "another recorder is recording into this meeting".into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            code: exit::FAILED,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

const USAGE: &str = "\
usage:
  rukoo-voice-recorder record --meeting <dir> [--resume] --track mic=<source> [--track call=<source>]
  rukoo-voice-recorder finalize --meeting <dir> [--reason interrupted]
  rukoo-voice-recorder devices";

/// Runs one command line (without the program name).
pub fn run(args: &[String]) -> Result<(), ExitError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(ExitError::usage(USAGE));
    };
    match command.as_str() {
        "record" => record(parse_record(rest)?),
        "finalize" => {
            let (dir, reason) = parse_finalize(rest)?;
            let report = meeting::finalize(&dir, &reason)?;
            print_line(&report.to_json().to_string());
            Ok(())
        }
        "devices" if rest.is_empty() => {
            let list = devices::list().map_err(ExitError::failed)?;
            print_line(&list.to_string());
            Ok(())
        }
        _ => Err(ExitError::usage(USAGE)),
    }
}

/// Prints a result line. A closed stdout is the caller's business, not a reason to panic.
fn print_line(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{line}");
}

fn parse_record(args: &[String]) -> Result<RecordOptions, ExitError> {
    let mut meeting = None;
    let mut resume = false;
    let mut tracks: Vec<(String, source::SourceSpec)> = Vec::new();
    let mut limit_ms = DEFAULT_LIMIT_MS;
    let mut segment_ms = DEFAULT_SEGMENT_MS;
    let mut silence_ms = silence::DEFAULT_THRESHOLD_MS;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| ExitError::usage(format!("{arg} needs a value\n{USAGE}")))
        };
        match arg.as_str() {
            "--meeting" => meeting = Some(PathBuf::from(value()?)),
            "--resume" => resume = true,
            "--track" => {
                let spec = value()?;
                let (name, source) = spec.split_once('=').ok_or_else(|| {
                    ExitError::usage(format!("--track takes <track>=<source>, got {spec:?}"))
                })?;
                if !meeting::TRACKS.contains(&name) {
                    return Err(ExitError::usage(format!(
                        "unknown track {name:?}; tracks are mic and call"
                    )));
                }
                if tracks.iter().any(|(n, _)| n == name) {
                    return Err(ExitError::usage(format!("track {name} is given twice")));
                }
                let source: source::SourceSpec = source.parse().map_err(ExitError::usage)?;
                if !source::supported(&source) {
                    return Err(ExitError::usage(source::UNSUPPORTED));
                }
                tracks.push((name.to_string(), source));
            }
            // For tests: the length limit is 8 hours, a segment 30 s and the silence check's wait
            // 30 s otherwise.
            "--limit-ms" => {
                limit_ms = value()?
                    .parse()
                    .map_err(|_| ExitError::usage("--limit-ms needs milliseconds"))?;
            }
            "--segment-ms" => {
                segment_ms = value()?
                    .parse()
                    .ok()
                    .filter(|&ms| ms >= 100)
                    .ok_or_else(|| ExitError::usage("--segment-ms needs at least 100 ms"))?;
            }
            "--silence-ms" => {
                silence_ms = value()?
                    .parse()
                    .ok()
                    .filter(|&ms| ms >= 100)
                    .ok_or_else(|| ExitError::usage("--silence-ms needs at least 100 ms"))?;
            }
            _ => {
                return Err(ExitError::usage(format!(
                    "unknown argument {arg:?}\n{USAGE}"
                )))
            }
        }
    }
    let meeting =
        meeting.ok_or_else(|| ExitError::usage(format!("--meeting is required\n{USAGE}")))?;
    if tracks.is_empty() {
        return Err(ExitError::usage(format!(
            "at least one --track is required\n{USAGE}"
        )));
    }
    // Keep the timeline's track order stable: mic first.
    tracks.sort_by_key(|(name, _)| meeting::TRACKS.iter().position(|t| t == name));
    Ok(RecordOptions {
        meeting,
        resume,
        tracks,
        limit_ms,
        segment_ms,
        silence_ms,
    })
}

fn parse_finalize(args: &[String]) -> Result<(PathBuf, String), ExitError> {
    let mut meeting = None;
    let mut reason = "interrupted".to_string();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--meeting" => meeting = args.next().map(PathBuf::from),
            "--reason" => {
                reason = args
                    .next()
                    .cloned()
                    .ok_or_else(|| ExitError::usage("--reason needs a value"))?;
                if !meeting::STOP_REASONS.contains(&reason.as_str()) {
                    return Err(ExitError::usage(format!(
                        "unknown reason {reason:?}; use user, rukoo, interrupted or limit"
                    )));
                }
            }
            _ => {
                return Err(ExitError::usage(format!(
                    "unknown argument {arg:?}\n{USAGE}"
                )))
            }
        }
    }
    let meeting =
        meeting.ok_or_else(|| ExitError::usage(format!("--meeting is required\n{USAGE}")))?;
    Ok((meeting, reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_a_record_command_line() {
        let opts = parse_record(&args(&[
            "--meeting",
            "m",
            "--track",
            "call=synthetic:tone:600",
            "--track",
            "mic=synthetic:tone:440",
            "--resume",
            "--limit-ms",
            "1500",
        ]))
        .unwrap();
        assert_eq!(opts.meeting, PathBuf::from("m"));
        assert!(opts.resume);
        assert_eq!(opts.limit_ms, 1500);
        let names: Vec<_> = opts.tracks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["mic", "call"]);
    }

    #[test]
    fn refuses_bad_record_command_lines() {
        for bad in [
            vec!["--track", "mic=synthetic:tone:440"],
            vec!["--meeting", "m"],
            vec!["--meeting", "m", "--track", "screen=synthetic:tone:1"],
            vec!["--meeting", "m", "--track", "mic=nothing"],
            vec![
                "--meeting",
                "m",
                "--track",
                "mic=synthetic:idle",
                "--track",
                "mic=synthetic:idle",
            ],
            vec!["--meeting", "m", "--track"],
            vec!["--meeting", "m", "--bogus"],
        ] {
            let err = parse_record(&args(&bad)).unwrap_err();
            assert_eq!(err.code, exit::USAGE, "{bad:?}");
        }
    }

    #[test]
    fn finalize_defaults_to_interrupted() {
        let (dir, reason) = parse_finalize(&args(&["--meeting", "m"])).unwrap();
        assert_eq!((dir, reason.as_str()), (PathBuf::from("m"), "interrupted"));
        assert!(parse_finalize(&args(&["--meeting", "m", "--reason", "bored"])).is_err());
    }
}
