//! The meeting folder: file names, `recorder.json`, the live-recorder check and `finalize`.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clock::wall_ms;
use crate::control::Client;
use crate::timeline::{self, History, Line, SegmentRecord, TimelineWriter};
use crate::ExitError;

pub const TRACKS: [&str; 2] = ["mic", "call"];
pub const RECORDER_FILE: &str = "recorder.json";
pub const STOP_REASONS: [&str; 4] = ["user", "rukoo", "interrupted", "limit"];

/// `recorder.json`: how to reach the live recorder. Removed on a clean stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderFile {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub session: u64,
    pub started_at: u64,
}

/// The folder-relative path of a segment, with forward slashes on every platform.
pub fn segment_file(track: &str, seq: u64) -> String {
    format!("audio/{track}/{seq:06}.wav")
}

pub fn timeline_path(dir: &Path) -> PathBuf {
    dir.join(timeline::FILE_NAME)
}

/// The highest segment number among the files in `audio/<track>/`, including files the
/// timeline never mentioned because the recorder died right after creating them.
pub fn highest_seq_on_disk(dir: &Path, track: &str) -> u64 {
    let Ok(entries) = fs::read_dir(dir.join("audio").join(track)) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            name.strip_suffix(".wav")?.parse::<u64>().ok()
        })
        .max()
        .unwrap_or(0)
}

/// Writes `recorder.json` through a temporary file and a rename, so a reader never sees half.
pub fn write_recorder_file(dir: &Path, file: &RecorderFile) -> io::Result<()> {
    let tmp = dir.join("recorder.json.tmp");
    let bytes = serde_json::to_vec(file)?;
    {
        let out = fs::File::create(&tmp)?;
        io::Write::write_all(&mut &out, &bytes)?;
        out.sync_data()?;
    }
    fs::rename(&tmp, dir.join(RECORDER_FILE))
}

pub fn read_recorder_file(dir: &Path) -> Option<RecorderFile> {
    let bytes = fs::read(dir.join(RECORDER_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn remove_recorder_file(dir: &Path) {
    let _ = fs::remove_file(dir.join(RECORDER_FILE));
}

/// The recorder named in `recorder.json`, if it answers on its port with its token. A stale
/// file, or another program that took the port since, doesn't count.
pub fn live_recorder(dir: &Path) -> Option<RecorderFile> {
    let file = read_recorder_file(dir)?;
    let mut client = Client::connect(file.port, Duration::from_millis(500)).ok()?;
    let reply = client
        .request(&json!({ "token": file.token, "cmd": "status" }))
        .ok()?;
    (reply["ok"] == true).then_some(file)
}

/// Opens the timeline for writing, reporting a second writer as [`ExitError::busy`].
pub fn open_timeline(dir: &Path) -> Result<TimelineWriter, ExitError> {
    TimelineWriter::open(&timeline_path(dir)).map_err(|e| {
        if held_by_another_writer(&e) {
            ExitError::busy()
        } else {
            ExitError::failed(format!("can't open the timeline: {e}"))
        }
    })
}

/// Windows refuses a second writer with a sharing violation at open; Unix with an advisory lock
/// on the open file that it can't take ([`io::ErrorKind::WouldBlock`]).
fn held_by_another_writer(e: &io::Error) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let sharing_violation = cfg!(windows) && e.raw_os_error() == Some(ERROR_SHARING_VIOLATION);
    sharing_violation || (cfg!(unix) && e.kind() == io::ErrorKind::WouldBlock)
}

/// Repairs the headers of `segments` from their file lengths and returns the samples each
/// holds. A missing or unreadable file counts as empty and is reported in `problems`.
pub fn repair_segments<'a>(
    dir: &Path,
    segments: impl IntoIterator<Item = &'a SegmentRecord>,
    problems: &mut Vec<String>,
) -> HashMap<(String, u64), u64> {
    let mut samples = HashMap::new();
    for s in segments {
        let n = match voice_audio::repair_segment(&dir.join(&s.file)) {
            Ok(n) => n,
            Err(e) => {
                problems.push(format!("{}: {e}", s.file));
                0
            }
        };
        samples.insert((s.track.clone(), s.seq), n);
    }
    samples
}

/// What `finalize` did.
#[derive(Debug, Default)]
pub struct Finalized {
    /// Segments that got their `segment_end` now.
    pub ended: usize,
    /// The `stop` line written now, or `None` when the timeline already had one.
    pub stopped_at: Option<u64>,
    pub problems: Vec<String>,
}

impl Finalized {
    pub fn to_json(&self) -> Value {
        json!({
            "ok": true,
            "ended": self.ended,
            "stopped": self.stopped_at.is_some(),
            "t": self.stopped_at,
            "problems": self.problems,
        })
    }
}

/// Finishes a meeting whose recorder died: repairs every segment header from the file length,
/// writes `segment_end` for segments that have none, and a final `stop` when the timeline has
/// none. Running it again changes nothing.
pub fn finalize(dir: &Path, reason: &str) -> Result<Finalized, ExitError> {
    if !dir.is_dir() {
        return Err(ExitError::usage(format!(
            "meeting folder {} doesn't exist",
            dir.display()
        )));
    }
    if live_recorder(dir).is_some() {
        return Err(ExitError::busy());
    }
    let mut timeline = open_timeline(dir)?;
    let lines = timeline::read(&timeline_path(dir))
        .map_err(|e| ExitError::failed(format!("can't read the timeline: {e}")))?;
    let history = History::from_lines(&lines);
    let mut report = Finalized::default();
    let samples = repair_segments(dir, &history.segments, &mut report.problems);
    let on_disk = |s: &SegmentRecord| samples.get(&(s.track.clone(), s.seq)).copied().unwrap_or(0);

    for s in history.open_segments() {
        let n = on_disk(s);
        write(
            &mut timeline,
            &Line::SegmentEnd {
                t: s.t + timeline::samples_to_ms(n),
                track: s.track.clone(),
                seq: s.seq,
                samples: n,
            },
        )?;
        report.ended += 1;
    }
    if !history.stopped {
        let t = history.last_seen(on_disk);
        // The app that dictated read the microphone through the dead recorder's tap.
        if let Some(since) = history.dictating {
            write(
                &mut timeline,
                &Line::Dictation {
                    t: t.max(since),
                    state: "end".into(),
                },
            )?;
        }
        let wall = history.first_wall.map_or_else(wall_ms, |w| w + t);
        write(
            &mut timeline,
            &Line::Stop {
                t,
                wall,
                reason: reason.to_string(),
            },
        )?;
        report.stopped_at = Some(t);
    }
    timeline
        .sync()
        .map_err(|e| ExitError::failed(format!("can't sync the timeline: {e}")))?;
    remove_recorder_file(dir);
    Ok(report)
}

fn write(timeline: &mut TimelineWriter, line: &Line) -> Result<(), ExitError> {
    timeline
        .write(line)
        .map_err(|e| ExitError::failed(format!("can't write the timeline: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_files_are_numbered_with_six_digits() {
        assert_eq!(segment_file("mic", 1), "audio/mic/000001.wav");
        assert_eq!(segment_file("call", 1234567), "audio/call/1234567.wav");
    }

    #[test]
    fn finds_the_highest_segment_on_disk() {
        let dir = std::env::temp_dir().join(format!(
            "voice-recorder-seq-{}-{}",
            std::process::id(),
            wall_ms()
        ));
        let audio = dir.join("audio").join("mic");
        fs::create_dir_all(&audio).unwrap();
        assert_eq!(highest_seq_on_disk(&dir, "call"), 0);
        for name in ["000001.wav", "000007.wav", "notes.txt", "000003.wav"] {
            fs::write(audio.join(name), b"").unwrap();
        }
        assert_eq!(highest_seq_on_disk(&dir, "mic"), 7);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_second_writer_is_busy_but_readers_still_read() {
        let dir = std::env::temp_dir().join(format!(
            "voice-recorder-busy-{}-{}",
            std::process::id(),
            wall_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        let mut first = open_timeline(&dir).unwrap();
        first.write(&Line::Pause { t: 1 }).unwrap();
        let second = match open_timeline(&dir) {
            Ok(_) => panic!("a second writer opened the timeline"),
            Err(e) => e,
        };
        assert_eq!(second.code, crate::exit::BUSY, "{second}");
        // The lock is the writer's alone: reading the timeline and its segments keeps working.
        assert_eq!(
            timeline::read(&timeline_path(&dir)).unwrap(),
            vec![Line::Pause { t: 1 }]
        );
        drop(first);
        let mut again = open_timeline(&dir).unwrap();
        again.write(&Line::Resume { t: 2 }).unwrap();
        drop(again);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recorder_file_uses_camel_case() {
        let file = RecorderFile {
            pid: 7,
            port: 4000,
            token: "t".into(),
            session: 2,
            started_at: 99,
        };
        assert_eq!(
            serde_json::to_string(&file).unwrap(),
            r#"{"pid":7,"port":4000,"token":"t","session":2,"startedAt":99}"#
        );
    }
}
