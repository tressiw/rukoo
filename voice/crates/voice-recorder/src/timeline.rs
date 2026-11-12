//! `timeline.jsonl`: the meeting's append-only journal. The recorder is its only writer.
//!
//! Every line is one JSON object written with a single write and a trailing newline. A reader
//! ignores a last line without a newline and any line that doesn't parse.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "timeline.jsonl";
/// The version of the meeting folder format, in `session` lines.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Line {
    Session {
        t: u64,
        wall: u64,
        /// Windows interrupt time when `t` was read, in 100 ns units; missing elsewhere.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boot: Option<u64>,
        session: u64,
        pid: u32,
        resumed: bool,
        version: u32,
        #[serde(default)]
        recorder: String,
    },
    Track {
        t: u64,
        track: String,
        source: String,
        device: String,
        #[serde(default)]
        device_id: String,
        native_rate: u32,
        native_channels: u16,
    },
    Segment {
        t: u64,
        track: String,
        seq: u64,
        file: String,
    },
    SegmentEnd {
        t: u64,
        track: String,
        seq: u64,
        samples: u64,
    },
    Heartbeat {
        t: u64,
        wall: u64,
        /// Windows interrupt time when `t` was read, in 100 ns units; missing elsewhere.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boot: Option<u64>,
        samples: BTreeMap<String, u64>,
    },
    DeviceLost {
        t: u64,
        track: String,
        error: String,
    },
    Device {
        t: u64,
        track: String,
        device: String,
        reason: String,
    },
    Pause {
        t: u64,
    },
    Resume {
        t: u64,
    },
    Mark {
        t: u64,
        id: String,
        label: Option<String>,
    },
    Dictation {
        t: u64,
        state: String,
    },
    Suspend {
        t: u64,
        until: u64,
    },
    Crash {
        t: u64,
        last_seen: u64,
    },
    Stop {
        t: u64,
        wall: u64,
        reason: String,
    },
    /// A kind this version doesn't know.
    #[serde(other)]
    Unknown,
}

impl Line {
    pub fn t(&self) -> Option<u64> {
        match self {
            Line::Session { t, .. }
            | Line::Track { t, .. }
            | Line::Segment { t, .. }
            | Line::SegmentEnd { t, .. }
            | Line::Heartbeat { t, .. }
            | Line::DeviceLost { t, .. }
            | Line::Device { t, .. }
            | Line::Pause { t }
            | Line::Resume { t }
            | Line::Mark { t, .. }
            | Line::Dictation { t, .. }
            | Line::Suspend { t, .. }
            | Line::Crash { t, .. }
            | Line::Stop { t, .. } => Some(*t),
            Line::Unknown => None,
        }
    }
}

/// Appends lines to the timeline.
pub struct TimelineWriter {
    file: File,
}

impl TimelineWriter {
    /// Opens the timeline for appending, creating it when missing. A last line without a
    /// newline, left by a writer that was killed mid-write, is cut off first: readers ignore it
    /// anyway, and the next line must not run into it.
    ///
    /// Other processes can still read the file, but no second writer can open it: Windows refuses
    /// that at open through the share mode, Unix through a lock on the open file.
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        exclusive_writer(&mut options);
        let mut file = options.open(path)?;
        lock_writer(&file)?;
        let mut text = Vec::new();
        file.read_to_end(&mut text)?;
        if text.last().is_some_and(|&b| b != b'\n') {
            let keep = text.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
            file.set_len(keep as u64)?;
        }
        file.seek(SeekFrom::End(0))?;
        Ok(Self { file })
    }

    pub fn write(&mut self, line: &Line) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(line)?;
        bytes.push(b'\n');
        self.file.write_all(&bytes)
    }

    pub fn sync(&self) -> io::Result<()> {
        self.file.sync_data()
    }
}

#[cfg(windows)]
fn exclusive_writer(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_DELETE: u32 = 0x4;
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
}

#[cfg(not(windows))]
fn exclusive_writer(_options: &mut OpenOptions) {}

/// Keeps a second writer off the file. Windows already refused it at open through the share mode;
/// elsewhere an exclusive `flock` on the open file does it, which every process on the machine
/// sees and which ends with this one, also when it is hard-killed. A reader never takes the lock,
/// so following the timeline keeps working.
///
/// A lock belongs to the open file, so a process that forks hands it to the child until that child
/// execs, and another process can find the file held for those milliseconds. The recorder starts
/// no other program, so nothing holds a meeting's files that way.
///
/// `WouldBlock` says another writer holds it, which callers know as [`io::ErrorKind::WouldBlock`].
#[cfg(unix)]
fn lock_writer(file: &File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    loop {
        // SAFETY: `flock` only reads the descriptor; the lock belongs to the file, not to memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // EWOULDBLOCK reads as `WouldBlock`; a signal in the call changes nothing, so try again.
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(not(unix))]
fn lock_writer(_file: &File) -> io::Result<()> {
    Ok(())
}

/// Reads a timeline; a missing file reads as empty.
pub fn read(path: &Path) -> io::Result<Vec<Line>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(parse(&String::from_utf8_lossy(&bytes))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Parses complete lines, skipping the ones that don't parse and a last line without a newline.
pub fn parse(text: &str) -> Vec<Line> {
    let complete = match text.rfind('\n') {
        Some(end) => &text[..end],
        None => "",
    };
    complete
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// A segment as the timeline records it.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentRecord {
    pub track: String,
    pub seq: u64,
    pub t: u64,
    pub file: String,
    /// The samples in its `segment_end` line, when it has one.
    pub ended: Option<u64>,
}

/// What a resumed session or `finalize` needs to know about the meeting so far.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct History {
    pub lines: usize,
    pub sessions: u64,
    /// Wall time of session 1; meeting time 0.
    pub first_wall: Option<u64>,
    pub last_session_t: Option<u64>,
    pub latest_t: u64,
    pub last_heartbeat: Option<u64>,
    pub last_segment_end: Option<u64>,
    pub segments: Vec<SegmentRecord>,
    pub stopped: bool,
    pub paused: bool,
    /// The start of a dictation stretch that no `dictation` `end`, `crash` or `stop` closed.
    pub dictating: Option<u64>,
    /// The highest `n` among the `k-<n>` mark ids.
    pub last_mark: u64,
}

impl History {
    pub fn from_lines(lines: &[Line]) -> Self {
        let mut h = History {
            lines: lines.len(),
            ..History::default()
        };
        for line in lines {
            if let Some(t) = line.t() {
                h.latest_t = h.latest_t.max(t);
            }
            match line {
                Line::Session {
                    t, wall, session, ..
                } => {
                    h.sessions = h.sessions.max(*session);
                    h.first_wall.get_or_insert(*wall);
                    h.last_session_t = Some(*t);
                }
                Line::Segment {
                    t,
                    track,
                    seq,
                    file,
                } => h.segments.push(SegmentRecord {
                    track: track.clone(),
                    seq: *seq,
                    t: *t,
                    file: file.clone(),
                    ended: None,
                }),
                Line::SegmentEnd {
                    t,
                    track,
                    seq,
                    samples,
                } => {
                    h.last_segment_end = Some(*t);
                    if let Some(s) = h
                        .segments
                        .iter_mut()
                        .rev()
                        .find(|s| s.track == *track && s.seq == *seq)
                    {
                        s.ended = Some(*samples);
                    }
                }
                Line::Heartbeat { t, .. } => h.last_heartbeat = Some(*t),
                Line::Pause { .. } => h.paused = true,
                Line::Resume { .. } => h.paused = false,
                Line::Mark { id, .. } => {
                    if let Some(n) = id.strip_prefix("k-").and_then(|n| n.parse::<u64>().ok()) {
                        h.last_mark = h.last_mark.max(n);
                    }
                }
                Line::Dictation { t, state } => match state.as_str() {
                    "start" => {
                        h.dictating.get_or_insert(*t);
                    }
                    "end" => h.dictating = None,
                    _ => {}
                },
                // A dead recorder's dictation ended with it, as voice-meetings reads it.
                Line::Crash { .. } => h.dictating = None,
                Line::Stop { .. } => {
                    h.stopped = true;
                    h.dictating = None;
                }
                _ => {}
            }
        }
        h
    }

    pub fn is_empty(&self) -> bool {
        self.lines == 0
    }

    pub fn open_segments(&self) -> impl Iterator<Item = &SegmentRecord> {
        self.segments.iter().filter(|s| s.ended.is_none())
    }

    pub fn last_seq(&self, track: &str) -> u64 {
        self.segments
            .iter()
            .filter(|s| s.track == track)
            .map(|s| s.seq)
            .max()
            .unwrap_or(0)
    }

    /// The last moment the previous session is known to have recorded: the highest of its
    /// last heartbeat, its last `segment_end`, and for each track the last segment's start plus
    /// the samples on disk. `disk_samples` gives a segment's samples on disk.
    pub fn last_seen(&self, disk_samples: impl Fn(&SegmentRecord) -> u64) -> u64 {
        let mut seen = self
            .last_session_t
            .into_iter()
            .chain(self.last_heartbeat)
            .chain(self.last_segment_end)
            .max()
            .unwrap_or(0);
        let mut last_per_track: BTreeMap<&str, &SegmentRecord> = BTreeMap::new();
        for s in &self.segments {
            let entry = last_per_track.entry(&s.track).or_insert(s);
            if s.seq >= entry.seq {
                *entry = s;
            }
        }
        for s in last_per_track.values() {
            seen = seen.max(s.t + samples_to_ms(disk_samples(s)));
        }
        seen
    }
}

/// Duration of `samples` at 16 kHz, in whole milliseconds.
pub fn samples_to_ms(samples: u64) -> u64 {
    samples * 1000 / voice_audio::SAMPLE_RATE as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_have_the_documented_shape() {
        let line = Line::Track {
            t: 3,
            track: "mic".into(),
            source: "default-input".into(),
            device: "USB mic".into(),
            device_id: "wasapi:x".into(),
            native_rate: 48_000,
            native_channels: 2,
        };
        assert_eq!(
            serde_json::to_string(&line).unwrap(),
            r#"{"kind":"track","t":3,"track":"mic","source":"default-input","device":"USB mic","deviceId":"wasapi:x","nativeRate":48000,"nativeChannels":2}"#
        );
        let end = Line::SegmentEnd {
            t: 30_000,
            track: "call".into(),
            seq: 1,
            samples: 480_000,
        };
        assert_eq!(
            serde_json::to_string(&end).unwrap(),
            r#"{"kind":"segment_end","t":30000,"track":"call","seq":1,"samples":480000}"#
        );
        let crash = Line::Crash { t: 9, last_seen: 7 };
        assert_eq!(
            serde_json::to_string(&crash).unwrap(),
            r#"{"kind":"crash","t":9,"lastSeen":7}"#
        );
    }

    #[test]
    fn boot_is_written_only_when_known() {
        let heartbeat = |boot| Line::Heartbeat {
            t: 1_000,
            wall: 1_793_384_409_989,
            boot,
            samples: BTreeMap::from([("mic".to_string(), 16_144)]),
        };
        let with = heartbeat(Some(123_456_789_012));
        assert_eq!(
            serde_json::to_string(&with).unwrap(),
            r#"{"kind":"heartbeat","t":1000,"wall":1793384409989,"boot":123456789012,"samples":{"mic":16144}}"#
        );
        assert_eq!(
            serde_json::to_string(&heartbeat(None)).unwrap(),
            r#"{"kind":"heartbeat","t":1000,"wall":1793384409989,"samples":{"mic":16144}}"#
        );
        // Lines from before the field read as `None`, and the field reads back.
        let old = parse(concat!(
            r#"{"kind":"session","t":0,"wall":1,"session":1,"pid":1,"resumed":false,"version":1,"recorder":"0.1.0"}"#,
            "\n",
            r#"{"kind":"heartbeat","t":1000,"wall":1793384409989,"boot":123456789012,"samples":{"mic":16144}}"#,
            "\n"
        ));
        assert!(matches!(old[0], Line::Session { boot: None, .. }));
        assert_eq!(old[1], with);
    }

    #[test]
    fn parse_skips_a_partial_last_line_and_broken_lines() {
        let text = concat!(
            r#"{"kind":"pause","t":5}"#,
            "\n",
            "garbage\n",
            r#"{"kind":"something_new","t":6}"#,
            "\n",
            r#"{"kind":"resume","t":7}"#,
            "\n",
            r#"{"kind":"heartbeat","t":8,"wa"#
        );
        assert_eq!(
            parse(text),
            vec![Line::Pause { t: 5 }, Line::Unknown, Line::Resume { t: 7 }]
        );
        // A complete object without its newline is still a partial line.
        assert_eq!(parse(r#"{"kind":"pause","t":5}"#), vec![]);
    }

    #[test]
    fn history_finds_open_segments_marks_and_last_seen() {
        let lines = vec![
            Line::Session {
                t: 0,
                wall: 1_000,
                boot: None,
                session: 1,
                pid: 1,
                resumed: false,
                version: 1,
                recorder: String::new(),
            },
            Line::Segment {
                t: 5,
                track: "mic".into(),
                seq: 1,
                file: "audio/mic/000001.wav".into(),
            },
            Line::Segment {
                t: 6,
                track: "call".into(),
                seq: 1,
                file: "audio/call/000001.wav".into(),
            },
            Line::Mark {
                t: 900,
                id: "k-4".into(),
                label: None,
            },
            Line::Heartbeat {
                t: 1_000,
                wall: 2_000,
                boot: Some(50_000_000),
                samples: BTreeMap::new(),
            },
            Line::SegmentEnd {
                t: 1_500,
                track: "call".into(),
                seq: 1,
                samples: 23_904,
            },
            Line::Pause { t: 1_500 },
        ];
        let h = History::from_lines(&lines);
        assert_eq!(h.sessions, 1);
        assert_eq!(h.first_wall, Some(1_000));
        assert_eq!(h.last_mark, 4);
        assert!(h.paused);
        assert!(!h.stopped);
        let open: Vec<_> = h.open_segments().map(|s| s.track.as_str()).collect();
        assert_eq!(open, vec!["mic"]);
        assert_eq!(h.last_seq("mic"), 1);
        assert_eq!(h.last_seq("call"), 1);
        // The mic segment holds 2 s on disk: 5 + 2000 beats the last segment_end.
        let seen = h.last_seen(|s| if s.track == "mic" { 32_000 } else { 23_904 });
        assert_eq!(seen, 2_005);
        let seen = h.last_seen(|_| 0);
        assert_eq!(seen, 1_500);
    }

    #[test]
    fn history_knows_an_open_dictation() {
        let dictation = |t, state: &str| Line::Dictation {
            t,
            state: state.into(),
        };
        let open = |lines: &[Line]| History::from_lines(lines).dictating;
        assert_eq!(open(&[dictation(10, "start")]), Some(10));
        assert_eq!(
            open(&[dictation(10, "start"), dictation(20, "start")]),
            Some(10)
        );
        assert_eq!(open(&[dictation(10, "start"), dictation(30, "end")]), None);
        assert_eq!(
            open(&[
                dictation(10, "start"),
                Line::Crash {
                    t: 90,
                    last_seen: 40
                }
            ]),
            None
        );
        assert_eq!(
            open(&[
                dictation(10, "start"),
                Line::Stop {
                    t: 40,
                    wall: 0,
                    reason: "user".into()
                }
            ]),
            None
        );
    }

    #[test]
    fn writer_cuts_off_a_partial_last_line() {
        let dir = std::env::temp_dir().join(format!(
            "voice-recorder-timeline-{}-{}",
            std::process::id(),
            crate::clock::wall_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "{\"kind\":\"pause\",\"t\":1}\n{\"kind\":\"resu").unwrap();
        let mut w = TimelineWriter::open(&path).unwrap();
        w.write(&Line::Resume { t: 2 }).unwrap();
        drop(w);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"kind\":\"pause\",\"t\":1}\n{\"kind\":\"resume\",\"t\":2}\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Windows reports its share mode as a sharing violation; Unix as a lock it can't take. The
    /// recorder maps both to its "another recorder" exit code, which `meeting.rs` tests.
    #[cfg(unix)]
    #[test]
    fn a_second_writer_cannot_open_the_timeline_but_a_reader_can() {
        let dir = std::env::temp_dir().join(format!(
            "voice-recorder-lock-{}-{}",
            std::process::id(),
            crate::clock::wall_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let mut first = TimelineWriter::open(&path).unwrap();
        first.write(&Line::Pause { t: 1 }).unwrap();
        let err = match TimelineWriter::open(&path) {
            Ok(_) => panic!("a second writer opened the timeline"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
        assert_eq!(read(&path).unwrap(), vec![Line::Pause { t: 1 }]);
        drop(first);
        // The next writer gets it once the first closed the file.
        let mut next = TimelineWriter::open(&path).unwrap();
        next.write(&Line::Resume { t: 2 }).unwrap();
        drop(next);
        assert_eq!(
            read(&path).unwrap(),
            vec![Line::Pause { t: 1 }, Line::Resume { t: 2 }]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
