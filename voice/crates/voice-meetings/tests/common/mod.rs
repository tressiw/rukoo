//! Helpers for the pipeline tests: temp folders, recognizers that need no model, a meeting
//! folder written the way the recorder writes one, and for speaker labels a scripted diarizer, a
//! fake realtime stream and the accuracy scorer.
#![allow(dead_code)]

mod labels;
pub mod score;

// Only the speaker tests use them.
#[allow(unused_imports)]
pub use labels::*;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use voice_audio::SegmentWriter;
use voice_bridge::{EventLog, Stored};
use voice_core::{Hints, ProviderError, Recognized, SpeechToText};
use voice_meetings::{
    CreateMeeting, Library, MeetingDir, PipelineHandle, PipelineState, TimedSpeechToText, TimedText,
};
use voice_protocol::{ScreenMode, StartedBy, Track};

pub const RATE: usize = 16_000;

/// Live speaker labels need the NeMo worker to run faster than real time. Below this many seconds
/// of audio per second, a host (a hosted macOS runner measured 0.5, a Windows runner 8, an M4 27)
/// can't keep labels up, and the tests that play audio in real time would only time out.
/// `RUKOO_VOICE_TEST_MIN_REALTIME` overrides it, to see the skip on a fast machine.
pub fn min_realtime_factor() -> f64 {
    std::env::var("RUKOO_VOICE_TEST_MIN_REALTIME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0)
}

/// True, after printing why, when the speaker worker measured `factor` and that is below
/// [`min_realtime_factor`].
pub fn too_slow_for_live_labels(test: &str, factor: f64) -> bool {
    let min = min_realtime_factor();
    let slow = factor < min;
    if slow {
        eprintln!(
            "skipped {test}: the speaker worker runs at {factor} times real time on this host, below the {min} that live labels need"
        );
    }
    slow
}

/// A temp folder, removed after a passing test and kept after a failing one.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "voice-meetings-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("kept {}", self.0.display());
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// Audio made of (seconds, loud) pieces: loud is a 296 Hz square wave of amplitude 4000, quiet
/// is digital silence.
pub fn speech(pieces: &[(f64, bool)]) -> Vec<i16> {
    let mut out = Vec::new();
    for &(seconds, loud) in pieces {
        let n = (seconds * RATE as f64).round() as usize;
        out.extend((0..n).map(|i| match (loud, (i / 27) % 2) {
            (false, _) => 0,
            (true, 0) => 4000,
            (true, _) => -4000,
        }));
    }
    out
}

/// Text recognizer: "heard <ms> ms", so a passage tells how much audio it came from.
#[derive(Default)]
pub struct Echo {
    pub calls: AtomicUsize,
}

impl SpeechToText for Echo {
    fn transcribe(&self, pcm: &[i16], _hints: &Hints) -> Result<Recognized, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Recognized {
            text: format!("heard {} ms", pcm.len() / 16),
            language: Some("en".into()),
        })
    }
}

/// A recognizer that is never reachable.
pub struct Offline;

impl SpeechToText for Offline {
    fn transcribe(&self, _pcm: &[i16], _hints: &Hints) -> Result<Recognized, ProviderError> {
        Err(ProviderError::new(
            voice_core::ErrorKind::Network,
            "Network error. Check your connection.",
        ))
    }
}

/// A timed recognizer: one word per burst of sound, " w<start in ms of the audio passed in>",
/// timed to the burst. It hears exactly what the window holds, context included.
pub struct Bursts;

impl TimedSpeechToText for Bursts {
    fn transcribe_timed(&self, pcm: &[i16], _hints: &Hints) -> Result<TimedText, ProviderError> {
        let mut out = TimedText::default();
        let frame = 160;
        let mut start: Option<usize> = None;
        for (i, chunk) in pcm.chunks(frame).enumerate() {
            let loud = chunk.iter().any(|s| s.unsigned_abs() > 1000);
            match (loud, start) {
                (true, None) => start = Some(i * frame),
                (false, Some(s)) => {
                    push_word(&mut out, s, i * frame);
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            push_word(&mut out, s, pcm.len());
        }
        out.text = out.tokens.concat().trim().to_owned();
        Ok(out)
    }
}

fn push_word(out: &mut TimedText, from: usize, to: usize) {
    out.tokens.push(format!(" w{}", from / 16));
    out.timestamps.push(from as f64 / RATE as f64);
    out.durations.push((to - from) as f64 / RATE as f64);
}

/// A meeting folder written as the recorder would: segments through the real segment writer,
/// timeline lines one by one.
pub struct Recording {
    pub dir: MeetingDir,
}

impl Recording {
    pub fn new(lib: &Library, tracks: &[Track]) -> Recording {
        Recording::with_screen(lib, tracks, ScreenMode::Off)
    }

    /// A meeting that records the screen as `screen`; the test writes its journal with
    /// [`Recording::journal`], as the screen process would.
    pub fn with_screen(lib: &Library, tracks: &[Track], screen: ScreenMode) -> Recording {
        let dir = lib
            .create(&CreateMeeting {
                title: "Planning".into(),
                tracks: tracks.to_vec(),
                sources: Default::default(),
                screen,
                started_by: StartedBy::Voice,
                context: None,
            })
            .unwrap();
        Recording { dir }
    }

    /// One line of the screen journal, `screen/journal.jsonl`.
    pub fn journal(&self, line: Value) {
        let path = voice_screen_files::paths::journal_path(self.dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(format!("{line}\n").as_bytes()).unwrap();
    }

    pub fn id(&self) -> String {
        self.dir.id().to_string()
    }

    pub fn line(&self, line: Value) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.path().join("timeline.jsonl"))
            .unwrap();
        file.write_all(format!("{line}\n").as_bytes()).unwrap();
    }

    pub fn session(&self, t: u64, session: u64) {
        self.line(json!({"kind":"session","t":t,"wall":1_793_000_000_000u64 + t,"session":session,"pid":1,"resumed":session > 1,"version":1}));
    }

    /// Opens segment `seq` of `track` at `t` and writes its line.
    pub fn open_segment(&self, track: &str, seq: u64, t: u64) -> SegmentWriter {
        let file = format!("audio/{track}/{seq:06}.wav");
        let path = self.dir.path().join(&file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let writer = SegmentWriter::create(&path).unwrap();
        self.line(json!({"kind":"segment","t":t,"track":track,"seq":seq,"file":file}));
        writer
    }

    pub fn close_segment(&self, writer: SegmentWriter, track: &str, seq: u64, t: u64) {
        let samples = writer.close().unwrap();
        self.line(json!({"kind":"segment_end","t":t + samples / 16,"track":track,"seq":seq,"samples":samples}));
    }

    /// A whole segment at once.
    pub fn segment(&self, track: &str, seq: u64, t: u64, pcm: &[i16]) {
        let mut writer = self.open_segment(track, seq, t);
        writer.append(pcm).unwrap();
        self.close_segment(writer, track, seq, t);
    }

    pub fn stop(&self, t: u64) {
        self.line(json!({"kind":"stop","t":t,"wall":1_793_000_000_000u64 + t,"reason":"user"}));
    }
}

pub fn wait_until<T>(what: &str, timeout: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_for_state(
    handle: &PipelineHandle,
    what: impl Fn(&PipelineState) -> bool,
) -> PipelineState {
    wait_until("the pipeline state", Duration::from_secs(30), || {
        let state = handle.state();
        what(&state).then_some(state)
    })
}

pub fn events(log: &EventLog, meeting: &str) -> Vec<Stored> {
    log.meeting_events(meeting).unwrap().1
}

pub fn of<'a>(events: &'a [Stored], name: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|e| e.event == name)
        .map(|e| &e.data)
        .collect()
}

/// Every (event, key) pair must be unique: no event was sent twice.
pub fn assert_no_repeats(events: &[Stored]) {
    let mut seen = std::collections::HashSet::new();
    for e in events {
        let key = match e.event.as_str() {
            "passage" => format!("passage {} {}", e.data["id"], e.data["revision"]),
            "meeting.state" => format!("state {} {}", e.data["state"], e.data["at"]),
            "mark" | "speaker" | "frame" => format!("{} {}", e.event, e.data["id"]),
            other => other.to_owned(),
        };
        assert!(seen.insert(key.clone()), "sent twice: {key}");
    }
}
