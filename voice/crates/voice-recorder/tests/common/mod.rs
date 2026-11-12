//! Helpers for tests that run the real `rukoo-voice-recorder` binary on a temp meeting folder.
#![allow(dead_code)]

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use voice_recorder::control::Client;
use voice_recorder::meeting::RecorderFile;

pub const BIN: &str = env!("CARGO_BIN_EXE_rukoo-voice-recorder");

/// CI machines are slow and run several recorders at once, so every wait is generous; the
/// tolerances on recorded times are in the tests that use them.
pub const START_TIMEOUT: Duration = Duration::from_secs(20);
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// True, after printing why, when CI's host probe found the host too stalled for timing windows
/// (`voice/scripts/host-probe.mjs` sets `RUKOO_VOICE_SLOW_HOST` for the test step). A test with a
/// tight window on a late tick returns early on it. Windows and Linux CI never set it.
pub fn skip_on_slow_host(test: &str) -> bool {
    let slow = std::env::var_os("RUKOO_VOICE_SLOW_HOST").is_some_and(|v| !v.is_empty());
    if slow {
        eprintln!(
            "skipped {test}: RUKOO_VOICE_SLOW_HOST is set, this host stalls longer than its timing windows allow"
        );
    }
    slow
}

/// A temp folder holding a meeting folder and the recorder's stderr logs. It's removed after a
/// passing test and kept after a failing one, so the files can be inspected.
pub struct TempMeeting {
    pub root: PathBuf,
    pub dir: PathBuf,
}

impl TempMeeting {
    pub fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "voice-recorder-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        let dir = root.join("m-20261030-120000-abcd");
        fs::create_dir_all(&dir).unwrap();
        // The app writes meeting.json before it starts the recorder.
        let meeting = json!({
            "id": "m-20261030-120000-abcd",
            "title": "Test meeting",
            "createdAt": 1_793_000_000_000u64,
            "tracks": ["mic", "call"],
            "sources": {},
            "screen": "off",
            "startedBy": "voice",
            "context": null,
            "version": 1,
        });
        fs::write(dir.join("meeting.json"), meeting.to_string()).unwrap();
        Self { root, dir }
    }

    pub fn arg(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }

    /// Starts the recorder with `args` after the program name.
    pub fn spawn(&self, args: &[&str]) -> Child {
        let n = fs::read_dir(&self.root).map(|d| d.count()).unwrap_or(0);
        let log = File::create(self.root.join(format!("stderr-{n}.log"))).unwrap();
        Command::new(BIN)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("start rukoo-voice-recorder")
    }

    pub fn record(&self, tracks: &[&str], extra: &[&str]) -> Child {
        let dir = self.arg();
        let mut args = vec!["record", "--meeting", dir.as_str()];
        for track in tracks {
            args.push("--track");
            args.push(track);
        }
        args.extend_from_slice(extra);
        self.spawn(&args)
    }

    /// Runs a command to completion and returns its status and stdout.
    pub fn run(&self, args: &[&str]) -> (ExitStatus, String) {
        let out = Command::new(BIN)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("run rukoo-voice-recorder");
        (
            out.status,
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    pub fn timeline_text(&self) -> String {
        fs::read_to_string(self.dir.join("timeline.jsonl")).unwrap_or_default()
    }

    /// Every complete line, each of which must parse; a partial last line is allowed.
    pub fn lines(&self) -> Vec<Value> {
        let text = self.timeline_text();
        let complete = match text.rfind('\n') {
            Some(end) => &text[..end],
            None => "",
        };
        complete
            .lines()
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|e| panic!("bad line {line:?}: {e}"))
            })
            .collect()
    }

    /// Lines that may still be growing, read leniently while the recorder runs.
    pub fn lines_so_far(&self) -> Vec<Value> {
        let text = self.timeline_text();
        let complete = match text.rfind('\n') {
            Some(end) => &text[..end],
            None => "",
        };
        complete
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    pub fn recorder_file(&self) -> Option<RecorderFile> {
        let bytes = fs::read(self.dir.join("recorder.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Waits until the recorder of `session` answers on its control port.
    pub fn connect(&self, session: u64) -> (Client, String) {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if let Some(file) = self.recorder_file().filter(|f| f.session == session) {
                if let Ok(mut client) = Client::connect(file.port, Duration::from_secs(2)) {
                    let status = client
                        .request(&json!({ "token": file.token, "cmd": "status" }))
                        .unwrap();
                    assert_eq!(status["ok"], true, "{status}");
                    return (client, file.token);
                }
            }
            assert!(
                Instant::now() < deadline,
                "recorder {session} never answered"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn segment_path(&self, file: &str) -> PathBuf {
        self.dir.join(file)
    }

    pub fn wait_for_heartbeat(&self, min_t: u64) -> Vec<Value> {
        wait_until(WAIT_TIMEOUT, || {
            let lines = self.lines_so_far();
            let reached = lines
                .iter()
                .any(|l| l["kind"] == "heartbeat" && t(l) >= min_t);
            reached.then_some(lines)
        })
    }
}

impl Drop for TempMeeting {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("kept the test folder {}", self.root.display());
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub fn wait_until<T>(timeout: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out after {timeout:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_exit(child: &mut Child) -> ExitStatus {
    wait_until(WAIT_TIMEOUT, || child.try_wait().unwrap())
}

pub fn t(line: &Value) -> u64 {
    line["t"].as_u64().unwrap()
}

pub fn of_kind<'a>(lines: &'a [Value], kind: &str) -> Vec<&'a Value> {
    lines.iter().filter(|l| l["kind"] == kind).collect()
}

/// Sends a command with the token and returns the reply.
pub fn command(client: &mut Client, token: &str, mut request: Value) -> Value {
    request["token"] = json!(token);
    client.request(&request).unwrap()
}

/// A segment from the timeline, read back with hound.
#[derive(Debug)]
pub struct Seg {
    pub seq: u64,
    pub t: u64,
    pub file: String,
    pub pcm: Vec<i16>,
}

impl Seg {
    /// End time by the samples in the file.
    pub fn end(&self) -> u64 {
        self.t + self.pcm.len() as u64 / 16
    }
}

/// The track's segments in timeline order. Every file must open with an independent WAV reader
/// and its header must count no more samples than the file holds.
pub fn segments(m: &TempMeeting, lines: &[Value], track: &str) -> Vec<Seg> {
    lines
        .iter()
        .filter(|l| l["kind"] == "segment" && l["track"] == track)
        .map(|l| {
            let file = l["file"].as_str().unwrap().to_string();
            let path = m.segment_path(&file);
            let len = fs::metadata(&path).unwrap().len();
            let reader = hound::WavReader::open(&path)
                .unwrap_or_else(|e| panic!("{file} doesn't open as a WAV: {e}"));
            let spec = reader.spec();
            assert_eq!(
                (spec.channels, spec.sample_rate, spec.bits_per_sample),
                (1, 16_000, 16),
                "{file}"
            );
            let counted = reader.len() as u64;
            assert!(
                44 + counted * 2 <= len,
                "{file}: header counts {counted} samples, file has {len} bytes"
            );
            let pcm: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
            Seg {
                seq: l["seq"].as_u64().unwrap(),
                t: t(l),
                file,
                pcm,
            }
        })
        .collect()
}

/// Asserts the samples are a clean tone at `hz` at the synthetic level, window by window, so a
/// stretch of zeros anywhere fails. The first and last 10 ms are skipped: the resampler starts
/// and ends each stream from silence.
pub fn assert_tone(pcm: &[i16], hz: f64, what: &str) {
    const EDGE: usize = 160;
    const WINDOW: usize = 1_600;
    assert!(
        pcm.len() > 2 * EDGE + WINDOW,
        "{what}: only {} samples",
        pcm.len()
    );
    let expected_rms = 0.5 * 32_767.0 / 2f64.sqrt();
    for (n, window) in pcm[EDGE..pcm.len() - EDGE].chunks(WINDOW).enumerate() {
        if window.len() < WINDOW / 2 {
            continue;
        }
        let rms =
            (window.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / window.len() as f64).sqrt();
        assert!(
            (0.85..1.15).contains(&(rms / expected_rms)),
            "{what}: window {n} has RMS {rms:.0}, expected about {expected_rms:.0}"
        );
        let crossings = window
            .windows(2)
            .filter(|w| (w[0] < 0) != (w[1] < 0))
            .count() as f64;
        let measured = crossings * 16_000.0 / window.len() as f64 / 2.0;
        assert!(
            (measured - hz).abs() < hz * 0.05 + 10.0,
            "{what}: window {n} sounds like {measured:.0} Hz, expected {hz}"
        );
    }
}
