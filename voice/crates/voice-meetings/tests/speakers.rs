//! Speaker labels in the pipeline, with a scripted diarizer and a fake realtime stream instead of
//! the NeMo worker and Mistral: ids per voice, the gate, stable ids across worker crashes and app
//! restarts, failures that never cost transcription, backfill, padding at gaps, Mistral's
//! consent and reconnects, and meetings from Voice 0.1.
//!
//! The calls are synthetic: each hidden speaker is a square wave of its own loudness, which the
//! scripted diarizer and the fake stream hear, and which the test knows from the script.

mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::score::{accuracy, Span};
use common::*;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use voice_bridge::{EventLog, Stored};
use voice_core::speakers::DiarErrorKind;
use voice_core::{Hints, ProviderError};
use voice_meetings::transcript::{DIARIZE_KEY, LABELLED_KEY, MODE_KEY, REASON_KEY, SETTLED_KEY};
use voice_meetings::{
    FollowerTiming, LabelState, Labels, Library, MeetingDir, Pipeline, PipelineHandle,
    PipelineOptions, PipelineState, StreamOptions, TimedSpeechToText, TimedText, Timing,
    Transcript,
};
use voice_protocol::{Passage, ScreenMode, Track};

const WAIT: Duration = Duration::from_secs(60);

fn fast() -> Timing {
    Timing {
        poll: Duration::from_millis(10),
        busy_poll: Duration::from_millis(1),
        retry_first: Duration::from_millis(10),
        retry_max: Duration::from_millis(40),
        attempts_after_stop: 3,
    }
}

fn follower_timing() -> FollowerTiming {
    FollowerTiming {
        poll: Duration::from_millis(5),
        backoff: vec![Duration::from_millis(10)],
        stream_speed: 1000.0,
        stream_retry_first: Duration::from_millis(10),
        stream_retry_max: Duration::from_millis(40),
        stream_drain: Duration::from_secs(2),
        ..FollowerTiming::default()
    }
}

/// Orukeet-like word timings from [`Bursts`], labels from a scripted diarizer.
fn labelled(d: &ScriptedDiarizer) -> PipelineOptions {
    PipelineOptions {
        timed: Some(Arc::new(Bursts)),
        timing: fast(),
        labels: Labels::Local {
            diarizer: Arc::new(d.clone()),
        },
        speakers: follower_timing(),
        ..PipelineOptions::new(Arc::new(Offline))
    }
}

/// Three voices take turns, each burst `talk` s long with 1.2 s pauses between them.
fn three_voices(rounds: usize, talk: f64) -> Vec<(f64, Option<usize>)> {
    let order = [0, 1, 2, 0, 2, 1];
    let mut pieces = vec![(0.5, None)];
    for i in 0..rounds {
        pieces.push((talk, Some(order[i % order.len()])));
        pieces.push((1.2, None));
    }
    pieces
}

fn seconds(pieces: &[(f64, Option<usize>)]) -> f64 {
    pieces.iter().map(|p| p.0).sum()
}

fn ms(seconds: f64) -> u64 {
    (seconds * 1000.0).round() as u64
}

/// A stopped recording: a quiet microphone and `pieces` on the call.
fn finished_call(lib: &Library, pieces: &[(f64, Option<usize>)]) -> Recording {
    let rec = Recording::new(lib, &[Track::Mic, Track::Call]);
    rec.session(0, 1);
    let length = seconds(pieces);
    rec.segment("mic", 1, 0, &voices(&[(length, None)]));
    rec.segment("call", 1, 0, &voices(pieces));
    rec.stop(ms(length));
    rec
}

#[derive(Clone, Debug, PartialEq)]
struct Found {
    id: String,
    track: String,
    start: u64,
    end: u64,
    speaker: Option<String>,
    revision: u64,
    text: String,
}

fn found(p: &Value) -> Found {
    Found {
        id: p["id"].as_str().unwrap().to_owned(),
        track: p["track"].as_str().unwrap().to_owned(),
        start: p["start"].as_u64().unwrap(),
        end: p["end"].as_u64().unwrap(),
        speaker: p["speaker"].as_str().map(str::to_owned),
        revision: p["revision"].as_u64().unwrap(),
        text: p["text"].as_str().unwrap().to_owned(),
    }
}

/// Every passage event in order.
fn all_passages(events: &[Stored]) -> Vec<Found> {
    of(events, "passage").into_iter().map(found).collect()
}

/// Each passage's last revision on `track`, by start.
fn last_revisions(events: &[Stored], track: &str) -> Vec<Found> {
    let mut last: HashMap<String, Found> = HashMap::new();
    for p in all_passages(events) {
        if p.track == track {
            last.insert(p.id.clone(), p);
        }
    }
    let mut list: Vec<Found> = last.into_values().collect();
    list.sort_by_key(|p| p.start);
    list
}

/// Every named call passage belongs to one hidden speaker, and every hidden speaker has one id:
/// the stable-id property. Returns hidden speaker to id.
fn assert_stable_ids(passages: &[Found], timeline: &[(u64, u64, usize)]) -> HashMap<usize, String> {
    let mut ids: HashMap<usize, HashSet<String>> = HashMap::new();
    for p in passages {
        let Some(id) = &p.speaker else { continue };
        let hidden = who(timeline, p.start, p.end).unwrap_or_else(|| panic!("nobody in {p:?}"));
        ids.entry(hidden).or_default().insert(id.clone());
    }
    let mut out = HashMap::new();
    for (hidden, set) in &ids {
        assert_eq!(set.len(), 1, "voice {hidden} got {set:?}: {passages:#?}");
        out.insert(*hidden, set.iter().next().unwrap().clone());
    }
    let distinct: HashSet<&String> = out.values().collect();
    assert_eq!(distinct.len(), out.len(), "two voices share an id: {out:?}");
    out
}

/// No passage moved from one speaker id to another between revisions.
fn assert_no_id_moves(events: &[Stored]) {
    let mut seen: HashMap<String, String> = HashMap::new();
    for p in all_passages(events) {
        if let Some(id) = p.speaker {
            if let Some(before) = seen.insert(p.id.clone(), id.clone()) {
                assert_eq!(before, id, "passage {} moved speakers", p.id);
            }
        }
    }
}

fn speakers(events: &[Stored]) -> Vec<(String, String)> {
    of(events, "speaker")
        .iter()
        .map(|s| {
            (
                s["id"].as_str().unwrap().to_owned(),
                s["label"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// Each `call-N` comes before the first passage that names it.
fn assert_speakers_come_first(events: &[Stored]) {
    let mut sent = HashSet::new();
    for e in events {
        match e.event.as_str() {
            "speaker" => {
                sent.insert(e.data["id"].as_str().unwrap().to_owned());
            }
            "passage" => {
                if let Some(id) = e.data["speaker"].as_str() {
                    assert!(sent.contains(id), "passage before its speaker {id}");
                }
            }
            _ => {}
        }
    }
}

fn wait_done(handle: &PipelineHandle) {
    wait_until("done", WAIT, || {
        let state = handle.state();
        assert!(
            !matches!(state, PipelineState::Failed { .. } | PipelineState::Stopped),
            "{state:?}"
        );
        (state == PipelineState::Done).then_some(())
    });
}

/// A value in the meta table of `dir`'s `transcript.db`, `None` while the pipeline hasn't created
/// the store. Tests poll it while the pipeline writes, so it reads without taking the write lock,
/// which [`Transcript::open`] takes. SQLite can answer busy without waiting for the busy timeout,
/// while another connection cleans up or recovers the WAL: that read is tried again until [`WAIT`]
/// is up. Any other error fails the test.
fn meta(dir: &MeetingDir, key: &str) -> Option<String> {
    let path = dir.path().join("transcript.db");
    let deadline = Instant::now() + WAIT;
    loop {
        match read_meta(&path, key) {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            read => return read.unwrap_or_else(|e| panic!("{}: {e}", path.display())),
        }
    }
}

fn read_meta(path: &Path, key: &str) -> rusqlite::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let created: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta')",
        [],
        |row| row.get(0),
    )?;
    if !created {
        return Ok(None);
    }
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
}

/// 1: three voices on the call get three ids, each sent right before its first passage.
#[test]
fn three_voices_get_three_ids() {
    let tmp = TempDir::new("speakers-three");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(9, 1.5);
    let rec = finished_call(&lib, &pieces);
    let d = ScriptedDiarizer::new();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    wait_done(&handle);
    let status = handle.labels();
    assert_eq!(
        (status.mode.as_str(), status.state),
        ("nemotron", LabelState::Finished)
    );
    assert_eq!(status.speakers, 3);
    handle.shutdown();

    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    assert_speakers_come_first(&all);
    // Numbered in the order the voices were first heard.
    assert_eq!(
        speakers(&all),
        vec![
            ("mic".to_owned(), "You".to_owned()),
            ("call-1".to_owned(), "Speaker 1".to_owned()),
            ("call-2".to_owned(), "Speaker 2".to_owned()),
            ("call-3".to_owned(), "Speaker 3".to_owned()),
        ]
    );
    let timeline = timeline_of(&pieces, 0);
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 9, "one passage per burst: {call:#?}");
    assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
    let ids = assert_stable_ids(&call, &timeline);
    assert_eq!(ids[&0], "call-1");
    assert_eq!(ids[&1], "call-2");
    assert_eq!(ids[&2], "call-3");
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("nemotron"));
    // The scorer agrees.
    let spans: Vec<Span> = call
        .iter()
        .map(|p| Span {
            start: p.start,
            end: p.end,
            speaker: p.speaker.clone(),
        })
        .collect();
    let reference: Vec<Span> = timeline
        .iter()
        .map(|&(start, end, k)| Span {
            start,
            end,
            speaker: Some(format!("S{k}")),
        })
        .collect();
    assert!((accuracy(&spans, &reference) - 1.0).abs() < 1e-9);
}

/// 2: while it records, no call passage ends after the follower's settled position.
#[test]
fn the_gate_holds_call_windows_at_the_settled_position() {
    let tmp = TempDir::new("speakers-gate");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let pieces = three_voices(8, 1.5);
    let call = voices(&pieces);
    // Model chunks commit 2 s late, so the settled position trails the audio by 2.5 s.
    let d = ScriptedDiarizer::with(|s| s.lag_frames = 200);
    let mut writer = rec.open_segment("call", 1, 0);
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    let check = || {
        let passages = last_revisions(&events(&log, &rec.id()), "call");
        let settled = handle.labels().settled_ms.unwrap_or(0);
        for p in &passages {
            assert!(p.end <= settled, "{p:?} ends after the settled {settled}");
        }
        passages.len()
    };
    for chunk in call.chunks(4_000) {
        writer.append(chunk).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        check();
    }
    wait_until("passages while it records", WAIT, || {
        (check() >= 3).then_some(())
    });
    let settled = handle.labels().settled_ms.unwrap();
    assert!(
        settled + 2_000 < ms(seconds(&pieces)),
        "the labels trail the audio: settled {settled}"
    );
    rec.close_segment(writer, "call", 1, 0);
    rec.stop(ms(seconds(&pieces)));
    wait_done(&handle);
    handle.shutdown();
    let all = events(&log, &rec.id());
    assert_stable_ids(&last_revisions(&all, "call"), &timeline_of(&pieces, 0));
    assert_eq!(last_revisions(&all, "call").len(), 8);
}

/// A frame of the screen journal at `t`, as the screen process writes one.
fn frame_line(t: u64) -> Value {
    json!({"kind":"frame","t":t,"id":format!("f-{t:09}"),"file":format!("screen/frames/f-{t:09}.jpg"),"width":1920,"height":1080,"text":format!("Slide {}", t / 2_000),"ocr":"ok","language":null})
}

/// 2b: frames and labels in one meeting (contract 3.2). Frames and screen marks go out while the
/// gate holds the newest call passages, and `done` waits for both: the gate letting go of what it
/// held at the stop, and the screen journal's `stop`.
#[test]
fn frames_and_speakers_together() {
    let tmp = TempDir::new("speakers-frames");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::with_screen(&lib, &[Track::Call], ScreenMode::Frames);
    rec.session(0, 1);
    rec.journal(json!({"kind":"session","t":40,"wall":1_793_000_000_040u64,"session":1,"pid":7,"resumed":false,"version":1,"mode":"frames","anchor":"boot","screen":"0.1.0"}));
    let pieces = three_voices(8, 1.5);
    let length = ms(seconds(&pieces));
    let call = voices(&pieces);
    // Model chunks commit 2 s late, so the gate holds the call windows of the last 2 s or more.
    let d = ScriptedDiarizer::with(|s| s.lag_frames = 200);
    let mut writer = rec.open_segment("call", 1, 0);
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    let gate = || {
        let passages = last_revisions(&events(&log, &rec.id()), "call");
        let settled = handle.labels().settled_ms.unwrap_or(0);
        for p in &passages {
            assert!(p.end <= settled, "{p:?} ends after the settled {settled}");
        }
        (passages.len(), settled)
    };

    // A new slide every 2 s while the call goes on, and the screen locked from 5 to 6 s.
    let mut frames = Vec::new();
    for (i, chunk) in call.chunks(4_000).enumerate() {
        writer.append(chunk).unwrap();
        let t = (i as u64 + 1) * 250;
        if t % 2_000 == 0 && t < length {
            rec.journal(frame_line(t));
            frames.push(t);
        }
        match t {
            5_000 => rec.journal(json!({"kind":"lock","t":t})),
            6_000 => rec.journal(json!({"kind":"unlock","t":t})),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(5));
        gate();
    }
    let newest = *frames.last().unwrap();
    let settled = wait_until(
        "every frame and the lock's gap while the gate holds the call",
        WAIT,
        || {
            let all = events(&log, &rec.id());
            let sent: Vec<u64> = of(&all, "frame")
                .iter()
                .map(|f| f["at"].as_u64().unwrap())
                .collect();
            let gap = of(&all, "mark")
                .into_iter()
                .any(|m| m["id"] == "gap-screen-5000");
            let (passages, settled) = gate();
            (sent == frames && gap && passages >= 3).then_some(settled)
        },
    );
    assert!(
        settled + 2_000 <= length && settled < newest,
        "the frame at {newest} went out while the gate held the call at {settled}"
    );
    let all = events(&log, &rec.id());
    let gap = of(&all, "mark")
        .into_iter()
        .find(|m| m["id"] == "gap-screen-5000")
        .unwrap();
    assert_eq!(
        (&gap["kind"], &gap["track"], &gap["detail"], &gap["until"]),
        (
            &json!("screen_gap"),
            &json!("screen"),
            &json!("locked"),
            &json!(6_000)
        )
    );
    assert!(!handle.state().is_final(), "{:?}", handle.state());

    // The recording stops. The labels finish and the gate lets go of every call passage, but the
    // meeting isn't done until the screen journal has its stop.
    rec.close_segment(writer, "call", 1, 0);
    rec.stop(length);
    wait_until(
        "the labels to finish and every call passage to have its speaker",
        WAIT,
        || {
            let call = last_revisions(&events(&log, &rec.id()), "call");
            (handle.labels().state == LabelState::Finished
                && call.len() == 8
                && call.iter().all(|p| p.speaker.is_some()))
            .then_some(())
        },
    );
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        !handle.state().is_final(),
        "done waits for the screen journal's stop: {:?}",
        handle.state()
    );
    rec.journal(json!({"kind":"stop","t":length,"reason":"meeting"}));
    wait_done(&handle);
    assert_eq!(handle.labels().speakers, 3);
    handle.shutdown();

    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    assert_speakers_come_first(&all);
    assert_no_id_moves(&all);
    let call = last_revisions(&all, "call");
    assert_eq!(assert_stable_ids(&call, &timeline_of(&pieces, 0)).len(), 3);
    let held: Vec<&Found> = call.iter().filter(|p| p.end > settled).collect();
    assert!(!held.is_empty(), "the gate held call passages at the stop");
    let done = all
        .iter()
        .find(|e| e.event == "meeting.state" && e.data["state"] == "done")
        .unwrap()
        .seq;
    for e in &all {
        if matches!(e.event.as_str(), "frame" | "mark" | "passage" | "speaker") {
            assert!(e.seq < done, "{} {} after done", e.event, e.data);
        }
    }
    let sent: Vec<u64> = of(&all, "frame")
        .iter()
        .map(|f| f["at"].as_u64().unwrap())
        .collect();
    assert_eq!(sent, frames, "every frame once, in order");
}

/// Call passages of the first revision that end past the settled position: what the gate
/// should have held. Each with the settled position it was seen at.
///
/// The events come first and the settled position after: it only moves forward, so the value
/// read belongs to a moment at or after the commits, and a commit the gate made legally is
/// never reported because the position advanced while the log was read.
fn past_settled(log: &EventLog, id: &str, handle: &PipelineHandle, seen: &mut Vec<(Found, u64)>) {
    let passages = all_passages(&events(log, id));
    let settled = handle.labels().settled_ms.unwrap_or(0);
    for p in passages {
        if p.track == "call"
            && p.revision == 1
            && p.end > settled
            && !seen.iter().any(|(q, _)| q.id == p.id)
        {
            seen.push((p, settled));
        }
    }
}

/// 2a: a pause longer than the stall doesn't release the gate when the recording resumes: the
/// follower padded the gap and waits for audio, which isn't a stall. Neither is the wait for the
/// recorder's next samples once every chunk is in: the gate holds until the stop, however long
/// that wait takes on a slow machine.
#[test]
fn a_long_pause_keeps_the_gate_after_the_resume() {
    let tmp = TempDir::new("speakers-long-pause");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let first = [
        (0.5, None),
        (1.5, Some(0)),
        (1.2, None),
        (1.5, Some(1)),
        (1.3, None),
    ];
    rec.segment("call", 1, 0, &voices(&first));
    rec.line(json!({"kind": "pause", "t": 6000}));
    let d = ScriptedDiarizer::with(|s| s.lag_frames = 200);
    let stall = Duration::from_millis(1_000);
    let options = PipelineOptions {
        speakers: FollowerTiming {
            stall,
            ..follower_timing()
        },
        ..labelled(&d)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_until("settled at the pause", WAIT, || {
        (handle.labels().settled_ms == Some(6_000)).then_some(())
    });
    wait_until("both bursts", WAIT, || {
        (last_revisions(&events(&log, &rec.id()), "call").len() >= 2).then_some(())
    });
    // The meeting stays paused longer than the stall.
    std::thread::sleep(stall + Duration::from_millis(500));
    rec.line(json!({"kind": "resume", "t": 12000}));
    let second = voices(&[(1.5, Some(2)), (1.0, None), (1.5, Some(0)), (1.0, None)]);
    let mut writer = rec.open_segment("call", 2, 12_000);
    let mut seen = Vec::new();
    for chunk in second.chunks(4_000) {
        writer.append(chunk).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        past_settled(&log, &rec.id(), &handle, &mut seen);
    }
    // Every chunk is in, but the meeting isn't stopped yet: the follower waits for the recorder
    // and that wait mustn't release the gate, also when a slow machine makes it longer than the
    // stall.
    let until = Instant::now() + stall + Duration::from_millis(500);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
        past_settled(&log, &rec.id(), &handle, &mut seen);
    }
    rec.close_segment(writer, "call", 2, 12_000);
    rec.stop(17_000);
    wait_done(&handle);
    handle.shutdown();
    assert!(seen.is_empty(), "committed past settled: {seen:#?}");
    let call = last_revisions(&events(&log, &rec.id()), "call");
    assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
}

/// 3c: a worker that is slow while it hears the reel after a crash doesn't release the gate:
/// the settled position can't move then, but every push that returns is progress.
#[test]
fn a_slow_reel_after_a_crash_keeps_the_gate() {
    let tmp = TempDir::new("speakers-slow-reel");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(12, 1.5);
    let rec = finished_call(&lib, &pieces);
    // The worker crashes 22 pushes in; the new one takes 120 ms a push, so its reel of about
    // 25 pushes takes 1.5 times the stall, and without the reel's progress the test fails. Other
    // pushes move the settled position every 120 ms, so only a pause of the disk or the scheduler
    // could trip the stall elsewhere: with a 1 s stall, 1 suite run in 51 failed that way.
    let d = ScriptedDiarizer::with(|s| {
        s.crash_on = vec![22];
        s.slow_after = 22;
        s.slow = Duration::from_millis(120);
    });
    let options = PipelineOptions {
        speakers: FollowerTiming {
            stall: Duration::from_millis(2_000),
            ..follower_timing()
        },
        ..labelled(&d)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_done(&handle);
    handle.shutdown();
    assert_eq!(d.script().opens, 2, "one restart");
    let all = events(&log, &rec.id());
    let unlabelled: Vec<Found> = all_passages(&all)
        .into_iter()
        .filter(|p| p.track == "call" && p.speaker.is_none())
        .collect();
    assert!(
        unlabelled.is_empty(),
        "released by a stall: {unlabelled:#?}"
    );
    assert_eq!(
        assert_stable_ids(&last_revisions(&all, "call"), &timeline_of(&pieces, 0)).len(),
        3
    );
}

/// 2c: Speaker labels switched off mid-meeting apply to the next meeting: a labelled meeting
/// whose pipeline starts again (a dictionary change, a Voice restart) keeps labelling, and a
/// new meeting gets none.
#[test]
fn the_switch_going_off_keeps_a_labelled_meeting_labelled() {
    let tmp = TempDir::new("speakers-switch-off");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let first = [
        (0.5, None),
        (1.5, Some(0)),
        (1.2, None),
        (1.5, Some(1)),
        (1.3, None),
    ];
    rec.segment("call", 1, 0, &voices(&first));
    rec.line(json!({"kind": "pause", "t": 6000}));
    let d = ScriptedDiarizer::new();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    wait_until("two labelled call passages", WAIT, || {
        let call = last_revisions(&events(&log, &rec.id()), "call");
        (call.len() >= 2 && call.iter().all(|p| p.speaker.is_some())).then_some(())
    });
    handle.shutdown();
    // The switch is off now; the engine still hands over the diarizer.
    let off = || PipelineOptions {
        live_speakers: false,
        ..labelled(&d)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), off());
    rec.line(json!({"kind": "resume", "t": 12000}));
    let second = [(1.5, Some(2)), (1.2, None), (1.5, Some(0)), (1.3, None)];
    rec.segment("call", 2, 12_000, &voices(&second));
    rec.stop(17_500);
    wait_done(&handle);
    let status = handle.labels();
    handle.shutdown();
    assert_eq!(
        (status.mode.as_str(), status.state),
        ("nemotron", LabelState::Finished),
        "{status:?}"
    );
    let call = last_revisions(&events(&log, &rec.id()), "call");
    let later: Vec<&Found> = call.iter().filter(|p| p.start >= 12_000).collect();
    assert_eq!(later.len(), 2, "{call:#?}");
    assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
    assert_eq!(later[0].speaker.as_deref(), Some("call-3"), "{call:#?}");
    assert_eq!(later[1].speaker.as_deref(), Some("call-1"), "{call:#?}");

    // A meeting that starts now gets no labels, though the diarizer could run.
    let opens = d.script().opens;
    let pieces = three_voices(3, 1.5);
    let next = finished_call(&lib, &pieces);
    let handle = Pipeline::start(next.dir.clone(), log.clone(), off());
    wait_done(&handle);
    let status = handle.labels();
    handle.shutdown();
    assert_eq!(status.mode, "off");
    assert_eq!(status.reason.as_deref(), Some("off"));
    assert_eq!(meta(&next.dir, MODE_KEY).as_deref(), Some("off"));
    assert_eq!(d.script().opens, opens, "no worker for the new meeting");
    let call = last_revisions(&events(&log, &next.id()), "call");
    assert!(call.iter().all(|p| p.speaker.as_deref() == Some("call-1")));
}

/// 4c: a stopped meeting whose labels are done but whose transcription waits, as for consent,
/// opens no new worker each time Voice starts, and its passages still get their speakers once
/// transcription may go on.
#[test]
fn finished_labels_open_no_worker_on_the_next_start() {
    let tmp = TempDir::new("speakers-labelled");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(6, 1.5);
    let rec = finished_call(&lib, &pieces);
    let d = ScriptedDiarizer::new();
    let agreed = Arc::new(AtomicBool::new(false));
    let options = || PipelineOptions {
        transcription_allowed: {
            let agreed = agreed.clone();
            Arc::new(move || {
                if agreed.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err("Agree to send meeting audio in Settings.".into())
                }
            })
        },
        ..labelled(&d)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options());
    wait_until("labels done", WAIT, || {
        (handle.labels().state == LabelState::Finished).then_some(())
    });
    assert!(matches!(handle.state(), PipelineState::Waiting { .. }));
    handle.shutdown();
    assert_eq!(meta(&rec.dir, LABELLED_KEY).as_deref(), Some("1"));
    let (opens, pushed) = {
        let script = d.script();
        (script.opens, script.pushed_samples)
    };
    // Voice starts twice more before the user agrees.
    for _ in 0..2 {
        let handle = Pipeline::start(rec.dir.clone(), log.clone(), options());
        wait_until("waiting for consent", WAIT, || {
            matches!(handle.state(), PipelineState::Waiting { .. }).then_some(())
        });
        let status = handle.labels();
        assert_eq!(
            (status.mode.as_str(), status.state),
            ("nemotron", LabelState::Finished)
        );
        std::thread::sleep(Duration::from_millis(100));
        handle.shutdown();
    }
    assert_eq!(d.script().opens, opens, "no new worker");
    assert_eq!(d.script().pushed_samples, pushed, "nothing heard again");
    agreed.store(true, Ordering::SeqCst);
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options());
    wait_done(&handle);
    handle.shutdown();
    let call = last_revisions(&events(&log, &rec.id()), "call");
    assert_eq!(call.len(), 6, "{call:#?}");
    assert_eq!(assert_stable_ids(&call, &timeline_of(&pieces, 0)).len(), 3);
    assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
}

/// 2b: a voice still talking at Stop keeps its last turn to the end of the call, and its last
/// passage keeps its speaker, though the runtime's last frame reaches past the audio pushed.
#[test]
fn a_voice_talking_at_stop_keeps_its_speaker() {
    // A call of whole 10 ms frames, and one that ends in a part frame.
    for (name, last) in [("whole", 2.0), ("part", 2.003)] {
        let tmp = TempDir::new(&format!("speakers-stop-{name}"));
        let lib = Library::open(tmp.path()).unwrap();
        let log = EventLog::open(tmp.path()).unwrap();
        let pieces = [(0.5, None), (1.5, Some(0)), (1.2, None), (last, Some(1))];
        let rec = finished_call(&lib, &pieces);
        let samples = voices(&pieces).len() as i64;
        let d = ScriptedDiarizer::with(|s| s.finish_extra = 1);
        let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
        wait_done(&handle);
        handle.shutdown();
        let turns = stored_turns(&rec.dir);
        let last_turn = turns
            .iter()
            .find(|t| t.0 == "call-2")
            .unwrap_or_else(|| panic!("{name}: {turns:?}"));
        assert_eq!(
            last_turn.2,
            Some((samples + 15) / 16),
            "{name}: the last turn ends with the call: {turns:?}"
        );
        let call = last_revisions(&events(&log, &rec.id()), "call");
        assert_eq!(call.len(), 2, "{name}: {call:#?}");
        assert_eq!(
            call[1].speaker.as_deref(),
            Some("call-2"),
            "{name}: {call:#?}"
        );
    }
}

/// 3: a worker crash two thirds into the meeting keeps every voice's id.
#[test]
fn a_worker_crash_keeps_the_ids() {
    let tmp = TempDir::new("speakers-crash");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(12, 1.5);
    let rec = finished_call(&lib, &pieces);
    // About 22 one-second pushes in: the second third.
    let d = ScriptedDiarizer::with(|s| s.crash_on = vec![22]);
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    wait_done(&handle);
    handle.shutdown();
    assert_eq!(d.script().opens, 2, "a new stream after the crash");
    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    assert_no_id_moves(&all);
    let call = last_revisions(&all, "call");
    let ids = assert_stable_ids(&call, &timeline_of(&pieces, 0));
    assert_eq!(ids.len(), 3);
    let named = call.iter().filter(|p| p.speaker.is_some()).count();
    assert!(named >= call.len() - 1, "{call:#?}");
    let epochs = Transcript::open(rec.dir.path()).unwrap().epochs().unwrap();
    assert_eq!(
        epochs.iter().map(|e| e.reason.as_str()).collect::<Vec<_>>(),
        ["start", "crash"]
    );
    assert!(epochs[1].feed_from > 0);
}

/// 4: a pipeline started again on the same folder keeps every voice's id.
#[test]
fn an_app_restart_keeps_the_ids() {
    let tmp = TempDir::new("speakers-restart");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Mic, Track::Call]);
    rec.session(0, 1);
    let pieces = three_voices(12, 1.5);
    let call = voices(&pieces);
    let half = call.len() / 2;
    let mut writer = rec.open_segment("call", 1, 0);
    writer.append(&call[..half]).unwrap();
    let d = ScriptedDiarizer::new();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    wait_until("three ids before the restart", WAIT, || {
        let ids: HashSet<String> = last_revisions(&events(&log, &rec.id()), "call")
            .into_iter()
            .filter_map(|p| p.speaker)
            .collect();
        (ids.len() == 3).then_some(())
    });
    handle.shutdown();

    // The same diarizer: its next stream numbers the voices differently.
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    writer.append(&call[half..]).unwrap();
    rec.close_segment(writer, "call", 1, 0);
    rec.segment("mic", 1, 0, &voices(&[(seconds(&pieces), None)]));
    rec.stop(ms(seconds(&pieces)));
    wait_done(&handle);
    handle.shutdown();

    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    assert_no_id_moves(&all);
    let ids = assert_stable_ids(&last_revisions(&all, "call"), &timeline_of(&pieces, 0));
    assert_eq!(ids.len(), 3);
    let epochs = Transcript::open(rec.dir.path()).unwrap().epochs().unwrap();
    assert_eq!(epochs.len(), 2);
    assert_eq!(epochs[1].reason, "restart");
    assert_eq!(d.script().opens, 2);
}

/// Every stored turn, open ones with no end, in a fixed order.
fn stored_turns(dir: &MeetingDir) -> Vec<(String, i64, Option<i64>, i64)> {
    let conn = rusqlite::Connection::open(dir.path().join("transcript.db")).unwrap();
    let mut statement = conn
        .prepare(
            r#"SELECT speaker, start, "end", epoch FROM speaker_turns ORDER BY start, speaker"#,
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap();
    rows.map(Result::unwrap).collect()
}

/// 4b: a follower stopped while a worker call hangs, longer than the 2 s it is waited for,
/// writes nothing when the call returns, so it can't cut the turns of the follower after it.
#[test]
fn a_follower_stopped_during_a_hung_call_writes_nothing_afterwards() {
    let tmp = TempDir::new("speakers-retired");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    // One voice talks for 20 s, so a turn is open when the call hangs.
    let pieces = [(0.5, None), (20.0, Some(0))];
    rec.segment("call", 1, 0, &voices(&pieces));
    let d = ScriptedDiarizer::with(|s| {
        s.hang_on = vec![6];
        s.hang = Duration::from_secs(3);
    });
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    wait_until("the hung call", WAIT, || {
        (d.script().pushes >= 6).then_some(())
    });
    let open = stored_turns(&rec.dir);
    assert!(
        open.iter().any(|(_, _, end, _)| end.is_none()),
        "a turn is open: {open:?}"
    );
    handle.shutdown();
    let before = stored_turns(&rec.dir);
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(
        stored_turns(&rec.dir),
        before,
        "the stopped follower changed the turns after its call returned"
    );
}

/// Word timings as [`Bursts`] gives them; once `block` is set, the next call first waits `hold`,
/// as a slow cloud provider or a loading Orukeet can.
struct SlowBursts {
    block: AtomicBool,
    blocking: AtomicUsize,
    hold: Duration,
}

impl TimedSpeechToText for SlowBursts {
    fn transcribe_timed(&self, pcm: &[i16], hints: &Hints) -> Result<TimedText, ProviderError> {
        if self.block.swap(false, Ordering::SeqCst) {
            self.blocking.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.hold);
        }
        Bursts.transcribe_timed(pcm, hints)
    }
}

/// 4c: a pipeline replaced while a recognition call outlasts the 5 s shutdown wait, as a
/// Settings change during a meeting does, stops its follower when the shutdown returns. Before,
/// the old follower went on next to the new one and left a turn open that covered the rest of
/// the meeting, so every later call passage lost its speaker.
#[test]
fn a_pipeline_replaced_during_a_long_recognition_stops_its_follower() {
    let tmp = TempDir::new("speakers-replaced");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let pieces = three_voices(16, 1.5);
    let call = voices(&pieces);
    let at = |s: f64| (s * RATE as f64) as usize;
    let mut writer = rec.open_segment("call", 1, 0);
    writer.append(&call[..at(16.7)]).unwrap();
    let d = ScriptedDiarizer::new();
    let stt = Arc::new(SlowBursts {
        block: AtomicBool::new(false),
        blocking: AtomicUsize::new(0),
        // Longer than the 5 s the shutdown waits.
        hold: Duration::from_secs(8),
    });
    let options = || PipelineOptions {
        timed: Some(stt.clone()),
        ..labelled(&d)
    };
    let first = Pipeline::start(rec.dir.clone(), log.clone(), options());
    wait_until("named passages", WAIT, || {
        let named = last_revisions(&events(&log, &rec.id()), "call")
            .iter()
            .filter(|p| p.speaker.is_some())
            .count();
        (named >= 2).then_some(())
    });
    stt.block.store(true, Ordering::SeqCst);
    writer.append(&call[at(16.7)..at(24.8)]).unwrap();
    wait_until("the slow recognition", WAIT, || {
        (stt.blocking.load(Ordering::SeqCst) >= 1).then_some(())
    });
    first.shutdown();

    // The old pipeline's thread still waits for its recognizer; its follower hears nothing more.
    let pushes = d.script().pushes;
    writer.append(&call[at(24.8)..at(28.0)]).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        d.script().pushes,
        pushes,
        "the replaced pipeline's follower went on"
    );

    let second = Pipeline::start(rec.dir.clone(), log.clone(), options());
    writer.append(&call[at(28.0)..]).unwrap();
    rec.close_segment(writer, "call", 1, 0);
    rec.stop(ms(seconds(&pieces)));
    wait_done(&second);
    second.shutdown();

    let open: Vec<_> = stored_turns(&rec.dir)
        .into_iter()
        .filter(|(_, _, end, _)| end.is_none())
        .collect();
    assert!(open.is_empty(), "turns left open: {open:?}");
    let all = events(&log, &rec.id());
    assert_no_id_moves(&all);
    let call = last_revisions(&all, "call");
    assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
    assert_eq!(assert_stable_ids(&call, &timeline_of(&pieces, 0)).len(), 3);
    let epochs = Transcript::open(rec.dir.path()).unwrap().epochs().unwrap();
    assert_eq!(
        epochs.iter().map(|e| e.reason.as_str()).collect::<Vec<_>>(),
        ["start", "restart"]
    );
}

/// 5: three crashes in a row stop labels for the meeting; transcription goes on without them.
#[test]
fn three_crashes_stop_labels_and_never_transcription() {
    let tmp = TempDir::new("speakers-failed");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let pieces = three_voices(10, 1.5);
    let call = voices(&pieces);
    let first = 4 * RATE * 27 / 10;
    let mut writer = rec.open_segment("call", 1, 0);
    writer.append(&call[..first]).unwrap();
    let d = ScriptedDiarizer::new();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    wait_until("named passages", WAIT, || {
        let named = last_revisions(&events(&log, &rec.id()), "call")
            .iter()
            .filter(|p| p.speaker.is_some())
            .count();
        (named >= 2).then_some(())
    });
    d.script().crash_all = true;
    writer.append(&call[first..]).unwrap();
    wait_until("labels to fail", WAIT, || {
        (handle.labels().state == LabelState::Failed).then_some(())
    });
    let failed_at = handle.labels().settled_ms.unwrap_or(0);
    rec.close_segment(writer, "call", 1, 0);
    rec.stop(ms(seconds(&pieces)));
    wait_done(&handle);
    let status = handle.labels();
    assert_eq!(
        (status.mode.as_str(), status.state, status.reason.as_deref()),
        ("nemotron", LabelState::Failed, Some("runtimeFailed"))
    );
    let all = events(&log, &rec.id());
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 10, "every burst transcribed: {call:#?}");
    let later: Vec<&Found> = call
        .iter()
        .filter(|p| p.start > failed_at + 3_000)
        .collect();
    assert!(!later.is_empty());
    assert!(later.iter().all(|p| p.speaker.is_none()), "{later:#?}");
    assert_stable_ids(&call, &timeline_of(&pieces, 0));
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("nemotron"));
}

/// 6: a runtime that refuses before any call passage makes the meeting one without labels.
#[test]
fn a_refused_runtime_before_any_passage_falls_back_to_call_1() {
    let tmp = TempDir::new("speakers-refused");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(4, 1.5);
    let rec = finished_call(&lib, &pieces);
    let d = ScriptedDiarizer::with(|s| s.refuse = Some(DiarErrorKind::Runtime));
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    wait_done(&handle);
    let status = handle.labels();
    assert_eq!(
        (status.mode.as_str(), status.state, status.reason.as_deref()),
        ("off", LabelState::Failed, Some("runtimeFailed"))
    );
    let all = events(&log, &rec.id());
    assert_eq!(
        speakers(&all),
        vec![
            ("mic".to_owned(), "You".to_owned()),
            ("call-1".to_owned(), "Speaker 1".to_owned())
        ]
    );
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 4);
    assert!(call.iter().all(|p| p.speaker.as_deref() == Some("call-1")));
    assert!(all_passages(&all).iter().all(|p| p.revision == 1));
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("off"));
    assert_eq!(meta(&rec.dir, REASON_KEY).as_deref(), Some("runtimeFailed"));
}

/// 6b: labels that stalled, so call passages went out without a speaker, and then failed before
/// any call speaker reached Rukoo: the meeting goes on without labels, and those passages get
/// `call-1` as well.
#[test]
fn labels_that_stall_then_fail_before_any_id_leave_the_whole_call_on_call_1() {
    let tmp = TempDir::new("speakers-stall-fail");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(6, 1.5);
    let rec = finished_call(&lib, &pieces);
    // The first push of each of three streams hangs; the pipeline stops waiting after 100 ms.
    let d = ScriptedDiarizer::with(|s| {
        s.hang_on = vec![1, 2, 3];
        s.hang = Duration::from_millis(600);
    });
    let options = PipelineOptions {
        speakers: FollowerTiming {
            stall: Duration::from_millis(100),
            ..follower_timing()
        },
        ..labelled(&d)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_done(&handle);
    let status = handle.labels();
    assert_eq!(
        (status.mode.as_str(), status.state, status.reason.as_deref()),
        ("off", LabelState::Failed, Some("runtimeFailed"))
    );
    handle.shutdown();
    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    assert_no_id_moves(&all);
    assert_eq!(
        speakers(&all),
        vec![
            ("mic".to_owned(), "You".to_owned()),
            ("call-1".to_owned(), "Speaker 1".to_owned())
        ]
    );
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 6, "{call:#?}");
    assert!(
        call.iter().all(|p| p.speaker.as_deref() == Some("call-1")),
        "{call:#?}"
    );
    assert!(
        all_passages(&all)
            .iter()
            .any(|p| p.track == "call" && p.speaker.is_none()),
        "some went out without a speaker during the stall"
    );
}

/// 7: without labels the events are exactly Voice 0.1's.
#[test]
fn without_labels_the_events_are_voice_0_1s() {
    let tmp = TempDir::new("speakers-off");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Mic, Track::Call]);
    rec.session(0, 1);
    rec.segment("mic", 1, 0, &speech(&[(2.0, true), (1.0, false)]));
    rec.segment(
        "call",
        1,
        0,
        &speech(&[(1.0, false), (1.0, true), (1.0, false)]),
    );
    rec.stop(3000);
    let options = PipelineOptions {
        timing: fast(),
        labels: Labels::Off {
            reason: "cpu".into(),
        },
        ..PipelineOptions::new(Arc::new(Echo::default()))
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_done(&handle);
    let status = handle.labels();
    assert_eq!(
        (status.mode.as_str(), status.state, status.reason.as_deref()),
        ("off", LabelState::Off, Some("cpu"))
    );
    let all = events(&log, &rec.id());
    let sequence: Vec<(String, Value)> = all
        .iter()
        .skip(1)
        .map(|e| (e.event.clone(), e.data.clone()))
        .collect();
    assert_eq!(all[0].event, "meeting.started");
    assert_eq!(
        sequence,
        vec![
            (
                "speaker".into(),
                json!({"id":"mic","track":"mic","label":"You"})
            ),
            (
                "speaker".into(),
                json!({"id":"call-1","track":"call","label":"Speaker 1"})
            ),
            ("meeting.state".into(), json!({"state":"recording","at":0})),
            (
                "meeting.state".into(),
                json!({"state":"processing","at":3000,"endedAt":1_793_000_003_000u64,"reason":"user"})
            ),
            (
                "passage".into(),
                json!({"id":"mic-000000000","track":"mic","speaker":"mic","start":0,"end":2400,"text":"heard 2400 ms","revision":1,"dictation":false,"language":"en"})
            ),
            (
                "passage".into(),
                json!({"id":"call-000000000","track":"call","speaker":"call-1","start":0,"end":2400,"text":"heard 2400 ms","revision":1,"dictation":false,"language":"en"})
            ),
            (
                "meeting.state".into(),
                json!({"state":"done","at":3000,"endedAt":1_793_000_003_000u64})
            ),
        ]
    );
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("off"));
}

/// 8: no audio goes to the diarizer while the user dictates; it catches up afterwards.
#[test]
fn labels_wait_while_the_user_dictates() {
    let tmp = TempDir::new("speakers-dictation");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let pieces = three_voices(3, 1.5);
    let mut writer = rec.open_segment("call", 1, 0);
    writer.append(&voices(&pieces)).unwrap();
    rec.line(json!({"kind":"dictation","t":ms(seconds(&pieces)) - 100,"state":"start"}));
    let d = ScriptedDiarizer::new();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(d.script().pushes, 0, "dictation comes first");
    assert!(events(&log, &rec.id()).iter().all(|e| e.event != "passage"));
    rec.line(json!({"kind":"dictation","t":ms(seconds(&pieces)),"state":"end"}));
    wait_until("pushes after the dictation", WAIT, || {
        (d.script().pushes > 0).then_some(())
    });
    rec.close_segment(writer, "call", 1, 0);
    rec.stop(ms(seconds(&pieces)) + 100);
    wait_done(&handle);
    handle.shutdown();
    let all = events(&log, &rec.id());
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 3);
    assert_eq!(assert_stable_ids(&call, &timeline_of(&pieces, 0)).len(), 3);
}

/// 9: passages committed without a speaker while the follower stalled get it as a new revision;
/// a passage with an id never changes it.
#[test]
fn passages_from_a_stall_get_their_speaker_later() {
    let tmp = TempDir::new("speakers-backfill");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(8, 1.5);
    let rec = finished_call(&lib, &pieces);
    // The sixth push hangs for 1.5 s; the pipeline stops waiting after 150 ms.
    let d = ScriptedDiarizer::with(|s| {
        s.hang_on = vec![6];
        s.hang = Duration::from_millis(1_500);
    });
    let options = PipelineOptions {
        speakers: FollowerTiming {
            stall: Duration::from_millis(150),
            ..follower_timing()
        },
        ..labelled(&d)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_done(&handle);
    handle.shutdown();
    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    assert_no_id_moves(&all);
    let revised: Vec<Found> = all_passages(&all)
        .into_iter()
        .filter(|p| p.track == "call" && p.revision == 2)
        .collect();
    assert!(!revised.is_empty(), "{:#?}", all_passages(&all));
    for p in &revised {
        assert!(p.speaker.is_some());
        let first = all_passages(&all)
            .into_iter()
            .find(|q| q.id == p.id && q.revision == 1)
            .unwrap();
        assert_eq!(
            first.speaker, None,
            "only a passage without a speaker is revised"
        );
    }
    let call = last_revisions(&all, "call");
    assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
    assert_eq!(assert_stable_ids(&call, &timeline_of(&pieces, 0)).len(), 3);
    let t = Transcript::open(rec.dir.path()).unwrap();
    assert_eq!(t.epochs().unwrap().len(), 2);
}

/// 10: a pause and a recorder crash each get padding once, so labels reach the audio before
/// the gap and the call is transcribed up to it. The diarizer commits in the runtime's chunks:
/// audio that ends 6.0 s into the stream is where the worker settled 0.445 s short with less
/// padding, and the passage before the pause then waited for the 20 s stall.
#[test]
fn gaps_get_padding_once_and_settle_the_audio_before_them() {
    let tmp = TempDir::new("speakers-gaps");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let first = [
        (0.5, None),
        (1.5, Some(0)),
        (1.2, None),
        (1.5, Some(1)),
        (1.3, None),
    ];
    rec.segment("call", 1, 0, &voices(&first));
    rec.line(json!({"kind":"pause","t":6000}));
    let d = ScriptedDiarizer::with(Script::like_the_runtime);
    let pad = FollowerTiming::default().pad;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    // Well before the 20 s after which the pipeline stops waiting for labels.
    let soon = Duration::from_secs(10);
    wait_until("the audio before the pause to settle", soon, || {
        (handle.labels().settled_ms == Some(6_000)).then_some(())
    });
    wait_until("both bursts transcribed", soon, || {
        (last_revisions(&events(&log, &rec.id()), "call").len() == 2).then_some(())
    });
    assert_eq!(d.script().zero_samples, pad, "padding once at the pause");

    rec.line(json!({"kind":"resume","t":12000}));
    let second = [(1.5, Some(2)), (1.0, None), (1.5, Some(0)), (1.0, None)];
    rec.segment("call", 2, 12_000, &voices(&second));
    // The recorder dies at 17 s and is back at 20 s.
    rec.session(20_000, 2);
    rec.line(json!({"kind":"crash","t":20000,"lastSeen":17000}));
    let third = [(1.5, Some(1)), (1.0, None)];
    rec.segment("call", 3, 20_000, &voices(&third));
    rec.stop(22_500);
    wait_done(&handle);
    assert_eq!(
        d.script().zero_samples,
        2 * pad,
        "padding once per gap, none at the end"
    );
    let mut timeline = timeline_of(&first, 0);
    timeline.extend(timeline_of(&second, 12_000));
    timeline.extend(timeline_of(&third, 20_000));
    let all = events(&log, &rec.id());
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 5, "{call:#?}");
    assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
    assert_eq!(assert_stable_ids(&call, &timeline).len(), 3);
    assert_eq!(
        Transcript::open(rec.dir.path())
            .unwrap()
            .meta_u64(SETTLED_KEY)
            .unwrap(),
        Some(22_500)
    );
}

fn stream_options(fake: &FakeStreaming, diarize: bool) -> StreamOptions {
    StreamOptions {
        speech: Arc::new(fake.clone()),
        diarize,
        delay_ms: 0,
        language: None,
        context_bias: Vec::new(),
    }
}

/// 11: Mistral's stream: nothing before consent, passages from segments without times in the
/// right windows, a reconnect that sends references named after the known ids and maps them
/// back, and microphone deltas as microphone passages.
#[test]
fn mistral_streams_after_consent_and_keeps_ids_across_a_reconnect() {
    let tmp = TempDir::new("speakers-mistral");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(8, 3.5);
    let length = seconds(&pieces);
    let rec = Recording::new(&lib, &[Track::Mic, Track::Call]);
    rec.session(0, 1);
    let mic = [(1.0, None), (2.0, Some(3)), (length - 3.0, None)];
    rec.segment("mic", 1, 0, &voices(&mic));
    rec.segment("call", 1, 0, &voices(&pieces));
    rec.stop(ms(length));
    let fake = FakeStreaming::default();
    // The labelling session drops at its 200th push, 20 s into the call.
    fake.state().fail_on = vec![200];
    let agreed = Arc::new(AtomicBool::new(false));
    let options = PipelineOptions {
        transcription_allowed: {
            let agreed = agreed.clone();
            Arc::new(move || {
                if agreed.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err("Agree to stream meeting audio to Mistral in Settings.".into())
                }
            })
        },
        timing: fast(),
        labels: Labels::Off {
            reason: "stream".into(),
        },
        stream: Some(stream_options(&fake, true)),
        // Twenty times real time, so the pipeline keeps up as it would.
        speakers: FollowerTiming {
            stream_speed: 20.0,
            ..follower_timing()
        },
        ..PipelineOptions::new(Arc::new(Offline))
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_until("waiting for consent", WAIT, || {
        matches!(handle.state(), PipelineState::Waiting { .. }).then_some(())
    });
    // The followers say why on their first look, which a busy PC can hold back a while.
    wait_until("the consent reason", WAIT, || {
        (handle.labels().reason.as_deref() == Some("consent")).then_some(())
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        fake.state().sessions.is_empty(),
        "no connection before consent"
    );
    assert_eq!(fake.state().pushes, 0, "no audio before consent");

    agreed.store(true, Ordering::SeqCst);
    handle.wake();
    wait_done(&handle);
    assert_eq!(handle.labels().mode, "mistral");
    handle.shutdown();

    let state = fake.state();
    let labelling: Vec<_> = state.sessions.iter().filter(|s| s.diarize).collect();
    assert_eq!(labelling.len(), 2, "one reconnect");
    assert!(labelling[0].references.is_empty());
    let mut names: Vec<&str> = labelling[1]
        .references
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["call-1", "call-2", "call-3"]);
    assert_eq!(state.sessions.iter().filter(|s| !s.diarize).count(), 1);
    drop(state);

    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    assert_speakers_come_first(&all);
    let timeline = timeline_of(&pieces, 0);
    let call = last_revisions(&all, "call");
    assert!(call.len() >= 8, "{call:#?}");
    let ids = assert_stable_ids(&call, &timeline);
    assert_eq!(ids.len(), 3);
    // Every burst has its passage.
    for &(start, end, _) in &timeline {
        assert!(
            call.iter().any(|p| p.start < end && p.end > start),
            "nothing for {start}..{end}: {call:#?}"
        );
    }
    let mic = last_revisions(&all, "mic");
    assert_eq!(mic.len(), 1, "{mic:#?}");
    assert_eq!(mic[0].speaker.as_deref(), Some("mic"));
    assert!(mic[0].text.starts_with("w1000"), "{mic:?}");
}

fn mistral_options(fake: &FakeStreaming, diarize: bool) -> PipelineOptions {
    PipelineOptions {
        timing: fast(),
        labels: Labels::Off {
            reason: "stream".into(),
        },
        stream: Some(stream_options(fake, diarize)),
        speakers: FollowerTiming {
            stream_speed: 20.0,
            ..follower_timing()
        },
        ..PipelineOptions::new(Arc::new(Offline))
    }
}

/// 11l: a stream that drops before the pipeline cut any passage still sends a reference for
/// every voice it named: the clips come from the stored segments, so no voice gets a second id.
#[test]
fn a_drop_before_any_passage_keeps_the_ids() {
    let tmp = TempDir::new("speakers-mistral-early-drop");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(8, 3.5);
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    // The labelling session drops 20 s into the call, after all three voices spoke.
    fake.state().fail_on = vec![200];
    let options = PipelineOptions {
        // The pipeline looks again only after the stream reconnected, so no passage exists
        // yet when the second session starts.
        timing: Timing {
            poll: Duration::from_secs(2),
            ..fast()
        },
        speakers: FollowerTiming {
            stream_speed: 1000.0,
            ..follower_timing()
        },
        ..mistral_options(&fake, true)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_done(&handle);
    handle.shutdown();
    let state = fake.state();
    let labelling: Vec<_> = state.sessions.iter().filter(|s| s.diarize).collect();
    assert_eq!(labelling.len(), 2, "one reconnect");
    let mut names: Vec<&str> = labelling[1]
        .references
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["call-1", "call-2", "call-3"]);
    drop(state);
    let all = events(&log, &rec.id());
    let call = last_revisions(&all, "call");
    let ids = assert_stable_ids(&call, &timeline_of(&pieces, 0));
    let mut ids: Vec<&str> = ids.values().map(String::as_str).collect();
    ids.sort();
    assert_eq!(ids, ["call-1", "call-2", "call-3"]);
}

/// 11k: a meeting stopped while paused keeps the words the stream answered after the padding
/// at the pause. Their estimated times fall in the padding and map to the last sample before
/// it, on the end of the last window, and no window comes after that one. On the call, labelled,
/// and on the microphone, whose answers are text deltas.
#[test]
fn words_answered_after_the_padding_survive_a_stop_while_paused() {
    for diarize in [true, false] {
        let tmp = TempDir::new(&format!("speakers-mistral-pause-stop-{diarize}"));
        let lib = Library::open(tmp.path()).unwrap();
        let log = EventLog::open(tmp.path()).unwrap();
        let pieces = [(0.5, None), (3.5, Some(0)), (1.2, None), (0.3, Some(1))];
        let rec = Recording::new(&lib, &[Track::Mic, Track::Call]);
        rec.session(0, 1);
        rec.segment("mic", 1, 0, &voices(&pieces));
        rec.segment("call", 1, 0, &voices(&pieces));
        rec.line(json!({"kind": "pause", "t": 5500}));
        let fake = FakeStreaming::default();
        let handle = Pipeline::start(
            rec.dir.clone(),
            log.clone(),
            mistral_options(&fake, diarize),
        );
        wait_until("both tracks answered up to the pause", WAIT, || {
            let answered = |track| meta(&rec.dir, &format!("stream.answered.{track}"));
            (answered("call").as_deref() == Some("5500")
                && answered("mic").as_deref() == Some("5500"))
            .then_some(())
        });
        rec.stop(9_000);
        wait_done(&handle);
        handle.shutdown();
        let all = events(&log, &rec.id());
        assert_no_repeats(&all);
        let timeline = timeline_of(&pieces, 0);
        every_burst_has_its_words(&last_revisions(&all, "call"), &timeline);
        every_burst_has_its_words(&last_revisions(&all, "mic"), &timeline);
        if diarize {
            let call = last_revisions(&all, "call");
            assert!(call.iter().all(|p| p.speaker.is_some()), "{call:#?}");
        }
    }
}

/// 11b: a session that drops while it finishes, after the whole call was sent, is followed by a
/// new one that sends what wasn't answered again, so the last voice still gets its passage.
#[test]
fn a_stream_that_drops_while_it_finishes_sends_the_rest_again() {
    let tmp = TempDir::new("speakers-mistral-drain");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    // The call ends while a voice talks, so its segment only comes when the session ends.
    let mut pieces = three_voices(4, 3.5);
    pieces.push((3.5, Some(1)));
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    fake.state().fail_ends = 1;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_done(&handle);
    handle.shutdown();

    let labelling = fake.state().sessions.iter().filter(|s| s.diarize).count();
    assert_eq!(labelling, 2, "a new session after the drop");
    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    let timeline = timeline_of(&pieces, 0);
    let call = last_revisions(&all, "call");
    let &(start, end, _) = timeline.last().unwrap();
    assert!(
        call.iter().any(|p| p.start < end && p.end > start),
        "nothing for the last voice at {start}..{end}: {call:#?}"
    );
    assert_eq!(assert_stable_ids(&call, &timeline).len(), 3);
}

/// 11c: the stored choice to label the call holds for the whole meeting, also when the stream
/// options of a later pipeline say otherwise.
#[test]
fn a_mistral_meeting_keeps_its_stored_label_choice() {
    let tmp = TempDir::new("speakers-mistral-stored");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(3, 3.5);
    let rec = finished_call(&lib, &pieces);
    // A pipeline before this one chose Mistral with labels.
    let t = Transcript::open(rec.dir.path()).unwrap();
    t.meta_set(MODE_KEY, "mistral").unwrap();
    t.meta_set(DIARIZE_KEY, "1").unwrap();
    drop(t);
    let fake = FakeStreaming::default();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, false));
    wait_done(&handle);
    handle.shutdown();
    let state = fake.state();
    assert_eq!(
        state.sessions.iter().filter(|s| s.diarize).count(),
        1,
        "the call is labelled as stored"
    );
    drop(state);
    let call = last_revisions(&events(&log, &rec.id()), "call");
    assert_eq!(
        assert_stable_ids(&call, &timeline_of(&pieces, 0)).len(),
        3,
        "{call:#?}"
    );
}

/// 11d: a track meeting.json didn't name, which the pipeline follows once the recorder writes
/// it, is streamed too, so its words aren't lost.
#[test]
fn mistral_streams_a_track_that_turns_up_later() {
    let tmp = TempDir::new("speakers-mistral-new-track");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(3, 3.5);
    let length = seconds(&pieces);
    let rec = Recording::new(&lib, &[Track::Mic]);
    rec.session(0, 1);
    rec.segment("mic", 1, 0, &voices(&[(length, None)]));
    rec.segment("call", 1, 0, &voices(&pieces));
    rec.stop(ms(length));
    let fake = FakeStreaming::default();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_done(&handle);
    handle.shutdown();
    assert_eq!(fake.state().sessions.len(), 2, "a session per track");
    let all = events(&log, &rec.id());
    assert_speakers_come_first(&all);
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 3, "{call:#?}");
    assert_eq!(assert_stable_ids(&call, &timeline_of(&pieces, 0)).len(), 3);
}

/// The words of a fake stream's segment for a burst that starts at `ms` of the session: every
/// burst of the call has its words in some call passage.
fn every_burst_has_its_words(call: &[Found], timeline: &[(u64, u64, usize)]) {
    for &(start, _, _) in timeline {
        let word = format!("w{start}");
        assert!(
            call.iter()
                .any(|p| p.text.split_whitespace().any(|w| w == word)),
            "no words for the burst at {start} ms: {call:#?}"
        );
    }
}

/// 11m: a delta that holds only the space between two words keeps them apart, on the microphone
/// and on a call without labels.
#[test]
fn spaces_sent_as_their_own_deltas_keep_the_words_apart() {
    let tmp = TempDir::new("speakers-mistral-spaces");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    // Three bursts of 3.5 s, ten words each.
    let pieces = three_voices(3, 3.5);
    let length = seconds(&pieces);
    let rec = Recording::new(&lib, &[Track::Mic, Track::Call]);
    rec.session(0, 1);
    rec.segment("mic", 1, 0, &voices(&pieces));
    rec.segment("call", 1, 0, &voices(&pieces));
    rec.stop(ms(length));
    let fake = FakeStreaming::default();
    fake.state().spaced_deltas = true;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, false));
    wait_done(&handle);
    handle.shutdown();
    let all = events(&log, &rec.id());
    let timeline = timeline_of(&pieces, 0);
    for track in ["mic", "call"] {
        let passages = last_revisions(&all, track);
        let words: Vec<&str> = passages
            .iter()
            .flat_map(|p| p.text.split_whitespace())
            .collect();
        assert_eq!(words.len(), 30, "{track}: {passages:#?}");
        assert!(
            words.iter().all(|w| w.matches('w').count() == 1),
            "{track}: words run together: {passages:#?}"
        );
        every_burst_has_its_words(&passages, &timeline);
    }
}

/// 11e: a session that fell behind and then dropped is followed by one that sends again what
/// the provider never answered, so no burst loses its words.
#[test]
fn a_stream_that_drops_while_behind_sends_the_unanswered_audio_again() {
    let tmp = TempDir::new("speakers-mistral-behind");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    // 57 s of call.
    let pieces = three_voices(12, 3.5);
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    {
        let mut state = fake.state();
        // The first labelling session answers nothing and drops 45 s into the call.
        state.mute_first = true;
        state.fail_on = vec![450];
    }
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_done(&handle);
    handle.shutdown();
    let labelling: Vec<_> = fake
        .state()
        .sessions
        .iter()
        .filter(|s| s.diarize)
        .cloned()
        .collect();
    assert_eq!(labelling.len(), 2, "a new session after the drop");
    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    // The second session started at the call's beginning: its stream ms are meeting ms.
    every_burst_has_its_words(&last_revisions(&all, "call"), &timeline_of(&pieces, 0));
}

/// The words of the call's passages, one count per burst: the passages that overlap it.
fn words_per_burst(call: &[Found], timeline: &[(u64, u64, usize)]) -> Vec<usize> {
    timeline
        .iter()
        .map(|&(start, end, _)| {
            call.iter()
                .filter(|p| p.start < end && p.end > start)
                .map(|p| p.text.split_whitespace().count())
                .sum()
        })
        .collect()
}

/// 11h: answers the store failed to write count as unanswered: the next session hears their
/// audio again, so no word is lost, and the voice in them gets no second id.
#[test]
fn answers_that_failed_to_store_are_sent_again() {
    let tmp = TempDir::new("speakers-mistral-store");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    // Six bursts of 3.5 s, ten words each.
    let pieces = three_voices(6, 3.5);
    let rec = finished_call(&lib, &pieces);
    drop(Transcript::open(rec.dir.path()).unwrap());
    // The first call session's third segment, the burst at 9.9 s, fails to store, as a write
    // past the busy timeout does.
    rusqlite::Connection::open(rec.dir.path().join("transcript.db"))
        .unwrap()
        .execute_batch(
            r#"CREATE TRIGGER refuse BEFORE INSERT ON stream_segments
               WHEN NEW.track = 'call' AND NEW.n = 2
                 AND NEW.epoch = (SELECT MIN(epoch) FROM speaker_epochs WHERE track = 'call')
               BEGIN SELECT RAISE(ABORT, 'disk I/O error'); END;"#,
        )
        .unwrap();
    let fake = FakeStreaming::default();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_done(&handle);
    handle.shutdown();
    let labelling = fake.state().sessions.iter().filter(|s| s.diarize).count();
    assert_eq!(labelling, 2, "a new session after the failed write");
    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    let timeline = timeline_of(&pieces, 0);
    let call = last_revisions(&all, "call");
    assert_eq!(words_per_burst(&call, &timeline), [10; 6], "{call:#?}");
    let ids = assert_stable_ids(&call, &timeline);
    let mut ids: Vec<&str> = ids.values().map(String::as_str).collect();
    ids.sort();
    assert_eq!(ids, ["call-1", "call-2", "call-3"]);
}

/// 11i: a session that never finishes after the stop failed rather than answered: the next
/// session sends the call again, and every burst gets its words before the meeting is done.
#[test]
fn a_stream_that_never_finishes_is_sent_again() {
    let tmp = TempDir::new("speakers-mistral-unfinished");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(3, 3.5);
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    // The first labelling session takes the audio, never answers and never finishes, as a
    // connection that stalled without a reset.
    fake.state().mute_first = true;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_done(&handle);
    handle.shutdown();
    let labelling = fake.state().sessions.iter().filter(|s| s.diarize).count();
    assert_eq!(
        labelling, 2,
        "a new session after the one that never finished"
    );
    let all = events(&log, &rec.id());
    assert_no_repeats(&all);
    // The second session started at the call's beginning: its stream ms are meeting ms.
    every_burst_has_its_words(&last_revisions(&all, "call"), &timeline_of(&pieces, 0));
}

/// 11k: a provider that is behind at the stop and answers slowly is waited for while it answers,
/// longer than the wait for a provider that answers nothing.
#[test]
fn a_stream_that_answers_slowly_after_the_stop_is_waited_for() {
    let tmp = TempDir::new("speakers-mistral-slow");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(3, 3.5);
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    // One answer per 0.9 s: the last three answers and the end take 3.6 s after the stream
    // ended, longer than the 2 s `follower_timing` waits without an answer.
    fake.state().slow_answers = Some(Duration::from_millis(900));
    let mut options = mistral_options(&fake, true);
    // The delay Voice asks Mistral for: late answers are placed that far before what was sent.
    options.stream.as_mut().unwrap().delay_ms = 480;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_done(&handle);
    handle.shutdown();
    let labelling = fake.state().sessions.iter().filter(|s| s.diarize).count();
    assert_eq!(labelling, 1, "the slow session finished");
    let all = events(&log, &rec.id());
    every_burst_has_its_words(&last_revisions(&all, "call"), &timeline_of(&pieces, 0));
}

/// 11j: a provider that never finishes a session fails the meeting after the stop, which keeps
/// its audio, instead of ending it `done` without the words.
#[test]
fn a_stream_that_never_finishes_fails_the_meeting() {
    let tmp = TempDir::new("speakers-mistral-never");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(2, 1.5);
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    fake.state().mute_all = true;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    let state = wait_until("the meeting to end", WAIT, || {
        let state = handle.state();
        state.is_final().then_some(state)
    });
    handle.shutdown();
    let PipelineState::Failed { reason } = state else {
        panic!("{state:?}");
    };
    assert!(reason.contains("stopped answering"), "{reason}");
    let sessions = fake.state().sessions.iter().filter(|s| s.diarize).count();
    // `fast()` gives up after three failures in a row.
    assert_eq!(sessions, 3);
}

/// 11o: after the stop, sessions that each answer part of what is left and then drop before they
/// finish are making progress. Every one moves the answered position forward, so the meeting
/// gets its tries back each time and finishes once a session gets through the rest, even when
/// that takes more sessions than the limit on failures in a row.
#[test]
fn sessions_that_answer_part_of_the_rest_and_drop_still_finish() {
    let tmp = TempDir::new("speakers-mistral-partial");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(9, 1.5);
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    {
        let mut state = fake.state();
        state.timed = true;
        // Every answer comes after the audio ended, while Voice waits for the session to
        // finish. Nine bursts, two answers a session: five sessions, four of them failing in
        // that wait, which is more than the three failures in a row `fast()` allows.
        state.answers_after_end = true;
        state.answers_per_session = Some(2);
    }
    let options = PipelineOptions {
        speakers: FollowerTiming {
            stream_speed: 20.0,
            stream_drain: Duration::from_millis(300),
            ..follower_timing()
        },
        ..mistral_options(&fake, true)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    let state = wait_until("the meeting to end", WAIT, || {
        let sessions = fake.state().sessions.iter().filter(|s| s.diarize).count();
        assert!(sessions <= 12, "{sessions} sessions and no end");
        let state = handle.state();
        state.is_final().then_some(state)
    });
    handle.shutdown();
    assert!(
        matches!(state, PipelineState::Done),
        "every session answered past the last one, so the meeting finishes: {state:?}"
    );
    let sessions = fake.state().sessions.iter().filter(|s| s.diarize).count();
    assert!(
        sessions > 3,
        "it took more sessions than the limit: {sessions}"
    );
    // Every burst arrives once: nine call passages with words, none lost and none twice. Where
    // they sit isn't checked here: answers that come only after the audio ended are later than
    // the cut the pipeline estimates from what was sent, so they land at that cut.
    let call = last_revisions(&events(&log, &rec.id()), "call");
    let worded = call.iter().filter(|p| !p.text.trim().is_empty()).count();
    assert_eq!(
        worded,
        timeline_of(&pieces, 0).len(),
        "{:?}",
        call.iter()
            .map(|p| (p.start, p.end, p.revision, p.text.as_str()))
            .collect::<Vec<_>>()
    );
}

/// 11n: after the stop, a provider that answers every session but never finishes runs out of
/// tries. Each new session sends the last audio again and hears the same words, which reach no
/// further than an earlier session's did, so they aren't progress: the meeting fails after the
/// limit instead of sending that audio again for ever.
#[test]
fn a_stream_that_answers_but_never_finishes_fails_the_meeting() {
    let tmp = TempDir::new("speakers-mistral-answers-unfinished");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    // The call ends while a voice talks, so every session answers that voice when it ends.
    let mut pieces = three_voices(2, 1.5);
    pieces.push((1.5, Some(2)));
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    fake.state().never_done = true;
    let options = PipelineOptions {
        speakers: FollowerTiming {
            stream_speed: 20.0,
            stream_drain: Duration::from_millis(300),
            ..follower_timing()
        },
        ..mistral_options(&fake, true)
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    let state = wait_until("the meeting to end", WAIT, || {
        let sessions = fake.state().sessions.iter().filter(|s| s.diarize).count();
        assert!(sessions <= 10, "{sessions} sessions and no end");
        let state = handle.state();
        state.is_final().then_some(state)
    });
    handle.shutdown();
    let PipelineState::Failed { reason } = state else {
        panic!("{state:?}");
    };
    assert!(reason.contains("stopped answering"), "{reason}");
    let sessions = fake.state().sessions.iter().filter(|s| s.diarize).count();
    // `fast()` gives up after three failures in a row.
    assert_eq!(sessions, 3);
    // The last session answered too: text without an end, not silence.
    let t = Transcript::open(rec.dir.path()).unwrap();
    let last = t
        .epochs()
        .unwrap()
        .into_iter()
        .filter(|e| e.track == Track::Call)
        .map(|e| e.epoch)
        .max()
        .unwrap();
    let answered = t.segments(Track::Call, 0, u64::MAX).unwrap();
    assert!(
        answered
            .iter()
            .any(|s| s.epoch == last && !s.text.trim().is_empty()),
        "{answered:#?}"
    );
}

/// 11f: after the stop, a provider that takes every connection and drops it at the first audio
/// runs out of tries, and the meeting fails instead of trying for ever.
#[test]
fn a_stream_that_always_drops_after_the_stop_fails_the_meeting() {
    let tmp = TempDir::new("speakers-mistral-give-up");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(2, 1.5);
    let rec = finished_call(&lib, &pieces);
    let fake = FakeStreaming::default();
    fake.state().fail_all = true;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_until("the meeting to fail", WAIT, || {
        matches!(handle.state(), PipelineState::Failed { .. }).then_some(())
    });
    handle.shutdown();
    let sessions = fake.state().sessions.iter().filter(|s| s.diarize).count();
    // `fast()` gives up after three failures in a row.
    assert!((3..=4).contains(&sessions), "{sessions} sessions");
}

/// 11g: a meeting whose realtime stream couldn't be set up yet, as without an API key, keeps
/// its label mode open: the windows fail meanwhile, and a pipeline that has the stream later
/// streams and labels the call, from `call-1` on.
#[test]
fn a_stream_without_its_key_yet_streams_once_it_has_one() {
    let tmp = TempDir::new("speakers-mistral-no-key");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(3, 3.5);
    let rec = finished_call(&lib, &pieces);
    let without = PipelineOptions {
        timing: fast(),
        labels: Labels::Off {
            reason: voice_meetings::STREAM_REASON.into(),
        },
        stream: None,
        ..PipelineOptions::new(Arc::new(Offline))
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), without);
    wait_until("the meeting to fail", WAIT, || {
        matches!(handle.state(), PipelineState::Failed { .. }).then_some(())
    });
    handle.shutdown();
    assert_eq!(
        meta(&rec.dir, MODE_KEY),
        None,
        "no mode before the stream exists"
    );
    assert!(
        !speakers(&events(&log, &rec.id()))
            .iter()
            .any(|(id, _)| id.starts_with("call-")),
        "no call speaker before the stream exists"
    );

    let fake = FakeStreaming::default();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_done(&handle);
    handle.shutdown();
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("mistral"));
    let all = events(&log, &rec.id());
    let call = last_revisions(&all, "call");
    let ids = assert_stable_ids(&call, &timeline_of(&pieces, 0));
    let mut numbers: Vec<String> = ids.into_values().collect();
    numbers.sort();
    assert_eq!(numbers, ["call-1", "call-2", "call-3"]);
}

/// 11h: a meeting that already streams to Mistral keeps that mode when a pipeline starts
/// without the stream, as after the API key was removed, also before any call speaker reached
/// Rukoo. With the key back, the next pipeline streams and labels the rest, instead of leaving
/// a meeting no recorded-window recognizer can transcribe.
#[test]
fn a_mistral_meeting_keeps_its_mode_while_its_stream_is_missing() {
    let tmp = TempDir::new("speakers-mistral-key-gone");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(3, 3.5);
    let rec = finished_call(&lib, &pieces);
    // The stream takes the meeting but never answers, so no call speaker reaches Rukoo.
    let mute = FakeStreaming::default();
    mute.state().mute_all = true;
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&mute, true));
    wait_until("the stream to give up", WAIT, || {
        handle.state().is_final().then_some(())
    });
    handle.shutdown();
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("mistral"));

    let without = PipelineOptions {
        timing: fast(),
        labels: Labels::Off {
            reason: voice_meetings::STREAM_REASON.into(),
        },
        stream: None,
        ..PipelineOptions::new(Arc::new(Offline))
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), without);
    wait_until("the keyless pipeline to settle", WAIT, || {
        (handle.labels().mode == "mistral" && handle.labels().state == LabelState::Failed)
            .then_some(())
    });
    // Long enough for the switch to labels off, which runs on every step.
    std::thread::sleep(Duration::from_millis(500));
    handle.shutdown();
    assert_eq!(
        meta(&rec.dir, MODE_KEY).as_deref(),
        Some("mistral"),
        "the stored mode stays while the stream is missing"
    );
    assert!(
        !speakers(&events(&log, &rec.id()))
            .iter()
            .any(|(id, _)| id.starts_with("call-")),
        "no call speaker without the stream"
    );

    let fake = FakeStreaming::default();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), mistral_options(&fake, true));
    wait_done(&handle);
    handle.shutdown();
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("mistral"));
    // Three voices, each with words. Where the passages sit isn't checked: the first session
    // answered nothing, so the cut it estimated runs ahead of the words (RUK-125).
    let call = last_revisions(&events(&log, &rec.id()), "call");
    let mut numbers: Vec<String> = call
        .iter()
        .filter(|p| !p.text.trim().is_empty())
        .filter_map(|p| p.speaker.clone())
        .collect();
    numbers.sort();
    numbers.dedup();
    assert_eq!(numbers, ["call-1", "call-2", "call-3"], "{call:#?}");
}

/// 12: Nemotron never falls back to Mistral: without a diarizer and without a stream, the call
/// is one speaker and no stream is reached.
#[test]
fn nemotron_never_falls_back_to_a_stream() {
    struct Counting(Arc<AtomicUsize>);
    impl voice_core::providers::stream::StreamingSpeech for Counting {
        fn connect(
            &self,
            _session: &voice_core::providers::stream::StreamSession,
        ) -> Result<Box<dyn voice_core::providers::stream::SpeechStream>, voice_core::ProviderError>
        {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(voice_core::ProviderError::provider("unreachable"))
        }
    }
    let tmp = TempDir::new("speakers-no-fallback");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(3, 1.5);
    let rec = finished_call(&lib, &pieces);
    let reached = Arc::new(AtomicUsize::new(0));
    // Stored nowhere the pipeline can reach.
    let _unused: Arc<dyn voice_core::providers::stream::StreamingSpeech> =
        Arc::new(Counting(reached.clone()));
    let options = PipelineOptions {
        timed: Some(Arc::new(Bursts)),
        timing: fast(),
        labels: Labels::Off {
            reason: "runtime".into(),
        },
        stream: None,
        ..PipelineOptions::new(Arc::new(Offline))
    };
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), options);
    wait_done(&handle);
    assert_eq!(reached.load(Ordering::SeqCst), 0);
    let all = events(&log, &rec.id());
    let call = last_revisions(&all, "call");
    assert_eq!(call.len(), 3);
    assert!(call.iter().all(|p| p.speaker.as_deref() == Some("call-1")));
    assert_eq!(meta(&rec.dir, REASON_KEY).as_deref(), Some("runtime"));
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// 13: a finished meeting opened again with an empty event log, as after a restore from the
/// bucket, sends its speakers before its passages and starts no follower.
#[test]
fn a_restored_meeting_sends_its_speakers_again_without_a_follower() {
    let tmp = TempDir::new("speakers-restore");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let pieces = three_voices(6, 1.5);
    let rec = finished_call(&lib, &pieces);
    let handle = Pipeline::start(
        rec.dir.clone(),
        log.clone(),
        labelled(&ScriptedDiarizer::new()),
    );
    wait_done(&handle);
    handle.shutdown();
    let before = last_revisions(&events(&log, &rec.id()), "call");

    let other = tmp.path().join("restored");
    let restored_lib = Library::open(&other).unwrap();
    let restored_log = EventLog::open(&other).unwrap();
    let target = restored_lib.root().join(rec.id());
    copy_dir(rec.dir.path(), &target);
    let dir = MeetingDir::at(&target).unwrap();
    // Without the bridge's own record of what it sent.
    Transcript::open(dir.path())
        .unwrap()
        .set_emitted(&HashMap::new())
        .unwrap();
    let d = ScriptedDiarizer::new();
    let handle = Pipeline::start(dir.clone(), restored_log.clone(), labelled(&d));
    wait_done(&handle);
    handle.shutdown();
    let script = d.script();
    assert_eq!((script.opens, script.pushes), (0, 0));
    drop(script);
    let all = events(&restored_log, &rec.id());
    assert_no_repeats(&all);
    let names: Vec<&str> = all.iter().take(5).map(|e| e.event.as_str()).collect();
    assert_eq!(
        names,
        [
            "meeting.started",
            "speaker",
            "speaker",
            "speaker",
            "speaker"
        ]
    );
    assert_eq!(
        speakers(&all),
        vec![
            ("mic".to_owned(), "You".to_owned()),
            ("call-1".to_owned(), "Speaker 1".to_owned()),
            ("call-2".to_owned(), "Speaker 2".to_owned()),
            ("call-3".to_owned(), "Speaker 3".to_owned()),
        ]
    );
    assert_eq!(all[5].event, "passage");
    assert_eq!(last_revisions(&all, "call"), before);
}

/// 14: a meeting Voice 0.1 started transcribing stays without labels; one without call passages
/// gets the normal choice.
#[test]
fn a_meeting_from_voice_0_1_keeps_call_1() {
    let tmp = TempDir::new("speakers-upgrade");
    let lib = Library::open(tmp.path()).unwrap();
    let log = EventLog::open(tmp.path()).unwrap();
    let rec = Recording::new(&lib, &[Track::Call]);
    rec.session(0, 1);
    let pieces = three_voices(6, 1.5);
    let call = voices(&pieces);
    let mut writer = rec.open_segment("call", 1, 0);
    writer.append(&call[..call.len() / 2]).unwrap();
    // Voice 0.1 stored two whole-call passages before it was updated.
    let passage = |start: u64, text: &str| voice_meetings::transcript::StoredPassage {
        passage: Passage {
            id: String::new(),
            track: Track::Call,
            speaker: Some("call-1".into()),
            start,
            end: start + 1_500,
            text: text.into(),
            revision: 1,
            dictation: false,
            language: None,
        },
        tokens: Vec::new(),
    };
    Transcript::open(rec.dir.path())
        .unwrap()
        .commit_window(
            Track::Call,
            0,
            ms(2.7 * 2.0 + 0.5) * 16,
            "pause",
            vec![passage(500, "w500"), passage(3_200, "w3200")],
        )
        .unwrap()
        .unwrap();
    let d = ScriptedDiarizer::new();
    let handle = Pipeline::start(rec.dir.clone(), log.clone(), labelled(&d));
    writer.append(&call[call.len() / 2..]).unwrap();
    rec.close_segment(writer, "call", 1, 0);
    rec.stop(ms(seconds(&pieces)));
    wait_done(&handle);
    assert_eq!(d.script().opens, 0, "no follower");
    assert_eq!(meta(&rec.dir, MODE_KEY).as_deref(), Some("off"));
    assert_eq!(meta(&rec.dir, REASON_KEY).as_deref(), Some("upgrade"));
    let status = handle.labels();
    assert_eq!(
        (status.mode.as_str(), status.reason.as_deref()),
        ("off", Some("upgrade"))
    );
    let all = events(&log, &rec.id());
    let call_passages = last_revisions(&all, "call");
    assert_eq!(call_passages.len(), 6);
    assert!(call_passages
        .iter()
        .all(|p| p.speaker.as_deref() == Some("call-1")));

    // The same kind of folder without call passages labels as usual.
    let fresh = finished_call(&lib, &three_voices(2, 1.5));
    let handle = Pipeline::start(
        fresh.dir.clone(),
        log.clone(),
        labelled(&ScriptedDiarizer::new()),
    );
    wait_done(&handle);
    assert_eq!(meta(&fresh.dir, MODE_KEY).as_deref(), Some("nemotron"));
}

/// The scorer passes the vectors JavaScript's scorer passes too.
#[test]
fn the_scorer_passes_the_shared_vectors() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/speaker-score-vectors.json");
    let vectors: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let spans = |list: &Value| -> Vec<Span> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|s| Span {
                start: s["start"].as_u64().unwrap(),
                end: s["end"].as_u64().unwrap(),
                speaker: s["speaker"].as_str().map(str::to_owned),
            })
            .collect()
    };
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for case in cases {
        let got = accuracy(&spans(&case["passages"]), &spans(&case["reference"]));
        let want = case["accuracy"].as_f64().unwrap();
        assert!(
            (got - want).abs() < 1e-9,
            "{}: {got} against {want}",
            case["name"]
        );
    }
}
