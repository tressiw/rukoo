//! `rukoo-voice-screen finalize`, and the repairs a resumed session makes first: the video file
//! a dead session left open is cut after its last complete fragment and gets `video_end crash`,
//! and JPEGs it saved without a `frame` line are read and journaled.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use voice_screen_files::journal::{self, History, JournalWriter, Line, OpenError};
use voice_screen_files::mp4::{self, Repaired};
use voice_screen_files::paths;
use voice_screen_files::screenfile;

use crate::platform::{ImageText, Platform, Reader};
use crate::{log, ExitError};

/// What `finalize` did. `stopped` says whether it wrote `stop` now; `t` is the `stop` line's time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finalized {
    pub stopped: bool,
    pub t: u64,
    pub repaired: Option<Repaired>,
}

impl Finalized {
    pub fn to_json(&self) -> Value {
        json!({
            "ok": true,
            "stopped": self.stopped,
            "t": self.t,
            "repaired": self.repaired.map(|r| json!({ "kept": r.kept, "cut": r.cut })),
        })
    }
}

/// Finishes the screen journal of a meeting whose screen process died: repairs the open video
/// file, journals orphan frames and writes `stop` at the last moment the journal or the video
/// reached. Running it again changes nothing. It refuses a live screen process with exit code 4.
pub fn finalize<P: Platform>(
    platform: &P,
    meeting: &Path,
    reason: &str,
) -> Result<Finalized, ExitError> {
    platform.init();
    if !meeting.is_dir() {
        return Err(ExitError::usage(format!(
            "meeting folder {} doesn't exist",
            meeting.display()
        )));
    }
    if screenfile::live(meeting).is_some() {
        return Err(ExitError::busy());
    }
    let lines = journal::read_all(meeting)
        .map_err(|e| ExitError::failed(format!("can't read the screen journal: {e}")))?;
    let history = History::from_lines(&lines);
    if history.stopped.is_some() {
        screenfile::remove(meeting);
        let t = lines
            .iter()
            .rev()
            .find_map(|l| match l {
                Line::Stop { t, .. } => Some(*t),
                _ => None,
            })
            .unwrap_or(history.last_seen);
        return Ok(Finalized {
            stopped: false,
            t,
            repaired: None,
        });
    }
    let mut journal = match JournalWriter::open(meeting) {
        Ok(journal) => journal,
        Err(OpenError::Busy) => return Err(ExitError::busy()),
        Err(OpenError::Io(e)) => {
            return Err(ExitError::failed(format!(
                "can't open the screen journal: {e}"
            )))
        }
    };
    let (repaired, video_end) = close_open_video(meeting, &mut journal, &lines, &history);
    let reader = platform.reader();
    let orphans = journal_orphans(meeting, &mut journal, &history, reader.as_ref());
    remove_temp_files(meeting);
    let t = history.last_seen.max(video_end).max(orphans);
    journal
        .write(&Line::Stop {
            t,
            reason: reason.into(),
        })
        .map_err(|e| ExitError::failed(format!("can't write the screen journal: {e}")))?;
    journal.sync();
    screenfile::remove(meeting);
    Ok(Finalized {
        stopped: true,
        t,
        repaired,
    })
}

/// Cuts the video file a dead session left open after its last complete fragment and writes
/// its `video_end` with reason `crash`. Returns the repair and the meeting time where the
/// file's video ends (0 without an open file).
pub(crate) fn close_open_video(
    meeting: &Path,
    journal: &mut JournalWriter,
    lines: &[Line],
    history: &History,
) -> (Option<Repaired>, u64) {
    let Some((seq, file)) = history.open_video.clone() else {
        return (None, 0);
    };
    let start = lines
        .iter()
        .rev()
        .find_map(|l| match l {
            Line::Video { seq: s, t, .. } if *s == seq => Some(*t),
            _ => None,
        })
        .unwrap_or(history.last_seen);
    // The repair truncates the file, so only a path of a video file's own shape gets it: a
    // damaged journal mustn't make it cut the audio or anything else in the meeting folder.
    let Some(path) = paths::video_path(meeting, &file) else {
        log(format_args!(
            "video {seq}: the journal names {file:?}, which isn't a video file; left as it is"
        ));
        let _ = journal.write(&Line::VideoEnd {
            t: start,
            seq,
            reason: "crash".into(),
            bytes: 0,
        });
        return (None, start);
    };
    let repaired = match mp4::repair(&path) {
        Ok(repaired) => Some(repaired),
        Err(e) => {
            log(format_args!("video {seq} couldn't be repaired: {e}"));
            None
        }
    };
    let end = match mp4::index_path(&path) {
        Ok(index) => start + media_ms(&index),
        Err(_) => start,
    };
    let bytes = fs::metadata(&path).map_or(0, |m| m.len());
    if let Some(r) = repaired {
        log(format_args!(
            "video {seq}: kept {} bytes, cut {} after the last complete fragment",
            r.kept, r.cut
        ));
    }
    let _ = journal.write(&Line::VideoEnd {
        t: end,
        seq,
        reason: "crash".into(),
        bytes,
    });
    (repaired, end)
}

/// The media time an index covers, in ms.
pub fn media_ms(index: &mp4::Index) -> u64 {
    match index.fragments.last() {
        Some(last) if index.timescale > 0 => index.to_ms(last.end()),
        _ => 0,
    }
}

/// Journals the JPEGs in `screen/frames/` that no `frame` line names: their text is read now
/// and `t` comes from the name. Returns the highest such `t`, or 0.
pub(crate) fn journal_orphans(
    meeting: &Path,
    journal: &mut JournalWriter,
    history: &History,
    reader: &dyn Reader,
) -> u64 {
    let dir = paths::screen_dir(meeting).join("frames");
    let Ok(entries) = fs::read_dir(&dir) else {
        return 0;
    };
    let mut orphans: Vec<(u64, String)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let id = name.strip_suffix(".jpg")?;
            let t: u64 = id.strip_prefix("f-")?.parse().ok()?;
            (paths::frame_id(t) == id).then(|| (t, id.to_string()))
        })
        .filter(|(_, id)| !history.journaled_frames.contains(&paths::frame_file(id)))
        .collect();
    orphans.sort();
    let mut last = 0;
    for (t, id) in orphans {
        let file = paths::frame_file(&id);
        let Ok(bytes) = fs::read(meeting.join(&file)) else {
            log(format_args!("frame {id} doesn't open; left out"));
            continue;
        };
        let Ok((text, (width, height))) = reader.read_image(&bytes) else {
            log(format_args!("frame {id} doesn't open; left out"));
            continue;
        };
        let (text, status) = match text {
            ImageText::Text(lines) => (voice_screen_files::frames::clean_text(&lines), "ok"),
            ImageText::None => (String::new(), "none"),
            ImageText::Failed => (String::new(), "failed"),
        };
        let _ = journal.write(&Line::Frame {
            t,
            id,
            file,
            width,
            height,
            text,
            ocr: status.into(),
            language: reader.language(),
        });
        last = last.max(t);
    }
    last
}

/// Removes `*.tmp` files a dead session left in `screen/frames/` and `screen/video/`.
pub(crate) fn remove_temp_files(meeting: &Path) {
    for sub in ["frames", "video"] {
        let Ok(entries) = fs::read_dir(paths::screen_dir(meeting).join(sub)) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.file_name().to_string_lossy().ends_with(".tmp") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Whether `screen/` holds a video file or a frame, with or without journal lines: a new
/// recording must not start over it without `--resume` (`voice/RECORDING.md`).
pub fn has_media(meeting: &Path) -> bool {
    [("video", ".mp4"), ("frames", ".jpg")]
        .iter()
        .any(|(sub, extension)| {
            fs::read_dir(paths::screen_dir(meeting).join(sub)).is_ok_and(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .any(|e| e.file_name().to_string_lossy().ends_with(extension))
            })
        })
}

/// The highest video file number on disk, also of files a crash left without a `video` line,
/// so a new file never collides with one.
pub fn highest_video_seq(meeting: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(paths::screen_dir(meeting).join("video")) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .into_string()
                .ok()?
                .strip_suffix(".mp4")?
                .parse::<u64>()
                .ok()
        })
        .max()
        .unwrap_or(0)
}
