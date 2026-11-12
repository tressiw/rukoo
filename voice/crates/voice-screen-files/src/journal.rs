//! `screen/journal.jsonl`: the screen process's append-only journal. It is its only writer.
//!
//! One JSON object per line, written with a single write and a trailing newline, `kind` first.
//! A reader ignores a last line without a newline and any line that doesn't parse, as for
//! `timeline.jsonl`. `t` is meeting time in ms on the recorder's clock.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::follow::{self, Follower};
use crate::paths;

/// The version of the screen folder format, in `session` lines.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Line {
    /// The process starts recording into the meeting. `mode` is `off`, `frames` or `video`; `anchor`
    /// is `boot` or `wall`, the clock the mapping uses; `screen` is the crate version.
    Session {
        t: u64,
        wall: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boot: Option<u64>,
        session: u64,
        pid: u32,
        resumed: bool,
        version: u32,
        mode: String,
        anchor: String,
        screen: String,
    },
    /// A user changes capture without ending the screen process or dropping a part.
    Mode {
        t: u64,
        id: String,
        mode: String,
        reason: String,
    },
    /// Capture moves to a monitor. `reason`: `start`, `resume`, `user`, `lost`.
    Monitor {
        t: u64,
        id: String,
        name: String,
        width: u32,
        height: u32,
        primary: bool,
        reason: String,
    },
    /// The first frame of a capture session arrived (start, switch, unlock, resume).
    Capture {
        t: u64,
        wall: u64,
        monitor: String,
        width: u32,
        height: u32,
    },
    /// A video file starts; its media time 0 is meeting time `t`.
    Video {
        t: u64,
        seq: u64,
        file: String,
        width: u32,
        height: u32,
        fps: u32,
        encoder: String,
        hardware: bool,
    },
    /// A video file ended; `t` is its last sample's end. `reason`: `pause`, `lock`, `monitor`,
    /// `resize`, `clock`, `stop`, `error`, `disk`, `crash`.
    VideoEnd {
        t: u64,
        seq: u64,
        reason: String,
        bytes: u64,
    },
    /// A frame was saved and read; written after OCR. `ocr`: `ok`, `none`, `skipped`, `failed`.
    Frame {
        t: u64,
        id: String,
        file: String,
        width: u32,
        height: u32,
        text: String,
        ocr: String,
        language: Option<String>,
    },
    /// A part stopped for the rest of the meeting. `what`: `video`, `frames`; `reason`:
    /// `disk`, `encoder`, `device`.
    Dropped {
        t: u64,
        what: String,
        reason: String,
    },
    /// The D3D device was removed or reset; the next `capture` ends the gap.
    DeviceLost { t: u64 },
    /// No monitor is left to record, or a pinned one went away; the next `capture` ends the wait.
    NoMonitor { t: u64 },
    /// The session locked or disconnected.
    Lock { t: u64 },
    /// The session came back.
    Unlock { t: u64 },
    /// Every 2 s.
    Heartbeat {
        t: u64,
        wall: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boot: Option<u64>,
        frames: u64,
        video_bytes: u64,
    },
    /// Written by a resumed session: the dead one was last seen at `last_seen`.
    Crash { t: u64, last_seen: u64 },
    /// Nothing follows. `reason`: `meeting`, `dropped`, `failed`, `interrupted`.
    Stop { t: u64, reason: String },
    /// A kind this version doesn't know.
    #[serde(other)]
    Unknown,
}

impl Line {
    /// The line's meeting time, or `None` for an unknown kind.
    pub fn t(&self) -> Option<u64> {
        match self {
            Line::Session { t, .. }
            | Line::Mode { t, .. }
            | Line::Monitor { t, .. }
            | Line::Capture { t, .. }
            | Line::Video { t, .. }
            | Line::VideoEnd { t, .. }
            | Line::Frame { t, .. }
            | Line::Dropped { t, .. }
            | Line::DeviceLost { t }
            | Line::NoMonitor { t }
            | Line::Lock { t }
            | Line::Unlock { t }
            | Line::Heartbeat { t, .. }
            | Line::Crash { t, .. }
            | Line::Stop { t, .. } => Some(*t),
            Line::Unknown => None,
        }
    }
}

/// The journal's one writer.
#[derive(Debug)]
pub struct JournalWriter {
    file: File,
}

impl JournalWriter {
    /// Exclusive writer: on Windows share READ | DELETE only, elsewhere a lock on the open file.
    /// Cuts a last line without a newline. `Busy` when another writer holds it. Creates `screen/`
    /// and the journal when missing. Readers take no lock, so `JournalReader` keeps following it.
    pub fn open(meeting: &Path) -> Result<JournalWriter, OpenError> {
        std::fs::create_dir_all(paths::screen_dir(meeting)).map_err(OpenError::Io)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        exclusive_writer(&mut options);
        let mut file = options.open(paths::journal_path(meeting)).map_err(|e| {
            if held_by_another_writer(&e) {
                OpenError::Busy
            } else {
                OpenError::Io(e)
            }
        })?;
        lock_writer(&file).map_err(|e| {
            if held_by_another_writer(&e) {
                OpenError::Busy
            } else {
                OpenError::Io(e)
            }
        })?;
        let mut text = Vec::new();
        file.read_to_end(&mut text).map_err(OpenError::Io)?;
        if text.last().is_some_and(|&b| b != b'\n') {
            // A writer killed mid-write left half a line; readers ignore it, and the next line
            // must not run into it.
            let keep = text.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
            file.set_len(keep as u64).map_err(OpenError::Io)?;
        }
        file.seek(SeekFrom::End(0)).map_err(OpenError::Io)?;
        Ok(JournalWriter { file })
    }

    /// One write, with a trailing newline.
    pub fn write(&mut self, line: &Line) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(line)?;
        bytes.push(b'\n');
        self.file.write_all(&bytes)
    }

    /// Flushes the journal to disk; a failure shows up again at the next write.
    pub fn sync(&mut self) {
        let _ = self.file.sync_data();
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

/// Keeps a second writer off the journal. Windows refused it at open through the share mode;
/// elsewhere an exclusive `flock` on the open file does it, which every process sees and which
/// ends with the process that holds it, also after a hard kill. Readers take no lock.
///
/// `WouldBlock`, what EWOULDBLOCK reads as, means another screen process holds the journal.
#[cfg(unix)]
fn lock_writer(file: &File) -> Result<(), io::Error> {
    use std::os::unix::io::AsRawFd;
    loop {
        // SAFETY: `flock` only reads the descriptor; the lock belongs to the file, not to memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // A signal in the call changes nothing, so try again.
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(not(unix))]
fn lock_writer(_file: &File) -> Result<(), io::Error> {
    Ok(())
}

/// Windows refuses a second writer with a sharing violation at open; Unix with a lock it can't
/// take ([`io::ErrorKind::WouldBlock`]).
fn held_by_another_writer(e: &io::Error) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let sharing_violation = cfg!(windows) && e.raw_os_error() == Some(ERROR_SHARING_VIOLATION);
    sharing_violation || (cfg!(unix) && e.kind() == io::ErrorKind::WouldBlock)
}

#[derive(Debug)]
pub enum OpenError {
    /// Another screen process writes this journal.
    Busy,
    Io(io::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Busy => f.write_str("another screen process writes this journal"),
            OpenError::Io(e) => write!(f, "can't open the screen journal: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Follows a growing journal: each `poll` returns the complete lines written since the last.
#[derive(Debug)]
pub struct JournalReader {
    follower: Follower,
}

impl JournalReader {
    pub fn new(meeting: &Path) -> JournalReader {
        JournalReader {
            follower: Follower::new(paths::journal_path(meeting)),
        }
    }

    /// The complete lines written since the last poll. A missing journal reads as no lines yet,
    /// and a journal replaced by a shorter one, as a restore can do, is read again from its start.
    pub fn poll(&mut self) -> io::Result<Vec<Line>> {
        self.follower.poll()
    }
}

/// Every complete line of the journal. A missing journal reads as no lines.
pub fn read_all(meeting: &Path) -> io::Result<Vec<Line>> {
    follow::read_all(&paths::journal_path(meeting))
}

/// What a journal says about the sessions that wrote it, for `--resume`, `finalize` and the
/// supervisor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    pub sessions: u64,
    /// The highest `t` of any line.
    pub last_seen: u64,
    pub last_video_seq: u64,
    /// The video file without a `video_end`: seq, relative file.
    pub open_video: Option<(u64, String)>,
    /// The reason of a `dropped` line for each part.
    pub dropped_video: Option<String>,
    pub dropped_frames: Option<String>,
    /// The reason of the `stop` line.
    pub stopped: Option<String>,
    /// The `mode` of the last `session` line.
    pub mode: Option<String>,
    /// The `t` of a `lock` without an `unlock` after it.
    pub locked_since: Option<u64>,
    /// The relative files of every `frame` line.
    pub journaled_frames: BTreeSet<String>,
}

impl History {
    pub fn from_lines(lines: &[Line]) -> History {
        let mut h = History::default();
        for line in lines {
            if let Some(t) = line.t() {
                h.last_seen = h.last_seen.max(t);
            }
            match line {
                Line::Session { session, mode, .. } => {
                    h.sessions = h.sessions.max(*session);
                    h.mode = Some(mode.clone());
                }
                Line::Mode { mode, .. } => h.mode = Some(mode.clone()),
                Line::Video { seq, file, .. } => {
                    h.last_video_seq = h.last_video_seq.max(*seq);
                    h.open_video = Some((*seq, file.clone()));
                }
                Line::VideoEnd { seq, .. } => {
                    if h.open_video.as_ref().is_some_and(|(open, _)| open == seq) {
                        h.open_video = None;
                    }
                }
                Line::Frame { file, .. } => {
                    h.journaled_frames.insert(file.clone());
                }
                Line::Dropped { what, reason, .. } => match what.as_str() {
                    "video" => h.dropped_video = Some(reason.clone()),
                    "frames" => h.dropped_frames = Some(reason.clone()),
                    _ => {}
                },
                Line::Lock { t } => {
                    h.locked_since.get_or_insert(*t);
                }
                Line::Unlock { .. } => h.locked_since = None,
                Line::Stop { reason, .. } => h.stopped = Some(reason.clone()),
                _ => {}
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    fn video(t: u64, seq: u64) -> Line {
        Line::Video {
            t,
            seq,
            file: paths::video_file(seq),
            width: 1920,
            height: 1080,
            fps: 15,
            encoder: "NVIDIA H.264 Encoder MFT".into(),
            hardware: true,
        }
    }

    fn frame(t: u64) -> Line {
        Line::Frame {
            t,
            id: paths::frame_id(t),
            file: paths::frame_file(&paths::frame_id(t)),
            width: 1920,
            height: 1080,
            text: String::new(),
            ocr: "none".into(),
            language: None,
        }
    }

    fn session(t: u64, session: u64, mode: &str) -> Line {
        Line::Session {
            t,
            wall: 1_793_000_000_000 + t,
            boot: None,
            session,
            pid: 1,
            resumed: session > 1,
            version: FORMAT_VERSION,
            mode: mode.into(),
            anchor: "wall".into(),
            screen: "0.1.0".into(),
        }
    }

    fn every_kind() -> Vec<Line> {
        vec![
            Line::Session {
                t: 120,
                wall: 1_793_384_409_120,
                boot: Some(123_456_789_012),
                session: 1,
                pid: 18_220,
                resumed: false,
                version: FORMAT_VERSION,
                mode: "video".into(),
                anchor: "boot".into(),
                screen: "0.1.0".into(),
            },
            Line::Monitor {
                t: 121,
                id: r"\\.\DISPLAY2".into(),
                name: "PHL 275V8".into(),
                width: 1440,
                height: 2560,
                primary: false,
                reason: "start".into(),
            },
            Line::Capture {
                t: 180,
                wall: 1_793_384_409_180,
                monitor: r"\\.\DISPLAY2".into(),
                width: 1440,
                height: 2560,
            },
            video(180, 1),
            Line::Frame {
                t: 180,
                id: paths::frame_id(180),
                file: paths::frame_file(&paths::frame_id(180)),
                width: 1440,
                height: 2560,
                text: "Q3 roadmap\nRevenue up".into(),
                ocr: "ok".into(),
                language: Some("nl-NL".into()),
            },
            Line::Heartbeat {
                t: 2_180,
                wall: 1_793_384_411_180,
                boot: None,
                frames: 1,
                video_bytes: 24_497,
            },
            Line::Lock { t: 3_000 },
            Line::Unlock { t: 9_000 },
            Line::DeviceLost { t: 9_500 },
            Line::VideoEnd {
                t: 9_500,
                seq: 1,
                reason: "error".into(),
                bytes: 1_967_880,
            },
            Line::Dropped {
                t: 9_600,
                what: "video".into(),
                reason: "device".into(),
            },
            Line::Crash {
                t: 12_000,
                last_seen: 9_700,
            },
            Line::NoMonitor { t: 13_000 },
            Line::Stop {
                t: 15_000,
                reason: "meeting".into(),
            },
        ]
    }

    #[test]
    fn every_kind_has_its_documented_shape_and_reads_back() {
        let lines = every_kind();
        let text: Vec<String> = lines
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect();
        assert_eq!(
            text[0],
            r#"{"kind":"session","t":120,"wall":1793384409120,"boot":123456789012,"session":1,"pid":18220,"resumed":false,"version":1,"mode":"video","anchor":"boot","screen":"0.1.0"}"#
        );
        assert_eq!(
            text[1],
            r#"{"kind":"monitor","t":121,"id":"\\\\.\\DISPLAY2","name":"PHL 275V8","width":1440,"height":2560,"primary":false,"reason":"start"}"#
        );
        assert_eq!(
            text[3],
            r#"{"kind":"video","t":180,"seq":1,"file":"screen/video/000001.mp4","width":1920,"height":1080,"fps":15,"encoder":"NVIDIA H.264 Encoder MFT","hardware":true}"#
        );
        assert_eq!(
            text[4],
            r#"{"kind":"frame","t":180,"id":"f-000000180","file":"screen/frames/f-000000180.jpg","width":1440,"height":2560,"text":"Q3 roadmap\nRevenue up","ocr":"ok","language":"nl-NL"}"#
        );
        assert_eq!(
            text[5],
            r#"{"kind":"heartbeat","t":2180,"wall":1793384411180,"frames":1,"videoBytes":24497}"#
        );
        assert_eq!(text[8], r#"{"kind":"device_lost","t":9500}"#);
        assert_eq!(
            text[9],
            r#"{"kind":"video_end","t":9500,"seq":1,"reason":"error","bytes":1967880}"#
        );
        assert_eq!(text[11], r#"{"kind":"crash","t":12000,"lastSeen":9700}"#);
        assert_eq!(text[12], r#"{"kind":"no_monitor","t":13000}"#);
        for (line, json) in lines.iter().zip(&text) {
            assert!(json.starts_with(r#"{"kind":""#), "{json}");
            assert_eq!(&serde_json::from_str::<Line>(json).unwrap(), line);
        }
        assert_eq!(lines[0].t(), Some(120));
        assert_eq!(lines.last().unwrap().t(), Some(15_000));
        assert_eq!(Line::Unknown.t(), None);
        // A frame line without a language reads as `None`; new fields are ignored.
        let older: Line = serde_json::from_str(
            r#"{"kind":"frame","t":1,"id":"f-000000001","file":"screen/frames/f-000000001.jpg","width":2,"height":2,"text":"","ocr":"none","colour":"blue"}"#,
        )
        .unwrap();
        assert!(matches!(older, Line::Frame { language: None, .. }));
    }

    #[test]
    fn unknown_kinds_broken_lines_and_a_torn_last_line_are_skipped() {
        let dir = TestDir::new("journal-read");
        std::fs::create_dir_all(paths::screen_dir(dir.path())).unwrap();
        let text = concat!(
            r#"{"kind":"lock","t":5}"#,
            "\n",
            "garbage\n",
            "\n",
            r#"{"kind":"hologram","t":6,"depth":3}"#,
            "\n",
            r#"{"kind":"frame","t":7}"#,
            "\n",
            r#"{"kind":"unlock","t":8}"#,
            "\r\n",
            r#"{"kind":"heartbeat","t":9,"wa"#
        );
        std::fs::write(paths::journal_path(dir.path()), text).unwrap();
        assert_eq!(
            read_all(dir.path()).unwrap(),
            vec![Line::Lock { t: 5 }, Line::Unknown, Line::Unlock { t: 8 }]
        );
        // A complete object without its newline is still a partial line.
        std::fs::write(paths::journal_path(dir.path()), r#"{"kind":"lock","t":5}"#).unwrap();
        assert_eq!(read_all(dir.path()).unwrap(), vec![]);
        assert_eq!(read_all(&dir.path().join("missing")).unwrap(), vec![]);
    }

    #[test]
    fn the_writer_cuts_a_torn_line_and_the_reader_follows() {
        let dir = TestDir::new("journal-write");
        let mut reader = JournalReader::new(dir.path());
        assert_eq!(reader.poll().unwrap(), vec![]);

        let mut w = JournalWriter::open(dir.path()).unwrap();
        w.write(&Line::Lock { t: 1 }).unwrap();
        w.sync();
        assert_eq!(reader.poll().unwrap(), vec![Line::Lock { t: 1 }]);
        assert_eq!(reader.poll().unwrap(), vec![]);
        drop(w);

        // A writer killed mid-write: the reader waits for the line to finish...
        let path = paths::journal_path(dir.path());
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(br#"{"kind":"unlock","t""#).unwrap();
        drop(f);
        assert_eq!(reader.poll().unwrap(), vec![]);
        // ...and the next writer cuts it off before it appends, so nothing is glued to it.
        let mut w = JournalWriter::open(dir.path()).unwrap();
        w.write(&Line::Unlock { t: 2 }).unwrap();
        w.write(&Line::DeviceLost { t: 3 }).unwrap();
        assert_eq!(
            reader.poll().unwrap(),
            vec![Line::Unlock { t: 2 }, Line::DeviceLost { t: 3 }]
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            concat!(
                r#"{"kind":"lock","t":1}"#,
                "\n",
                r#"{"kind":"unlock","t":2}"#,
                "\n",
                r#"{"kind":"device_lost","t":3}"#,
                "\n"
            )
        );
        drop(w);
        // A journal replaced by a shorter one is read again from its start.
        std::fs::write(&path, "{\"kind\":\"lock\",\"t\":4}\n").unwrap();
        assert_eq!(reader.poll().unwrap(), vec![Line::Lock { t: 4 }]);
    }

    #[test]
    fn a_second_writer_is_busy_but_readers_can_read() {
        let dir = TestDir::new("journal-busy");
        let mut w = JournalWriter::open(dir.path()).unwrap();
        w.write(&Line::Lock { t: 1 }).unwrap();
        assert!(matches!(
            JournalWriter::open(dir.path()),
            Err(OpenError::Busy)
        ));
        assert_eq!(read_all(dir.path()).unwrap(), vec![Line::Lock { t: 1 }]);
        drop(w);
        assert!(JournalWriter::open(dir.path()).is_ok());
    }

    #[test]
    fn history_of_a_crashed_journal() {
        // Session 1 recorded video 1, ended it at a pause, started video 2 and died while
        // locked; session 2 resumed and is recording video 3.
        let lines = vec![
            session(0, 1, "video"),
            video(40, 1),
            frame(40),
            Line::VideoEnd {
                t: 5_000,
                seq: 1,
                reason: "pause".into(),
                bytes: 10,
            },
            video(9_000, 2),
            frame(9_100),
            Line::Lock { t: 12_000 },
            Line::Heartbeat {
                t: 13_000,
                wall: 0,
                boot: None,
                frames: 2,
                video_bytes: 20,
            },
            Line::VideoEnd {
                t: 13_500,
                seq: 2,
                reason: "crash".into(),
                bytes: 15,
            },
            session(20_000, 2, "video"),
            Line::Crash {
                t: 20_000,
                last_seen: 13_500,
            },
            Line::Dropped {
                t: 20_000,
                what: "video".into(),
                reason: "disk".into(),
            },
            video(20_100, 3),
            frame(20_200),
            Line::Unknown,
        ];
        let h = History::from_lines(&lines);
        assert_eq!(h.sessions, 2);
        assert_eq!(h.last_seen, 20_200);
        assert_eq!(h.last_video_seq, 3);
        assert_eq!(h.open_video, Some((3, paths::video_file(3))));
        assert_eq!(h.dropped_video.as_deref(), Some("disk"));
        assert_eq!(h.dropped_frames, None);
        assert_eq!(h.stopped, None);
        assert_eq!(h.mode.as_deref(), Some("video"));
        assert_eq!(h.locked_since, Some(12_000));
        assert_eq!(
            h.journaled_frames,
            [40, 9_100, 20_200]
                .into_iter()
                .map(|t| paths::frame_file(&paths::frame_id(t)))
                .collect()
        );

        // The dead session's open file: a crash before its repair leaves it open.
        let h = History::from_lines(&lines[..8]);
        assert_eq!(h.open_video, Some((2, paths::video_file(2))));
        assert_eq!(h.sessions, 1);

        let mut done = lines.clone();
        done.extend([
            Line::Unlock { t: 21_000 },
            Line::VideoEnd {
                t: 22_000,
                seq: 3,
                reason: "stop".into(),
                bytes: 30,
            },
            Line::Dropped {
                t: 22_000,
                what: "frames".into(),
                reason: "disk".into(),
            },
            Line::Stop {
                t: 22_000,
                reason: "dropped".into(),
            },
        ]);
        let h = History::from_lines(&done);
        assert_eq!(h.open_video, None);
        assert_eq!(h.locked_since, None);
        assert_eq!(h.dropped_frames.as_deref(), Some("disk"));
        assert_eq!(h.stopped.as_deref(), Some("dropped"));
        assert_eq!(History::from_lines(&[]), History::default());
    }
}
