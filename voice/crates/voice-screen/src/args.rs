//! The command line of `rukoo-voice-screen`.

use std::path::PathBuf;

use voice_screen_files::policy::Part;

use crate::ExitError;

pub const USAGE: &str = "\
usage:
  rukoo-voice-screen record   --meeting <dir> --mode off|frames|video [--resume] [--monitor <id>] [--exclude-pid <pid>]... [--test <spec>]
  rukoo-voice-screen finalize --meeting <dir> [--reason interrupted|failed]
  rukoo-voice-screen probe
  rukoo-voice-screen inspect <file.mp4>";

/// What a meeting records of the screen, besides `off`, which never starts this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Frames,
    Video,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Frames => "frames",
            Mode::Video => "video",
        }
    }

    pub fn parse(mode: &str) -> Option<Mode> {
        match mode {
            "off" => Some(Mode::Off),
            "frames" => Some(Mode::Frames),
            "video" => Some(Mode::Video),
            _ => None,
        }
    }
}

/// `--test synthetic:<w>x<h>[,change-every=<ms>][,text=<word>][,monitors=<n>][,type-every=<ms>]
/// [,idle-every=<ms>][,dirty][,software-encoder][,no-hardware][,free-bytes-file=<path>]
/// [,encoder-fail-at=<ms>][,write-fail=<video|frames>@<ms>][,device-lost-at=<ms>[x<n>]]
/// [,no-video-device=<n>][,monitor-gone-at=<ms>][,warp]`: a synthetic screen instead of the real
/// monitors, and the faults the tests need. Times are meeting time in ms, except `type-every` and
/// `idle-every`, which count from the start of capture. `type-every` types a small mark every so
/// often, as a keystroke does. `idle-every` sends the unchanged picture again that often, as
/// Windows can on a still screen.
/// `dirty` gives each frame the dirty regions Windows reports; without it frames have none.
/// `warp` makes the graphics device Microsoft's software rasterizer, as on a CI runner without
/// a GPU; it does nothing on macOS, which has no such device. `no-video-device` gives the next
/// `n` graphics devices after the `device-lost-at` loss no video support, as a basic display
/// driver has while the real one restarts.
/// `monitor-gone-at` takes the first monitor out of the list from that moment, without the
/// capture's `Closed` notice, as when that notice is lost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestSpec {
    pub width: u32,
    pub height: u32,
    pub change_every_ms: Option<u64>,
    pub text: Option<String>,
    pub monitors: u32,
    pub type_every_ms: Option<u64>,
    pub idle_every_ms: Option<u64>,
    pub dirty: bool,
    pub software_encoder: bool,
    pub no_hardware: bool,
    pub free_bytes_file: Option<PathBuf>,
    pub encoder_fail_at_ms: Option<u64>,
    pub write_fail: Option<(Part, u64)>,
    pub device_lost_at_ms: Option<u64>,
    pub device_create_failures: u32,
    pub no_video_devices: u32,
    pub monitor_gone_at_ms: Option<u64>,
    pub warp: bool,
}

impl TestSpec {
    pub fn parse(spec: &str) -> Result<TestSpec, String> {
        let rest = spec
            .strip_prefix("synthetic:")
            .ok_or_else(|| format!("--test takes synthetic:<w>x<h>[,…], got {spec:?}"))?;
        let mut parts = rest.split(',');
        let size = parts.next().unwrap_or("");
        let (w, h) = size
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
            .filter(|&(w, h)| (16..=16_384).contains(&w) && (16..=16_384).contains(&h))
            .ok_or_else(|| {
                format!("the synthetic screen needs a size like 1920x1080, got {size:?}")
            })?;
        let mut test = TestSpec {
            width: w,
            height: h,
            monitors: 1,
            ..TestSpec::default()
        };
        let ms = |value: &str, what: &str| {
            value
                .parse::<u64>()
                .map_err(|_| format!("{what} needs milliseconds, got {value:?}"))
        };
        for part in parts {
            let (key, value) = match part.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (part, None),
            };
            match (key, value) {
                ("change-every", Some(v)) => {
                    test.change_every_ms = Some(ms(v, key)?).filter(|&ms| ms > 0)
                }
                ("text", Some(v)) if !v.is_empty() && v.chars().count() <= 200 => {
                    test.text = Some(v.to_string())
                }
                ("monitors", Some(v)) => {
                    test.monitors = v
                        .parse()
                        .ok()
                        .filter(|n| (1..=8).contains(n))
                        .ok_or_else(|| format!("monitors takes 1 to 8, got {v:?}"))?
                }
                ("type-every", Some(v)) => {
                    test.type_every_ms = Some(ms(v, key)?).filter(|&ms| ms > 0)
                }
                ("idle-every", Some(v)) => {
                    test.idle_every_ms = Some(ms(v, key)?).filter(|&ms| ms > 0)
                }
                ("dirty", None) => test.dirty = true,
                ("software-encoder", None) => test.software_encoder = true,
                ("no-hardware", None) => test.no_hardware = true,
                ("warp", None) => test.warp = true,
                ("free-bytes-file", Some(v)) if !v.is_empty() => {
                    test.free_bytes_file = Some(PathBuf::from(v))
                }
                ("encoder-fail-at", Some(v)) => test.encoder_fail_at_ms = Some(ms(v, key)?),
                ("write-fail", Some(v)) => {
                    let (what, at) = v.split_once('@').ok_or_else(|| {
                        format!("write-fail takes video@<ms> or frames@<ms>, got {v:?}")
                    })?;
                    let part = Part::parse(what)
                        .ok_or_else(|| format!("write-fail takes video or frames, got {what:?}"))?;
                    test.write_fail = Some((part, ms(at, key)?));
                }
                ("device-lost-at", Some(v)) => {
                    let (at, times) = match v.split_once('x') {
                        Some((at, n)) => (
                            at,
                            n.parse::<u32>()
                                .ok()
                                .filter(|n| (1..=10).contains(n))
                                .ok_or_else(|| {
                                    format!("device-lost-at takes <ms>x<n>, got {v:?}")
                                })?,
                        ),
                        None => (v, 0),
                    };
                    test.device_lost_at_ms = Some(ms(at, key)?);
                    test.device_create_failures = times;
                }
                ("no-video-device", Some(v)) => {
                    test.no_video_devices = v
                        .parse()
                        .ok()
                        .filter(|n| (1..=10).contains(n))
                        .ok_or_else(|| format!("no-video-device takes 1 to 10, got {v:?}"))?
                }
                ("monitor-gone-at", Some(v)) => test.monitor_gone_at_ms = Some(ms(v, key)?),
                _ => return Err(format!("unknown synthetic screen option {part:?}")),
            }
        }
        Ok(test)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordOptions {
    pub meeting: PathBuf,
    pub mode: Mode,
    pub resume: bool,
    /// `--monitor`: record this monitor or exit 1.
    pub monitor: Option<String>,
    /// `--exclude-pid`: leave the windows of each of these processes out of the recording. Only
    /// the platforms that can filter by process use it; Windows leaves Rukoo out through Rukoo's
    /// own content protection.
    pub exclude_pids: Vec<u32>,
    pub test: Option<TestSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
// The command line is parsed once, so the size of the largest variant costs nothing.
#[allow(clippy::large_enum_variant)]
pub enum Cmd {
    Record(RecordOptions),
    Finalize { meeting: PathBuf, reason: String },
    Probe,
    Inspect(PathBuf),
}

/// Reasons `finalize` writes into `stop`.
pub const FINALIZE_REASONS: [&str; 2] = ["interrupted", "failed"];

/// Parses a command line without the program name. `--test` needs `dev_mode`.
pub fn parse(args: &[String], dev_mode: bool) -> Result<Cmd, ExitError> {
    let Some((command, rest)) = args.split_first() else {
        return Err(ExitError::usage(USAGE));
    };
    match command.as_str() {
        "record" => parse_record(rest, dev_mode).map(Cmd::Record),
        "finalize" => parse_finalize(rest),
        "probe" if rest.is_empty() => Ok(Cmd::Probe),
        "inspect" if rest.len() == 1 => Ok(Cmd::Inspect(PathBuf::from(&rest[0]))),
        _ => Err(ExitError::usage(USAGE)),
    }
}

fn parse_record(args: &[String], dev_mode: bool) -> Result<RecordOptions, ExitError> {
    let mut meeting = None;
    let mut mode = None;
    let mut resume = false;
    let mut monitor = None;
    let mut exclude_pids: Vec<u32> = Vec::new();
    let mut test = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| ExitError::usage(format!("{arg} needs a value\n{USAGE}")))
        };
        match arg.as_str() {
            "--meeting" => meeting = Some(PathBuf::from(value()?)),
            "--mode" => {
                let v = value()?;
                mode = Some(Mode::parse(v).ok_or_else(|| {
                    ExitError::usage(format!("--mode takes off, frames or video, got {v:?}"))
                })?);
            }
            "--resume" => resume = true,
            "--exclude-pid" => {
                let v = value()?;
                let pid = v
                    .parse::<u32>()
                    .ok()
                    .filter(|&pid| pid != 0)
                    .ok_or_else(|| {
                        ExitError::usage(format!(
                            "--exclude-pid takes a process id of 1 or more, got {v:?}"
                        ))
                    })?;
                if !exclude_pids.contains(&pid) {
                    exclude_pids.push(pid);
                }
            }
            "--monitor" => {
                let v = value()?;
                if v.is_empty() || v.chars().count() > 64 {
                    return Err(ExitError::usage(
                        "--monitor takes a monitor id of 1 to 64 characters",
                    ));
                }
                monitor = Some(v.clone());
            }
            "--test" => {
                let v = value()?;
                if !dev_mode {
                    return Err(ExitError::usage(
                        "--test works only in a development copy, with RUKOO_VOICE_DATA_DIR set",
                    ));
                }
                test = Some(TestSpec::parse(v).map_err(ExitError::usage)?);
            }
            _ => {
                return Err(ExitError::usage(format!(
                    "unknown argument {arg:?}\n{USAGE}"
                )))
            }
        }
    }
    Ok(RecordOptions {
        meeting: meeting
            .ok_or_else(|| ExitError::usage(format!("--meeting is required\n{USAGE}")))?,
        mode: mode.ok_or_else(|| ExitError::usage(format!("--mode is required\n{USAGE}")))?,
        resume,
        monitor,
        exclude_pids,
        test,
    })
}

fn parse_finalize(args: &[String]) -> Result<Cmd, ExitError> {
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
                if !FINALIZE_REASONS.contains(&reason.as_str()) {
                    return Err(ExitError::usage(format!(
                        "unknown reason {reason:?}; use interrupted or failed"
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
    Ok(Cmd::Finalize { meeting, reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exit;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_a_record_command_line() {
        let cmd = parse(
            &args(&[
                "record",
                "--meeting",
                "m",
                "--mode",
                "video",
                "--resume",
                "--monitor",
                r"\\.\DISPLAY2",
            ]),
            false,
        )
        .unwrap();
        assert_eq!(
            cmd,
            Cmd::Record(RecordOptions {
                meeting: PathBuf::from("m"),
                mode: Mode::Video,
                resume: true,
                monitor: Some(r"\\.\DISPLAY2".into()),
                exclude_pids: Vec::new(),
                test: None,
            })
        );
    }

    /// Every platform parses the exclusions; only the platforms that can filter by process use
    /// them.
    #[test]
    fn parses_repeated_exceptions() {
        let cmd = parse(
            &args(&[
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--exclude-pid",
                "4821",
                "--exclude-pid",
                "77",
                "--exclude-pid",
                "4821",
            ]),
            false,
        )
        .unwrap();
        let Cmd::Record(opts) = cmd else {
            panic!("not a record command")
        };
        assert_eq!(opts.exclude_pids, vec![4821, 77]);
        // A line without the switch excludes nothing but this process, which the screen process
        // adds itself.
        let Cmd::Record(opts) =
            parse(&args(&["record", "--meeting", "m", "--mode", "off"]), false).unwrap()
        else {
            panic!("not a record command")
        };
        assert!(opts.exclude_pids.is_empty());
    }

    #[test]
    fn test_switches_need_a_development_copy() {
        let line = args(&[
            "record",
            "--meeting",
            "m",
            "--mode",
            "frames",
            "--test",
            "synthetic:1920x1080",
        ]);
        assert_eq!(parse(&line, false).unwrap_err().code, exit::USAGE);
        let Cmd::Record(opts) = parse(&line, true).unwrap() else {
            panic!("not a record command")
        };
        let test = opts.test.unwrap();
        assert_eq!((test.width, test.height, test.monitors), (1920, 1080, 1));
        assert_eq!(test.change_every_ms, None);
    }

    #[test]
    fn parses_every_synthetic_option() {
        let test = TestSpec::parse(
            "synthetic:1440x2560,change-every=10000,text=RUKOO DEMO 42,monitors=2,type-every=150,\
             idle-every=66,dirty,software-encoder,no-hardware,free-bytes-file=C:\\tmp\\free.txt,\
             encoder-fail-at=3000,write-fail=frames@4000,\
             device-lost-at=3000x2,no-video-device=3,monitor-gone-at=5000,warp",
        )
        .unwrap();
        assert_eq!(
            test,
            TestSpec {
                width: 1440,
                height: 2560,
                change_every_ms: Some(10_000),
                text: Some("RUKOO DEMO 42".into()),
                monitors: 2,
                type_every_ms: Some(150),
                idle_every_ms: Some(66),
                dirty: true,
                software_encoder: true,
                no_hardware: true,
                free_bytes_file: Some(PathBuf::from("C:\\tmp\\free.txt")),
                encoder_fail_at_ms: Some(3000),
                write_fail: Some((Part::Frames, 4000)),
                device_lost_at_ms: Some(3000),
                device_create_failures: 2,
                no_video_devices: 3,
                monitor_gone_at_ms: Some(5000),
                warp: true,
            }
        );
        let once =
            TestSpec::parse("synthetic:640x480,device-lost-at=1500,write-fail=video@20").unwrap();
        assert_eq!(
            (
                once.device_lost_at_ms,
                once.device_create_failures,
                once.write_fail
            ),
            (Some(1500), 0, Some((Part::Video, 20)))
        );
    }

    #[test]
    fn refuses_bad_command_lines() {
        for bad in [
            vec![],
            vec!["record", "--mode", "frames"],
            vec!["record", "--meeting", "m"],
            vec!["record", "--meeting", "m", "--mode", "audio"],
            vec!["record", "--meeting", "m", "--mode", "frames", "--bogus"],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--monitor",
                "",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--test",
                "real",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--test",
                "synthetic:0x0",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--test",
                "synthetic:64x64,fast",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--test",
                "synthetic:64x64,monitors=9",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--test",
                "synthetic:64x64,write-fail=audio@1",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--exclude-pid",
                "0",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--exclude-pid",
                "-3",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--exclude-pid",
                "rukoo",
            ],
            vec![
                "record",
                "--meeting",
                "m",
                "--mode",
                "frames",
                "--exclude-pid",
            ],
            vec!["finalize"],
            vec!["finalize", "--meeting", "m", "--reason", "bored"],
            vec!["probe", "extra"],
            vec!["inspect"],
            vec!["inspect", "a", "b"],
            vec!["explode"],
        ] {
            let err = parse(&args(&bad), true).unwrap_err();
            assert_eq!(err.code, exit::USAGE, "{bad:?}");
        }
    }

    #[test]
    fn finalize_defaults_to_interrupted() {
        assert_eq!(
            parse(&args(&["finalize", "--meeting", "m"]), false).unwrap(),
            Cmd::Finalize {
                meeting: PathBuf::from("m"),
                reason: "interrupted".into()
            }
        );
        assert_eq!(
            parse(
                &args(&["finalize", "--meeting", "m", "--reason", "failed"]),
                false
            )
            .unwrap(),
            Cmd::Finalize {
                meeting: PathBuf::from("m"),
                reason: "failed".into()
            }
        );
        assert_eq!(
            parse(&args(&["inspect", "x.mp4"]), false).unwrap(),
            Cmd::Inspect(PathBuf::from("x.mp4"))
        );
        assert_eq!(parse(&args(&["probe"]), false).unwrap(), Cmd::Probe);
    }
}
