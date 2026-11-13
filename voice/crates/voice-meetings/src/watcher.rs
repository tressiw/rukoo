//! Follows `timeline.jsonl` while the recorder writes it, and turns its lines into the
//! `meeting.state` and `mark` events of PROTOCOL.md.
//!
//! Every derived event has a key that depends only on the timeline, so a watcher that starts over
//! after a restart derives the same events with the same keys, and the pipeline sends only the
//! ones it hadn't sent yet.
//!
//! Mark ids: user marks keep the recorder's `k-<n>`. Derived marks use `gap-<track>-<at>`,
//! `dev-<track>-<at>`, `silent-<track>-<at>` and `dict-<at>`, and `gap-all-<at>` for a gap in every track (a crash, a
//! suspend), so they never collide with user marks and stay the same across restarts.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use voice_protocol::{Mark, MarkKind, MarkTrack, MeetingState, MeetingStateChange, Track};
use voice_recorder::timeline::{Line, SegmentRecord, FILE_NAME};

/// How the recording ended: the timeline's `stop` line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub t: u64,
    pub wall: u64,
    pub reason: String,
}

/// A stretch of dictation on the microphone, in meeting milliseconds. `end` is `None` while the
/// user still dictates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub start: u64,
    pub end: Option<u64>,
}

impl Stretch {
    /// Whether the stretch overlaps `[start, end]`.
    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        start < self.end.unwrap_or(u64::MAX) && end > self.start
    }
}

/// An event derived from the timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Derived {
    State(MeetingStateChange),
    Mark(Mark),
}

impl Derived {
    /// The key the pipeline remembers once the event is in the event log.
    pub fn key(&self) -> String {
        match self {
            Derived::State(s) => state_key(s.state, s.at),
            Derived::Mark(m) => mark_key(&m.id, m.kind.as_str(), m.until),
        }
    }
}

pub(crate) fn state_key(state: MeetingState, at: u64) -> String {
    format!("state:{state}:{at}")
}

/// The `kind` of the mark that says a call track held only silence (PROTOCOL.md).
pub(crate) const SILENT: &str = "silent";

/// A mark is sent once, except a `silent` mark, which is sent again with its end when the call
/// had sound; that second one has a key of its own.
pub(crate) fn mark_key(id: &str, kind: &str, until: Option<u64>) -> String {
    match until {
        Some(until) if kind == SILENT => format!("mark:{id}:until:{until}"),
        _ => format!("mark:{id}"),
    }
}

/// Turns timeline lines into events, one line at a time, and keeps what the transcriber needs to
/// know about the recording: its segments, the dictation stretches, where it ended.
#[derive(Clone, Debug, Default)]
pub struct Mapper {
    state: Option<MeetingState>,
    stop: Option<Stop>,
    dictating_since: Option<u64>,
    stretches: Vec<Stretch>,
    /// Tracks without a device, since when.
    lost: BTreeMap<String, u64>,
    /// The `silent` mark each track has open, until its sound.
    silent: BTreeMap<String, Mark>,
    /// The device each track last opened.
    devices: BTreeMap<String, String>,
    ids: HashSet<String>,
    segments: Vec<SegmentRecord>,
    latest_t: u64,
    lines: usize,
}

impl Mapper {
    /// The events `line` completes, in order.
    pub fn feed(&mut self, line: &Line) -> Vec<Derived> {
        self.lines += 1;
        // Nothing follows `stop`; a line that does anyway can't change the recording.
        if self.stop.is_some() {
            return Vec::new();
        }
        if let Some(t) = line.t() {
            self.latest_t = self.latest_t.max(t);
        }
        let mut out = Vec::new();
        match line {
            Line::Session { t, .. } => {
                if self.state.is_none() {
                    self.set_state(&mut out, MeetingState::Recording, *t);
                }
            }
            Line::Track { track, device, .. } => {
                self.devices.insert(track.clone(), device.clone());
            }
            Line::Segment {
                t,
                track,
                seq,
                file,
            } => {
                self.segments.push(SegmentRecord {
                    track: track.clone(),
                    seq: *seq,
                    t: *t,
                    file: file.clone(),
                    ended: None,
                });
                // A gap after a lost device ends when the track has audio again.
                if let Some(at) = self.lost.remove(track) {
                    self.mark(&mut out, gap(track_of(track), at, *t, "device", track));
                }
            }
            Line::SegmentEnd {
                track,
                seq,
                samples,
                ..
            } => {
                if let Some(s) = self
                    .segments
                    .iter_mut()
                    .rev()
                    .find(|s| s.track == *track && s.seq == *seq)
                {
                    s.ended = Some(*samples);
                }
            }
            Line::DeviceLost { t, track, .. } => {
                if !self.lost.contains_key(track) {
                    self.lost.insert(track.clone(), *t);
                    let label = self.devices.get(track).cloned();
                    self.mark(&mut out, device(track, *t, "lost", label));
                }
            }
            Line::Device {
                t,
                track,
                device: name,
                reason,
            } => {
                self.devices.insert(track.clone(), name.clone());
                self.mark(&mut out, device(track, *t, reason, Some(name.clone())));
            }
            Line::Pause { t } => {
                if self.state != Some(MeetingState::Paused) {
                    self.set_state(&mut out, MeetingState::Paused, *t);
                }
            }
            Line::Resume { t } => {
                if self.state == Some(MeetingState::Paused) {
                    self.set_state(&mut out, MeetingState::Recording, *t);
                }
            }
            Line::Mark { t, id, label } => {
                self.mark(
                    &mut out,
                    Mark {
                        id: id.clone(),
                        kind: MarkKind::User,
                        at: *t,
                        until: None,
                        track: None,
                        detail: None,
                        label: label.clone(),
                    },
                );
            }
            Line::Dictation { t, state } => match state.as_str() {
                "start" => {
                    self.dictating_since.get_or_insert(*t);
                }
                "end" => self.end_dictation(&mut out, *t),
                _ => {}
            },
            Line::Suspend { t, until } => {
                self.mark(&mut out, gap(None, *t, *until, "suspend", "all"));
            }
            Line::Crash { t, last_seen } => {
                // The recorder died: whatever the app said about dictation went with it.
                self.end_dictation(&mut out, *last_seen);
                self.mark(&mut out, gap(None, *last_seen, *t, "crash", "all"));
            }
            Line::Stop { t, wall, reason } => {
                let lost = std::mem::take(&mut self.lost);
                for (track, at) in lost {
                    self.mark(&mut out, gap(track_of(&track), at, *t, "device", &track));
                }
                self.end_dictation(&mut out, *t);
                let reason = if reason.is_empty() {
                    "interrupted".to_owned()
                } else {
                    reason.clone()
                };
                self.state = Some(MeetingState::Processing);
                out.push(Derived::State(MeetingStateChange {
                    state: MeetingState::Processing,
                    at: *t,
                    ended_at: Some(*wall),
                    reason: Some(reason.clone()),
                }));
                self.stop = Some(Stop {
                    t: *t,
                    wall: *wall,
                    reason,
                });
            }
            // The call track records only exact zeros while another app plays sound: macOS may
            // not let Voice record system audio. Rukoo hears of it at once, not at the end.
            Line::Silent { track, since, .. } => {
                let mark = Mark {
                    id: format!("silent-{track}-{since}"),
                    kind: MarkKind::Other(SILENT.into()),
                    at: *since,
                    until: None,
                    track: track_of(track),
                    detail: None,
                    label: None,
                };
                self.mark(&mut out, mark);
                if let Some(Derived::Mark(sent)) = out.last() {
                    self.silent.insert(track.clone(), sent.clone());
                }
            }
            // The call had sound after all: the same mark again, now with its end, which
            // replaces the first in Rukoo.
            Line::Sound { t, track } => {
                if let Some(mut mark) = self.silent.remove(track) {
                    mark.until = Some((*t).max(mark.at));
                    out.push(Derived::Mark(mark));
                }
            }
            Line::Heartbeat { .. } | Line::Unknown => {}
        }
        out
    }

    fn set_state(&mut self, out: &mut Vec<Derived>, state: MeetingState, at: u64) {
        self.state = Some(state);
        out.push(Derived::State(MeetingStateChange {
            state,
            at,
            ended_at: None,
            reason: None,
        }));
    }

    fn end_dictation(&mut self, out: &mut Vec<Derived>, t: u64) {
        let Some(start) = self.dictating_since.take() else {
            return;
        };
        let end = t.max(start);
        self.stretches.push(Stretch {
            start,
            end: Some(end),
        });
        self.mark(
            out,
            Mark {
                id: format!("dict-{start}"),
                kind: MarkKind::Dictation,
                at: start,
                until: Some(end),
                track: Some(MarkTrack::Mic),
                detail: None,
                label: None,
            },
        );
    }

    /// Adds a mark, giving it a fresh id should two derived marks ever land on the same one.
    fn mark(&mut self, out: &mut Vec<Derived>, mut mark: Mark) {
        if !self.ids.insert(mark.id.clone()) {
            let base = mark.id.clone();
            let mut n = 2;
            while !self.ids.insert(format!("{base}-{n}")) {
                n += 1;
            }
            mark.id = format!("{base}-{n}");
        }
        out.push(Derived::Mark(mark));
    }

    /// The meeting's state as far as the recording goes: `None` before the first session.
    pub fn state(&self) -> Option<MeetingState> {
        self.state
    }

    pub fn stop(&self) -> Option<&Stop> {
        self.stop.as_ref()
    }

    /// The dictation stretches so far, the open one last.
    pub fn stretches(&self) -> Vec<Stretch> {
        let mut all = self.stretches.clone();
        if let Some(start) = self.dictating_since {
            all.push(Stretch { start, end: None });
        }
        all
    }

    /// When the dictation that is going on now started.
    pub fn dictating_since(&self) -> Option<u64> {
        self.dictating_since
    }

    pub fn segments(&self) -> &[SegmentRecord] {
        &self.segments
    }

    /// The highest meeting time in the timeline so far.
    pub fn latest_t(&self) -> u64 {
        self.latest_t
    }

    /// Lines read so far, including ones that didn't parse as a known kind.
    pub fn lines(&self) -> usize {
        self.lines
    }
}

fn track_of(track: &str) -> Option<MarkTrack> {
    MarkTrack::parse(track)
}

fn gap(track: Option<MarkTrack>, at: u64, until: u64, detail: &str, name: &str) -> Mark {
    Mark {
        id: format!("gap-{name}-{at}"),
        kind: MarkKind::Gap,
        at,
        until: Some(until.max(at)),
        track,
        detail: Some(detail.to_owned()),
        label: None,
    }
}

fn device(track: &str, at: u64, detail: &str, label: Option<String>) -> Mark {
    Mark {
        id: format!("dev-{track}-{at}"),
        kind: MarkKind::Device,
        at,
        until: None,
        track: track_of(track),
        detail: Some(detail.to_owned()),
        label,
    }
}

/// Every event a whole timeline gives, in order.
pub fn derive(lines: &[Line]) -> (Mapper, Vec<Derived>) {
    let mut mapper = Mapper::default();
    let events = lines.iter().flat_map(|line| mapper.feed(line)).collect();
    (mapper, events)
}

/// Reads the lines the recorder adds to a timeline, from where it left off.
#[derive(Debug)]
pub struct TimelineWatcher {
    path: PathBuf,
    /// Bytes of complete lines read so far.
    offset: u64,
    mapper: Mapper,
}

impl TimelineWatcher {
    pub fn new(meeting_dir: &Path) -> TimelineWatcher {
        TimelineWatcher {
            path: meeting_dir.join(FILE_NAME),
            offset: 0,
            mapper: Mapper::default(),
        }
    }

    /// Reads the complete lines written since the last call and returns the events they give.
    /// A last line without its newline waits for the next call.
    pub fn poll(&mut self) -> io::Result<Vec<Derived>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else {
            return Ok(Vec::new());
        };
        self.offset += end as u64 + 1;
        let mut out = Vec::new();
        for raw in bytes[..end].split(|&b| b == b'\n') {
            // A line that doesn't parse is skipped, as voice/RECORDING.md asks of readers.
            if let Ok(line) = serde_json::from_slice::<Line>(raw) {
                out.extend(self.mapper.feed(&line));
            }
        }
        Ok(out)
    }

    pub fn mapper(&self) -> &Mapper {
        &self.mapper
    }
}

/// The tracks a meeting names, as protocol tracks.
pub(crate) fn tracks_of(names: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<Track> {
    let mut tracks: Vec<Track> = Vec::new();
    for name in names {
        if let Some(track) = Track::parse(name.as_ref()) {
            if !tracks.contains(&track) {
                tracks.push(track);
            }
        }
    }
    tracks.sort();
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn lines(values: &[serde_json::Value]) -> Vec<Line> {
        values
            .iter()
            .map(|v| serde_json::from_value(v.clone()).unwrap())
            .collect()
    }

    fn session(t: u64, n: u64) -> serde_json::Value {
        json!({"kind":"session","t":t,"wall":1_793_000_000_000u64 + t,"session":n,"pid":1,"resumed":n > 1,"version":1})
    }

    fn marks(events: &[Derived]) -> Vec<&Mark> {
        events
            .iter()
            .filter_map(|e| match e {
                Derived::Mark(m) => Some(m),
                _ => None,
            })
            .collect()
    }

    fn states(events: &[Derived]) -> Vec<(MeetingState, u64)> {
        events
            .iter()
            .filter_map(|e| match e {
                Derived::State(s) => Some((s.state, s.at)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_lost_device_gives_a_device_mark_then_a_gap_when_audio_returns() {
        let (_, events) = derive(&lines(&[
            session(0, 1),
            json!({"kind":"track","t":3,"track":"mic","source":"default-input","device":"USB mic","deviceId":"x","nativeRate":48000,"nativeChannels":2}),
            json!({"kind":"segment","t":5,"track":"mic","seq":1,"file":"audio/mic/000001.wav"}),
            json!({"kind":"segment_end","t":1985,"track":"mic","seq":1,"samples":31_680}),
            json!({"kind":"device_lost","t":2002,"track":"mic","error":"gone"}),
            // Retries that fail again don't repeat the mark.
            json!({"kind":"device_lost","t":3002,"track":"mic","error":"gone"}),
            json!({"kind":"device","t":4076,"track":"mic","device":"USB mic","reason":"reconnected"}),
            json!({"kind":"segment","t":4089,"track":"mic","seq":2,"file":"audio/mic/000002.wav"}),
            json!({"kind":"device","t":6000,"track":"call","device":"Speakers","reason":"default"}),
        ]));
        let m = marks(&events);
        assert_eq!(m.len(), 4, "{m:#?}");
        assert_eq!(
            *m[0],
            Mark {
                id: "dev-mic-2002".into(),
                kind: MarkKind::Device,
                at: 2002,
                until: None,
                track: Some(MarkTrack::Mic),
                detail: Some("lost".into()),
                label: Some("USB mic".into()),
            }
        );
        assert_eq!(m[1].id, "dev-mic-4076");
        assert_eq!(m[1].detail.as_deref(), Some("reconnected"));
        assert_eq!(
            *m[2],
            Mark {
                id: "gap-mic-2002".into(),
                kind: MarkKind::Gap,
                at: 2002,
                until: Some(4089),
                track: Some(MarkTrack::Mic),
                detail: Some("device".into()),
                label: None,
            }
        );
        assert_eq!(m[3].id, "dev-call-6000");
        assert_eq!(m[3].label.as_deref(), Some("Speakers"));
        assert_eq!(states(&events), vec![(MeetingState::Recording, 0)]);
    }

    #[test]
    fn a_silent_call_gives_a_mark_at_once_and_its_sound_ends_it() {
        let (_, events) = derive(&lines(&[
            session(0, 1),
            json!({"kind":"track","t":20,"track":"call","source":"default-output","device":"Mac mini Speakers","deviceId":"coreaudio:BuiltInSpeakerDevice","nativeRate":48000,"nativeChannels":2}),
            json!({"kind":"segment","t":30,"track":"call","seq":1,"file":"audio/call/000001.wav"}),
            json!({"kind":"silent","t":30_040,"track":"call","since":30}),
            json!({"kind":"sound","t":41_000,"track":"call"}),
        ]));
        let m = marks(&events);
        let first = Mark {
            id: "silent-call-30".into(),
            kind: MarkKind::Other("silent".into()),
            at: 30,
            until: None,
            track: Some(MarkTrack::Call),
            detail: None,
            label: None,
        };
        let ended = Mark {
            until: Some(41_000),
            ..first.clone()
        };
        assert_eq!(m, vec![&first, &ended]);
        // Two events with one id: the pipeline keeps a key for each.
        let keys: Vec<String> = events.iter().map(Derived::key).collect();
        assert!(keys.contains(&"mark:silent-call-30".to_string()));
        assert!(keys.contains(&"mark:silent-call-30:until:41000".to_string()));
    }

    #[test]
    fn crash_and_suspend_are_gaps_in_the_whole_meeting() {
        let (mapper, events) = derive(&lines(&[
            session(0, 1),
            json!({"kind":"dictation","t":900,"state":"start"}),
            json!({"kind":"heartbeat","t":1000,"wall":1,"samples":{}}),
            json!({"kind":"suspend","t":1000,"until":61_000}),
            json!({"kind":"segment_end","t":1500,"track":"mic","seq":1,"samples":0}),
            session(70_000, 2),
            json!({"kind":"crash","t":70_000,"lastSeen":62_300}),
        ]));
        let m = marks(&events);
        assert_eq!(m.len(), 3, "{m:#?}");
        assert_eq!(m[0].id, "gap-all-1000");
        assert_eq!(
            (m[0].until, m[0].track, m[0].detail.as_deref()),
            (Some(61_000), None, Some("suspend"))
        );
        // The crash ends the dictation that was going on when the recorder died.
        assert_eq!(m[1].id, "dict-900");
        assert_eq!(m[1].until, Some(62_300));
        assert_eq!(m[2].id, "gap-all-62300");
        assert_eq!(
            (m[2].at, m[2].until, m[2].detail.as_deref()),
            (62_300, Some(70_000), Some("crash"))
        );
        // A resumed session doesn't announce recording again.
        assert_eq!(states(&events), vec![(MeetingState::Recording, 0)]);
        assert_eq!(mapper.latest_t(), 70_000);
    }

    #[test]
    fn dictation_pause_resume_marks_and_stop() {
        let (mapper, events) = derive(&lines(&[
            session(0, 1),
            json!({"kind":"dictation","t":1000,"state":"start"}),
            json!({"kind":"dictation","t":1100,"state":"start"}),
            json!({"kind":"dictation","t":3000,"state":"end"}),
            json!({"kind":"dictation","t":3100,"state":"end"}),
            json!({"kind":"mark","t":3500,"id":"k-1","label":"Decision"}),
            json!({"kind":"pause","t":4000}),
            json!({"kind":"pause","t":4100}),
            json!({"kind":"mark","t":4500,"id":"k-2","label":null}),
            json!({"kind":"resume","t":5000}),
            json!({"kind":"resume","t":5100}),
            json!({"kind":"dictation","t":6000,"state":"start"}),
            json!({"kind":"device_lost","t":6500,"track":"call","error":"gone"}),
            json!({"kind":"stop","t":7000,"wall":1_793_000_007_000u64,"reason":"rukoo"}),
            json!({"kind":"mark","t":8000,"id":"k-3","label":null}),
        ]));
        assert_eq!(
            states(&events),
            vec![
                (MeetingState::Recording, 0),
                (MeetingState::Paused, 4000),
                (MeetingState::Recording, 5000),
                (MeetingState::Processing, 7000),
            ]
        );
        let ids: Vec<&str> = marks(&events).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "dict-1000",
                "k-1",
                "k-2",
                "dev-call-6500",
                "gap-call-6500",
                "dict-6000"
            ]
        );
        let m = marks(&events);
        assert_eq!((m[0].until, m[0].track), (Some(3000), Some(MarkTrack::Mic)));
        assert_eq!(m[1].kind, MarkKind::User);
        assert_eq!(m[1].label.as_deref(), Some("Decision"));
        assert_eq!(m[4].until, Some(7000), "the meeting's end closes a gap");
        assert_eq!(m[5].until, Some(7000), "and a dictation");
        let Some(Derived::State(processing)) = events.last() else {
            panic!("ends with the state");
        };
        assert_eq!(processing.ended_at, Some(1_793_000_007_000));
        assert_eq!(processing.reason.as_deref(), Some("rukoo"));
        assert_eq!(mapper.stop().unwrap().t, 7000);
        assert_eq!(
            mapper.stretches(),
            vec![
                Stretch {
                    start: 1000,
                    end: Some(3000)
                },
                Stretch {
                    start: 6000,
                    end: Some(7000)
                }
            ]
        );
    }

    #[test]
    fn derived_ids_never_collide() {
        let (_, events) = derive(&lines(&[
            session(0, 1),
            json!({"kind":"device_lost","t":500,"track":"mic","error":"gone"}),
            json!({"kind":"device","t":500,"track":"mic","device":"B","reason":"default"}),
        ]));
        let ids: Vec<String> = marks(&events).iter().map(|m| m.id.clone()).collect();
        assert_eq!(ids, vec!["dev-mic-500", "dev-mic-500-2"]);
        let keys: HashSet<String> = events.iter().map(Derived::key).collect();
        assert_eq!(keys.len(), events.len());
    }

    #[test]
    fn the_watcher_reads_complete_lines_as_they_arrive() {
        let tmp = crate::test_support::TempDir::new("watcher");
        let path = tmp.path().join(FILE_NAME);
        let mut w = TimelineWatcher::new(tmp.path());
        assert!(w.poll().unwrap().is_empty(), "no timeline yet");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(session(0, 1).to_string().as_bytes())
            .unwrap();
        assert!(
            w.poll().unwrap().is_empty(),
            "a line without its newline waits"
        );
        file.write_all(b"\n{\"kind\":\"pause\",\"t\":40").unwrap();
        assert_eq!(
            states(&w.poll().unwrap()),
            vec![(MeetingState::Recording, 0)]
        );
        file.write_all(b"00}\nnot json\n").unwrap();
        assert_eq!(
            states(&w.poll().unwrap()),
            vec![(MeetingState::Paused, 4000)]
        );
        assert!(w.poll().unwrap().is_empty());
        assert_eq!(w.mapper().lines(), 2);
    }
}
