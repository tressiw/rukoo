//! Bucket sync through the whole engine against the real recorder and the fake bucket (spec
//! RUK-110 4.5), and media links through `media.open` and `media.read`.
//!
//! Like tests/meetings.rs, this binary has its own main (`harness = false`) so it can also be the
//! recorder and the screen process: run with `VOICE_ENGINE_TEST_ROLE=recorder` it calls
//! `voice_recorder::run`, or `voice_screen::run` for arguments without `--track`. Sources are
//! synthetic tones and the screen tests record the screen process's synthetic screen; no real
//! audio or screen is recorded.

mod support;

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};
use support::{
    bucket_kinds, connect_bucket, decrypt_all, fake_s3, fast_sync, skip_on_slow_host, sync_state,
    wait_for, Harness,
};
use voice_bridge::{ControlError, EventLog, MediaKind, MediaOpenArgs, MeetingControl};
use voice_engine::media::ScreenMedia;
use voice_engine::meetings::controller::ControllerTiming;
use voice_engine::meetings::screen::ScreenTiming;
use voice_engine::{Engine, StopReason};
use voice_meetings::{TrackReader, Transcript};
use voice_protocol::{ErrorCode, MeetingId, Speaker, Track};
use voice_recorder::timeline::{self, History, Line};
use voice_s3_fake::{FakeS3, Op};
use voice_screen_files::journal::Line as ScreenLine;
use voice_sync::{ScreenFile, ScreenFileKind, ScreenFiles};

const ROLE: &str = "VOICE_ENGINE_TEST_ROLE";

fn main() {
    struct Warnings;
    impl log::Log for Warnings {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Warn
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!("{}: {}", record.target(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static WARNINGS: Warnings = Warnings;
    let _ = log::set_logger(&WARNINGS);
    log::set_max_level(log::LevelFilter::Warn);
    if std::env::var(ROLE).as_deref() == Ok("recorder") {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // Only the recorder takes `--track`; the screen process never does.
        if args.iter().any(|a| a == "--track") {
            if let Err(e) = voice_recorder::run(&args) {
                eprintln!("rukoo-voice-recorder: {e}");
                std::process::exit(e.code);
            }
        } else if let Err(e) = voice_screen::run(&args) {
            eprintln!("rukoo-voice-screen: {e}");
            std::process::exit(e.code);
        }
        return;
    }
    // Single-threaded here, so setting the variables is safe; every recorder and screen process
    // started from now on inherits them. The screen process takes `--test` only in a development
    // copy.
    std::env::set_var(ROLE, "recorder");
    let scratch = std::env::temp_dir().join(format!("voice-engine-sync-{}", std::process::id()));
    std::env::set_var(voice_core::paths::DATA_DIR_ENV, &scratch);
    let tests: Vec<(&str, fn())> = vec![
        (
            "a_meeting_uploads_while_it_records",
            a_meeting_uploads_while_it_records,
        ),
        (
            "a_failing_bucket_never_slows_the_recorder",
            a_failing_bucket_never_slows_the_recorder,
        ),
        (
            "retention_keeps_audio_that_hasn_t_uploaded",
            retention_keeps_audio_that_hasn_t_uploaded,
        ),
        (
            "media_open_and_read_give_the_mix",
            media_open_and_read_give_the_mix,
        ),
        (
            "a_live_meeting_s_audio_link_grows_with_the_recording",
            a_live_meeting_s_audio_link_grows_with_the_recording,
        ),
        (
            "media_comes_back_from_the_bucket",
            media_comes_back_from_the_bucket,
        ),
        (
            "a_timeline_cant_point_a_link_outside_the_meeting",
            a_timeline_cant_point_a_link_outside_the_meeting,
        ),
        (
            "a_restored_meeting_reaches_the_bridge",
            a_restored_meeting_reaches_the_bridge,
        ),
        (
            "media_open_for_video_answers_within_50_ms",
            media_open_for_video_answers_within_50_ms,
        ),
        (
            "media_open_serves_a_frame_and_opens_a_gone_clip_again",
            media_open_serves_a_frame_and_opens_a_gone_clip_again,
        ),
        (
            "retention_releases_holds_when_the_disk_runs_low",
            retention_releases_holds_when_the_disk_runs_low,
        ),
        (
            "retention_holds_when_the_queue_is_unreadable",
            retention_holds_when_the_queue_is_unreadable,
        ),
        (
            "repairing_a_failing_bucket_keeps_held_audio",
            repairing_a_failing_bucket_keeps_held_audio,
        ),
        (
            "screen_files_upload_by_their_rules",
            screen_files_upload_by_their_rules,
        ),
        (
            "media_open_serves_a_frame_and_a_clip",
            media_open_serves_a_frame_and_a_clip,
        ),
        ("retention_holds_screen_media", retention_holds_screen_media),
        (
            "a_restored_meeting_keeps_its_speakers",
            a_restored_meeting_keeps_its_speakers,
        ),
        (
            "the_key_dialogs_wait_while_the_screen_records",
            the_key_dialogs_wait_while_the_screen_records,
        ),
    ];
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let mut failed = Vec::new();
    let mut ran = 0;
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        ran += 1;
        let started = Instant::now();
        match panic::catch_unwind(AssertUnwindSafe(test)) {
            Ok(()) => println!("test {name} ... ok ({} ms)", started.elapsed().as_millis()),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(name);
            }
        }
    }
    println!(
        "test result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        ran - failed.len(),
        failed.len()
    );
    if !failed.is_empty() {
        std::process::exit(101);
    }
}

// Helpers ------------------------------------------------------------------------------------

/// An engine on fakes that records with the real recorder: 1-second segments, a tone on each
/// track, and bucket sync's clocks shortened.
fn harness(name: &str, config: Value) -> Harness {
    let mut h = Harness::new(name);
    h.recorder_exe = std::env::current_exe().unwrap();
    h.timing = ControllerTiming {
        poll: Duration::from_millis(100),
        start_timeout: Duration::from_secs(20),
        backoff: vec![
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ],
        stable_after: Duration::from_secs(60),
        ping_failures: 3,
        recorder_args: vec!["--segment-ms".into(), "1000".into()],
    };
    h.test_sources = Some("mic=synthetic:tone:440,call=synthetic:tone:600".into());
    h.sync_timing = fast_sync();
    h.config(config);
    h
}

/// Waits up to `limit`, for the long waits of a failing bucket.
fn wait_long(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn meeting_dir(data: &Path, id: &str) -> PathBuf {
    data.join("meetings").join(id)
}

fn lines(data: &Path, id: &str) -> Vec<Line> {
    timeline::read(&meeting_dir(data, id).join("timeline.jsonl")).unwrap_or_default()
}

fn wait_recorded(data: &Path, id: &str, at_least: u64) {
    wait_long(
        &format!("{at_least} ms of audio"),
        Duration::from_millis(at_least) + Duration::from_secs(30),
        || {
            lines(data, id)
                .iter()
                .any(|l| matches!(l, Line::Heartbeat { t, .. } if *t >= at_least))
        },
    );
}

fn wait_done(data: &Path, id: &str) {
    wait_long("meeting.state done", Duration::from_secs(60), || {
        EventLog::open(data)
            .unwrap()
            .after(0)
            .unwrap()
            .iter()
            .any(|s| {
                s.meeting.as_deref() == Some(id)
                    && s.event == "meeting.state"
                    && s.data["state"] == "done"
            })
    });
}

/// Records `ms` of a meeting, stops it and waits until it is done.
fn record(engine: &Engine, data: &Path, ms: u64) -> String {
    let id = engine.start_meeting().unwrap();
    wait_recorded(data, &id, ms);
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(data, &id);
    id
}

/// The recovery code; tests keep it to open the bucket's objects.
fn recovery_code(engine: &Engine) -> String {
    engine.settings_sync_key("show", None).unwrap()["code"]
        .as_str()
        .unwrap()
        .to_string()
}

fn wait_uploaded(engine: &Engine) {
    wait_long("every file in the bucket", Duration::from_secs(60), || {
        let state = sync_state(engine);
        state["waiting"] == 0 && state["state"] == "idle"
    });
}

fn audio(meeting: &str, start: u64, end: Option<u64>) -> MediaOpenArgs {
    MediaOpenArgs {
        meeting: MeetingId::parse(meeting).unwrap(),
        kind: MediaKind::Audio,
        start,
        end,
    }
}

/// `media.open` until the file is ready; the ready answer.
fn open_ready(control: &dyn MeetingControl, args: &MediaOpenArgs) -> Value {
    let mut answer = Value::Null;
    wait_long("prepared media", Duration::from_secs(60), || {
        answer = control.media_open(args).expect("media.open");
        answer["ready"] == true
    });
    answer
}

/// The whole prepared file through `media.read`, in pieces of `piece` bytes.
fn read_all(control: &dyn MeetingControl, ready: &Value, piece: u64) -> Vec<u8> {
    let media = ready["media"].as_str().unwrap();
    let size = ready["size"].as_u64().unwrap();
    let mut bytes = Vec::new();
    while (bytes.len() as u64) < size {
        let offset = bytes.len() as u64;
        let reply = control.media_read(media, offset, piece).unwrap();
        assert_eq!(reply["offset"], offset);
        let data = base64::engine::general_purpose::STANDARD
            .decode(reply["data"].as_str().unwrap())
            .unwrap();
        assert!(!data.is_empty() && data.len() as u64 <= piece);
        bytes.extend(data);
    }
    assert_eq!(bytes.len() as u64, size);
    // Past the end there is nothing more.
    let past = control.media_read(media, size, piece).unwrap();
    assert_eq!(past["data"], "");
    bytes
}

/// The mix of every track from `start` to `end` ms, as a WAV file.
fn expected_mix(dir: &Path, start: u64, end: u64) -> Vec<u8> {
    let history = History::from_lines(&timeline::read(&dir.join("timeline.jsonl")).unwrap());
    let readers: Vec<TrackReader> = ["mic", "call"]
        .iter()
        .map(|track| TrackReader::new(dir, track, &history.segments))
        .collect();
    let (from, to) = (start * 16, end * 16);
    let mut sum = vec![0i32; (to - from) as usize];
    for reader in &readers {
        for (total, sample) in sum.iter_mut().zip(reader.read(from, to).unwrap()) {
            *total += i32::from(sample);
        }
    }
    let mut wav = voice_audio::segment_header(to - from).to_vec();
    for s in sum {
        wav.extend_from_slice(&(s.clamp(-32768, 32767) as i16).to_le_bytes());
    }
    wav
}

/// The meeting time just after the last recorded sample of any track, in ms rounded up.
fn recorded_end(dir: &Path) -> u64 {
    let history = History::from_lines(&timeline::read(&dir.join("timeline.jsonl")).unwrap());
    ["mic", "call"]
        .iter()
        .map(|track| TrackReader::new(dir, track, &history.segments).end())
        .max()
        .unwrap()
        .div_ceil(16)
}

fn code_of<T: std::fmt::Debug>(result: Result<T, ControlError>) -> ErrorCode {
    result.unwrap_err().code
}

fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

// Uploads ------------------------------------------------------------------------------------

fn a_meeting_uploads_while_it_records() {
    let h = harness("sync-live", json!({"meetings_retention": "keep"}));
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    let code = recovery_code(&engine);
    let id = engine.start_meeting().unwrap();
    wait_for("sync to say a meeting records", || {
        sync_state(&engine)["state"] == "live"
    });
    wait_recorded(&h.data(), &id, 4000);
    wait_for("audio in the bucket while the meeting records", || {
        bucket_kinds(&fake, &code).contains(&"audio")
    });
    let live = bucket_kinds(&fake, &code);
    assert!(live.contains(&"meeting"), "{live:?}");
    assert!(
        !live.contains(&"timeline") && !live.contains(&"transcript"),
        "the text waits for the stop: {live:?}"
    );
    assert!(!lines(&h.data(), &id)
        .iter()
        .any(|l| matches!(l, Line::Stop { .. })));

    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &id);
    wait_long(
        "the transcript, the timeline, the manifest and the index",
        Duration::from_secs(60),
        || {
            let kinds = bucket_kinds(&fake, &code);
            ["timeline", "transcript", "manifest", "index"]
                .iter()
                .all(|k| kinds.contains(k))
        },
    );
    wait_uploaded(&engine);
    assert!(sync_state(&engine)["lastUpload"].is_u64());

    // Every closed segment is in the bucket byte for byte, and so is meeting.json.
    let dir = meeting_dir(&h.data(), &id);
    let objects = decrypt_all(&fake, &code);
    let history = History::from_lines(&lines(&h.data(), &id));
    assert!(
        history.segments.len() >= 8,
        "{} segments",
        history.segments.len()
    );
    for segment in &history.segments {
        let bytes = std::fs::read(dir.join(&segment.file)).unwrap();
        assert!(
            objects.iter().any(|(_, plain)| *plain == bytes),
            "{} isn't in the bucket",
            segment.file
        );
    }
    let meeting_json = std::fs::read(dir.join("meeting.json")).unwrap();
    assert!(objects.iter().any(|(_, plain)| *plain == meeting_json));
    // No object name shows the meeting or what it holds.
    for (name, _) in &objects {
        assert!(!name.contains("m-20") && !name.contains(".wav"), "{name}");
    }
    engine.shutdown();
    fake.stop();
}

/// Every heartbeat within 1,300 ms of the one before, no lost device or crash, each segment
/// closed and following the one before, and a stop.
fn assert_steady(data: &Path, id: &str) {
    let lines = lines(data, id);
    let mut last: Option<u64> = None;
    for line in &lines {
        match line {
            Line::Heartbeat { t, .. } => {
                if let Some(before) = last {
                    assert!(t - before <= 1300, "{id}: heartbeats {before} and {t}");
                }
                last = Some(*t);
            }
            Line::DeviceLost { .. } | Line::Crash { .. } => panic!("{id}: {line:?}"),
            _ => {}
        }
    }
    assert!(
        matches!(lines.last(), Some(Line::Stop { .. })),
        "{id} has no stop"
    );
    let history = History::from_lines(&lines);
    for track in ["mic", "call"] {
        let mut segments: Vec<_> = history
            .segments
            .iter()
            .filter(|s| s.track == track)
            .collect();
        segments.sort_by_key(|s| s.seq);
        assert!(segments.len() >= 20, "{id}/{track}: {}", segments.len());
        for pair in segments.windows(2) {
            assert_eq!(pair[1].seq, pair[0].seq + 1, "{id}/{track}");
            let end = pair[0].t + pair[0].ended.expect("a closed segment") / 16;
            assert!(
                pair[1].t.abs_diff(end) <= 200,
                "{id}/{track}: a gap between {} and {}",
                pair[0].seq,
                pair[1].seq
            );
        }
        assert!(segments.iter().all(|s| s.ended.is_some()), "{id}/{track}");
    }
}

fn a_failing_bucket_never_slows_the_recorder() {
    // Its heartbeat limit (1,300 ms for a 1 s heartbeat) is a timing window.
    if skip_on_slow_host("a_failing_bucket_never_slows_the_recorder") {
        return;
    }
    let h = harness("sync-failing", json!({"meetings_retention": "keep"}));
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    type Fault = fn(&FakeS3);
    let faults: [(&str, Fault); 3] = [
        ("a bucket that stalls every answer for 30 s", |fake| {
            fake.faults().stall = Some(Duration::from_secs(30));
        }),
        ("a bucket that refuses connections", |fake| {
            fake.faults().refuse = true;
        }),
        ("a bucket that answers 500", |fake| {
            let mut faults = fake.faults();
            for op in [Op::Put, Op::Create, Op::UploadPart, Op::Complete] {
                faults
                    .fail
                    .extend(std::iter::repeat_n((op, 500, "InternalError"), 10_000));
            }
        }),
    ];
    let mut meetings = Vec::new();
    for (what, fault) in faults {
        println!("  {what}");
        *fake.faults() = Default::default();
        fault(&fake);
        let id = engine.start_meeting().unwrap();
        wait_long("sync to retry", Duration::from_secs(40), || {
            sync_state(&engine)["state"] == "retrying"
        });
        wait_recorded(&h.data(), &id, 25_000);
        engine.stop_meeting(StopReason::User).unwrap();
        wait_long("the stop", Duration::from_secs(30), || {
            lines(&h.data(), &id)
                .iter()
                .any(|l| matches!(l, Line::Stop { .. }))
        });
        assert_steady(&h.data(), &id);
        meetings.push(id);
    }
    // Healthy again: everything goes up.
    *fake.faults() = Default::default();
    for id in &meetings {
        wait_done(&h.data(), id);
    }
    wait_long("the queue to drain", Duration::from_secs(120), || {
        let state = sync_state(&engine);
        state["waiting"] == 0 && state["state"] == "idle"
    });
    let code = recovery_code(&engine);
    let kinds = bucket_kinds(&fake, &code);
    let audio = kinds.iter().filter(|k| **k == "audio").count();
    let segments: usize = meetings
        .iter()
        .map(|id| History::from_lines(&lines(&h.data(), id)).segments.len())
        .sum();
    assert_eq!(audio, segments, "every segment once");
    assert_eq!(kinds.iter().filter(|k| **k == "timeline").count(), 3);
    engine.shutdown();
    fake.stop();
}

// Retention ----------------------------------------------------------------------------------

fn retention_keeps_audio_that_hasn_t_uploaded() {
    // "Remove after transcription": the meeting's time is up as soon as it is done.
    let h = harness(
        "sync-retention",
        json!({"meetings_retention": "after_transcription"}),
    );
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    fake.faults().refuse = true;
    let held = record(&engine, &h.data(), 2000);
    // The engine sweeps right after it reads `done`, within a second or two.
    std::thread::sleep(Duration::from_millis(3000));
    let audio = meeting_dir(&h.data(), &held).join("audio");
    assert!(audio.is_dir(), "audio that isn't in the bucket stays");
    assert!(sync_state(&engine)["waiting"].as_u64().unwrap() > 0);

    // Once it is in the bucket, the next sweep removes it.
    fake.faults().refuse = false;
    wait_long("the uploaded audio to go", Duration::from_secs(60), || {
        !audio.exists()
    });
    assert!(meeting_dir(&h.data(), &held)
        .join("timeline.jsonl")
        .is_file());

    // With sync off, a meeting's audio goes as soon as it is done.
    engine
        .settings_set("storage.sync.enabled", json!(false))
        .unwrap();
    fake.faults().refuse = true;
    let free = record(&engine, &h.data(), 2000);
    wait_for("the audio to go without sync", || {
        !meeting_dir(&h.data(), &free).join("audio").exists()
    });
    engine.shutdown();
    fake.stop();
}

fn retention_releases_holds_when_the_disk_runs_low() {
    let h = harness(
        "sync-floor",
        json!({"meetings_retention": "after_transcription"}),
    );
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    fake.faults().refuse = true;
    let older = record(&engine, &h.data(), 2000);
    let newer = record(&engine, &h.data(), 2000);
    assert!(older < newer);
    let live = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &live, 1000);
    for id in [&older, &newer, &live] {
        assert!(meeting_dir(&h.data(), id).join("audio").is_dir(), "{id}");
    }
    // 9 GiB free until the older meeting's audio goes, then 10 GiB.
    let older_audio = meeting_dir(&h.data(), &older).join("audio");
    h.set_free(move |_| {
        Some(if older_audio.exists() {
            9 << 30
        } else {
            10 << 30
        })
    });
    // A new start sweeps at once; the recorder records on meanwhile.
    engine.shutdown();
    let engine = h.start();
    wait_for("the older meeting's audio to go", || {
        !meeting_dir(&h.data(), &older).join("audio").exists()
    });
    assert!(meeting_dir(&h.data(), &newer).join("audio").is_dir());
    assert!(meeting_dir(&h.data(), &live).join("audio").is_dir());
    wait_for("the lost files counted", || {
        sync_state(&engine)["lost"].as_u64().unwrap_or(0) > 0
    });
    let notes: Vec<_> = h
        .sink
        .notes()
        .into_iter()
        .filter(|n| n.title_key == voice_engine::NOTIFY_SYNC)
        .collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].body.contains("hadn't reached your bucket"));
    engine.stop_meeting(StopReason::User).unwrap();
    wait_long("the stop", Duration::from_secs(30), || {
        lines(&h.data(), &live)
            .iter()
            .any(|l| matches!(l, Line::Stop { .. }))
    });
    engine.shutdown();
    fake.stop();
}

/// The usual repair of a failing bucket: a new secret, a corrected field, or the secret removed
/// and saved again. Each pauses uploads until Test connection passes, and none may let retention
/// take audio that never reached the bucket (D6, D8, spec 2.15 "Wrong secret").
fn repairing_a_failing_bucket_keeps_held_audio() {
    let h = harness(
        "sync-repair",
        json!({"meetings_retention": "after_transcription"}),
    );
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    fake.faults().refuse = true;
    let held = record(&engine, &h.data(), 2000);
    std::thread::sleep(Duration::from_millis(3000));
    let audio = meeting_dir(&h.data(), &held).join("audio");
    assert!(audio.is_dir(), "held for the bucket");

    let sync_settings =
        |engine: &Engine| engine.settings_state()["config"]["storage"]["sync"].clone();
    engine
        .settings_sync_save_secret("a-replacement-secret".into())
        .unwrap();
    assert_eq!(
        sync_settings(&engine)["enabled"],
        true,
        "the switch stays on"
    );
    assert_eq!(sync_settings(&engine)["tested"], false);
    assert_eq!(sync_state(&engine)["state"], "paused");
    engine
        .settings_set("storage.sync.bucket", json!("corrected-bucket"))
        .unwrap();
    engine.settings_remove_key("sync").unwrap();
    assert_eq!(sync_state(&engine)["state"], "paused");
    // Longer than the engine waits before it sweeps again after a sync change.
    std::thread::sleep(Duration::from_millis(7000));
    assert!(audio.is_dir(), "a paused sync keeps holding");
    // A start sweeps at once.
    engine.shutdown();
    let engine = h.start();
    std::thread::sleep(Duration::from_millis(3000));
    assert!(audio.is_dir(), "still held after a sweep");
    assert_eq!(sync_state(&engine)["state"], "paused");

    // Repaired: the right bucket and secret pass the test, uploads go on by themselves, and the
    // audio goes once it is in the bucket.
    engine
        .settings_set("storage.sync.bucket", json!(support::BUCKET))
        .unwrap();
    engine
        .settings_sync_save_secret(support::BUCKET_SECRET.into())
        .unwrap();
    fake.faults().refuse = false;
    engine.settings_sync_test().unwrap();
    assert_ne!(sync_state(&engine)["state"], "paused");
    wait_long("the uploaded audio to go", Duration::from_secs(60), || {
        !audio.exists()
    });
    engine.shutdown();
    fake.stop();
}

/// An empty SQLite database whose `user_version` is 2: a queue from a newer Voice.
fn newer_queue_db() -> Vec<u8> {
    let mut db = vec![0u8; 4096];
    db[..16].copy_from_slice(b"SQLite format 3\0");
    db[16..18].copy_from_slice(&4096u16.to_be_bytes());
    db[18] = 1;
    db[19] = 1;
    db[21] = 64;
    db[22] = 32;
    db[23] = 32;
    db[24..28].copy_from_slice(&1u32.to_be_bytes()); // change counter
    db[28..32].copy_from_slice(&1u32.to_be_bytes()); // pages
    db[40..44].copy_from_slice(&1u32.to_be_bytes()); // schema cookie
    db[44..48].copy_from_slice(&4u32.to_be_bytes()); // schema format
    db[56..60].copy_from_slice(&1u32.to_be_bytes()); // UTF-8
    db[60..64].copy_from_slice(&2u32.to_be_bytes()); // user_version
    db[92..96].copy_from_slice(&1u32.to_be_bytes());
    db[96..100].copy_from_slice(&3_045_000u32.to_be_bytes());
    // Page 1: an empty table leaf whose cells would start at the end of the page.
    db[100] = 0x0D;
    db[105..107].copy_from_slice(&4096u16.to_be_bytes());
    db
}

fn retention_holds_when_the_queue_is_unreadable() {
    // Sync is on for a tested bucket, but its queue is from a newer Voice.
    let mut sync = json!({
        "enabled": true, "service": "other", "endpoint": "http://127.0.0.1:9",
        "region": "us-east-1", "bucket": "test", "accessKeyId": "test", "urlStyle": "path"
    });
    sync["tested"] = json!(voice_sync::SyncConfig::from_json(&sync)
        .unwrap()
        .fingerprint());
    let h = harness(
        "sync-unreadable",
        json!({"meetings_retention": "after_transcription", "storage_sync": sync}),
    );
    let queue = h.data().join("sync");
    std::fs::create_dir_all(&queue).unwrap();
    std::fs::write(queue.join("queue.db"), newer_queue_db()).unwrap();
    let engine = h.start();
    let view = sync_state(&engine);
    assert_eq!(view["available"], false);
    assert_eq!(engine.settings_state()["features"]["sync"], false);
    // What Settings › Storage shows with the switch instead of the sync card: nothing uploads, and
    // why under Error details.
    assert_eq!(view["state"], "paused");
    assert_eq!(view["lastError"]["code"], "failed");
    assert!(
        view["lastError"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("from a newer Voice")),
        "{view}"
    );
    let id = record(&engine, &h.data(), 2000);
    std::thread::sleep(Duration::from_millis(3000));
    let audio = meeting_dir(&h.data(), &id).join("audio");
    assert!(audio.is_dir(), "a broken queue never lets audio go");

    // Turning the switch off, which that page keeps, is the way out.
    engine
        .settings_set("storage.sync.enabled", json!(false))
        .unwrap();
    wait_for("the audio to go with the switch off", || !audio.exists());
    engine.shutdown();
}

// Media links --------------------------------------------------------------------------------

fn a_live_meeting_s_audio_link_grows_with_the_recording() {
    let h = harness("media-live", json!({"meetings_retention": "keep"}));
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &id, 1500);
    let control = engine.meeting_control().unwrap();
    // Ten minutes asked while about two seconds are recorded: cut to what is there.
    let args = audio(&id, 0, Some(600_000));
    let first = open_ready(&*control, &args);
    let held = first["end"].as_u64().unwrap();
    assert!(held < 600_000, "{first}");
    // Once the meeting recorded on, the same arguments mix up to the new end instead of handing
    // back the short file.
    wait_recorded(&h.data(), &id, held + 2000);
    let second = open_ready(&*control, &args);
    assert!(
        second["end"].as_u64().unwrap() > held,
        "{first} then {second}"
    );
    assert_ne!(second["media"], first["media"], "a new mix");
    // The short mix still answers for the range it holds.
    let again = open_ready(&*control, &audio(&id, 0, Some(held)));
    assert_eq!(again["media"], first["media"]);
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &id);
}

fn media_open_and_read_give_the_mix() {
    let h = harness("media-mix", json!({"meetings_retention": "keep"}));
    let engine = h.start();
    let id = record(&engine, &h.data(), 3000);
    let control = engine.meeting_control().unwrap();
    let dir = meeting_dir(&h.data(), &id);
    let end = recorded_end(&dir);
    assert!(end < 20_000);

    // 0 to 20 s: cut to what was recorded.
    let args = audio(&id, 0, Some(20_000));
    let first = control.media_open(&args).unwrap();
    assert_eq!(first["ready"], false, "{first}");
    let ready = open_ready(&*control, &args);
    assert_eq!(
        ready["media"], first["media"],
        "the same arguments, the same media"
    );
    assert_eq!(
        (
            &ready["type"],
            &ready["start"],
            &ready["end"],
            &ready["size"]
        ),
        (
            &json!("audio/wav"),
            &json!(0),
            &json!(end),
            &json!(44 + end * 32)
        )
    );
    let wav = read_all(&*control, &ready, 100_000);
    assert_eq!(wav, expected_mix(&dir, 0, end));
    // The mix holds both tones, not silence.
    assert!(wav[44..].iter().any(|b| *b != 0));

    // A range inside, read in other pieces.
    let inner = open_ready(&*control, &audio(&id, 1000, Some(2500)));
    assert_eq!(
        read_all(&*control, &inner, 7_001),
        expected_mix(&dir, 1000, 2500)
    );

    // What isn't there.
    assert_eq!(
        code_of(control.media_open(&audio(&id, end, Some(end + 1000)))),
        ErrorCode::NotFound
    );
    assert_eq!(
        code_of(control.media_open(&audio(&id, 0, None))),
        ErrorCode::BadRequest
    );
    assert_eq!(
        code_of(control.media_open(&audio("m-20000101-000000-abcd", 0, Some(1000)))),
        ErrorCode::NotFound
    );
    let frame = MediaOpenArgs {
        kind: MediaKind::Frame,
        ..audio(&id, 0, None)
    };
    assert_eq!(code_of(control.media_open(&frame)), ErrorCode::NotAvailable);
    let video = MediaOpenArgs {
        kind: MediaKind::Video,
        ..audio(&id, 0, Some(400_000))
    };
    assert_eq!(code_of(control.media_open(&video)), ErrorCode::BadRequest);
    assert_eq!(
        code_of(control.media_read("md-AAAAAAAAAAAAAAAAAAAAAA", 0, 10)),
        ErrorCode::NotFound
    );
    // Nothing is left behind but the prepared mixes.
    let prepared = std::fs::read_dir(h.data().join("media")).unwrap().count();
    assert_eq!(prepared, 2);
    engine.shutdown();
}

fn media_comes_back_from_the_bucket() {
    let h = harness("media-fetch", json!({"meetings_retention": "keep"}));
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    let id = record(&engine, &h.data(), 3000);
    wait_uploaded(&engine);
    let control = engine.meeting_control().unwrap();
    let dir = meeting_dir(&h.data(), &id);
    let before = read_all(
        &*control,
        &open_ready(&*control, &audio(&id, 0, Some(20_000))),
        1 << 20,
    );

    // Retention removed the audio here; the bucket still has it.
    std::fs::remove_dir_all(dir.join("audio")).unwrap();
    let args = audio(&id, 0, Some(20_001));
    let first = control.media_open(&args).unwrap();
    assert_eq!(first["ready"], false, "{first}");
    let after = read_all(&*control, &open_ready(&*control, &args), 1 << 20);
    assert_eq!(after, before, "the same mix from the bucket");
    let history = History::from_lines(&lines(&h.data(), &id));
    assert!(history.segments.iter().all(|s| dir.join(&s.file).is_file()));

    // Fetched audio stays a day after its last use, not after its fetch (D8): another link to it
    // a little later, from the files now on disk, moves the hold on.
    let fetched = wall_ms();
    std::thread::sleep(Duration::from_millis(400));
    let _ = read_all(
        &*control,
        &open_ready(&*control, &audio(&id, 0, Some(20_003))),
        1 << 20,
    );
    let queue = voice_sync::queue::Queue::open(&h.data().join("sync")).unwrap();
    let later = fetched + voice_sync::queue::RESTORED_HOLD_MS + 200;
    assert!(
        queue.holds_media(&id, false, true, later).unwrap(),
        "the hold counts from the last use"
    );
    drop(queue);

    // With sync off, Voice never asks the bucket.
    engine
        .settings_set("storage.sync.enabled", json!(false))
        .unwrap();
    std::fs::remove_dir_all(dir.join("audio")).unwrap();
    assert_eq!(
        code_of(control.media_open(&audio(&id, 0, Some(20_002)))),
        ErrorCode::NotAvailable
    );
    engine.shutdown();
    fake.stop();
}

fn a_timeline_cant_point_a_link_outside_the_meeting() {
    let h = harness("media-outside", json!({"meetings_retention": "keep"}));
    // A file elsewhere on this computer, and a meeting whose timeline names it twice, as one from
    // the bucket could: by its absolute path and through `..`.
    let outside = h.data().join("outside.pdf");
    let bytes: Vec<u8> = b"NOT-A-MEETING-FILE-"
        .iter()
        .cycle()
        .take(200_044)
        .copied()
        .collect();
    std::fs::create_dir_all(h.data()).unwrap();
    std::fs::write(&outside, &bytes).unwrap();
    let created_at = wall_ms() - 60_000;
    let id = MeetingId::with_suffix(created_at, 0x0abc).into_string();
    let dir = meeting_dir(&h.data(), &id);
    std::fs::create_dir_all(&dir).unwrap();
    let meeting = json!({
        "id": id, "title": "Outside", "createdAt": created_at, "tracks": ["mic"],
        "sources": { "mic": "default-input" }, "screen": "off", "startedBy": "voice",
        "context": null, "retention": "keep", "version": 1
    });
    std::fs::write(
        dir.join("meeting.json"),
        serde_json::to_vec(&meeting).unwrap(),
    )
    .unwrap();
    let timeline = [
        json!({"kind":"session","t":0,"wall":created_at,"session":1,"pid":1,"resumed":false,
            "version":1,"recorder":"0.1.0"}),
        json!({"kind":"segment","t":0,"track":"mic","seq":1,"file":outside.to_string_lossy()}),
        json!({"kind":"segment_end","t":6000,"track":"mic","seq":1,"samples":96_000}),
        json!({"kind":"segment","t":6000,"track":"mic","seq":2,"file":"../../outside.pdf"}),
        json!({"kind":"segment_end","t":12000,"track":"mic","seq":2,"samples":96_000}),
        json!({"kind":"stop","t":12000,"wall":created_at + 12_000,"reason":"user"}),
    ];
    let text: String = timeline.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(dir.join("timeline.jsonl"), text).unwrap();

    let engine = h.start();
    let control = engine.meeting_control().unwrap();
    // Neither segment names its own file, so nothing was recorded as far as a link goes.
    assert_eq!(
        code_of(control.media_open(&audio(&id, 0, Some(12_000)))),
        ErrorCode::NotFound
    );
    engine.shutdown();
}

/// Stand-in screen media: one frame at 1 s, and one video file whose "clip" is a copy of it.
struct StandInScreen;

const VIDEO: &str = "screen/video/000001.mp4";
const FRAME: &str = "screen/frames/f-000001000.jpg";
const FRAME_AT: u64 = 1000;

fn stand_in_jpeg() -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    bytes.extend_from_slice(b"JFIF\0");
    bytes.extend((0..20_000u32).map(|i| (i % 241) as u8));
    bytes.extend_from_slice(&[0xFF, 0xD9]);
    bytes
}

fn stand_in_mp4() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&24u32.to_be_bytes());
    bytes.extend_from_slice(b"ftypisom\0\0\x02\0isomiso2");
    bytes.extend_from_slice(&8u32.to_be_bytes());
    bytes.extend_from_slice(b"moov");
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    bytes.extend_from_slice(&(8 + data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(b"mdat");
    bytes.extend(data);
    bytes
}

impl ScreenFiles for StandInScreen {
    fn list(&self, meeting_dir: &Path, stopped: bool) -> std::io::Result<Vec<ScreenFile>> {
        let mut files = Vec::new();
        if meeting_dir.join(FRAME).is_file() {
            files.push(ScreenFile {
                path: FRAME.into(),
                kind: ScreenFileKind::Frame,
                closed: true,
            });
        }
        if meeting_dir.join(VIDEO).is_file() {
            files.push(ScreenFile {
                path: VIDEO.into(),
                kind: ScreenFileKind::Video,
                closed: stopped,
            });
        }
        Ok(files)
    }
}

impl ScreenMedia for StandInScreen {
    fn frame_at(&self, _dir: &Path, at: u64) -> std::io::Result<Option<(String, u64)>> {
        Ok((at >= FRAME_AT).then(|| (FRAME.to_string(), FRAME_AT)))
    }

    fn clip_inputs(&self, _dir: &Path, _start: u64, _end: u64) -> std::io::Result<Vec<String>> {
        Ok(vec![VIDEO.into()])
    }

    fn cut(&self, dir: &Path, start: u64, end: u64) -> Result<(PathBuf, u64, u64), ControlError> {
        let clips = dir.join("screen").join("clips");
        std::fs::create_dir_all(&clips).map_err(|e| ControlError::failed(e.to_string()))?;
        let clip = clips.join(format!("c-{start:09}-{end:09}-000001.mp4"));
        std::fs::copy(dir.join(VIDEO), &clip).map_err(|e| ControlError::failed(e.to_string()))?;
        Ok((clip, start, end))
    }

    fn recorded(&self, _dir: &Path) -> (bool, bool) {
        (true, true)
    }
}

fn media_open_for_video_answers_within_50_ms() {
    let mut h = harness("media-video", json!({"meetings_retention": "keep"}));
    h.screen_files = Arc::new(StandInScreen);
    h.screen_media = Arc::new(StandInScreen);
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    let code = recovery_code(&engine);
    let id = record(&engine, &h.data(), 2000);
    let dir = meeting_dir(&h.data(), &id);
    let video = stand_in_mp4();
    std::fs::create_dir_all(dir.join("screen").join("video")).unwrap();
    std::fs::write(dir.join(VIDEO), &video).unwrap();
    wait_long("the video in the bucket", Duration::from_secs(60), || {
        bucket_kinds(&fake, &code).contains(&"mp4")
    });
    wait_uploaded(&engine);
    std::fs::remove_file(dir.join(VIDEO)).unwrap();

    // The bucket answers slowly; Voice still answers at once.
    fake.faults().stall = Some(Duration::from_secs(2));
    let control = engine.meeting_control().unwrap();
    let args = MediaOpenArgs {
        kind: MediaKind::Video,
        ..audio(&id, 0, Some(10_000))
    };
    let asked = Instant::now();
    let first = control.media_open(&args).unwrap();
    let took = asked.elapsed();
    assert_eq!(first["ready"], false, "{first}");
    assert!(took < Duration::from_millis(50), "media.open took {took:?}");
    let asked = Instant::now();
    let _ = control.status();
    let took = asked.elapsed();
    assert!(took < Duration::from_millis(50), "status took {took:?}");

    let ready = open_ready(&*control, &args);
    assert_eq!(ready["type"], "video/mp4");
    assert_eq!(
        (&ready["start"], &ready["end"]),
        (&json!(0), &json!(10_000))
    );
    assert_eq!(read_all(&*control, &ready, 1 << 20), video);
    assert!(dir.join(VIDEO).is_file(), "the video came back");
    engine.shutdown();
    fake.stop();
}

/// What `media_open_serves_a_frame_and_a_clip` can't reach with a real screen recording: a frame
/// fetched back while the meeting's other media stays, and a clip that went mid-read, on
/// stand-in screen files that run on every platform.
fn media_open_serves_a_frame_and_opens_a_gone_clip_again() {
    let mut h = harness("media-frame", json!({"meetings_retention": "keep"}));
    h.screen_files = Arc::new(StandInScreen);
    h.screen_media = Arc::new(StandInScreen);
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    let code = recovery_code(&engine);
    let id = record(&engine, &h.data(), 2000);
    let dir = meeting_dir(&h.data(), &id);
    let (jpeg, video) = (stand_in_jpeg(), stand_in_mp4());
    std::fs::create_dir_all(dir.join("screen").join("frames")).unwrap();
    std::fs::create_dir_all(dir.join("screen").join("video")).unwrap();
    std::fs::write(dir.join(FRAME), &jpeg).unwrap();
    std::fs::write(dir.join(VIDEO), &video).unwrap();
    wait_long(
        "the frame and the video in the bucket",
        Duration::from_secs(60),
        || {
            let kinds = bucket_kinds(&fake, &code);
            kinds.contains(&"jpeg") && kinds.contains(&"mp4")
        },
    );
    wait_uploaded(&engine);
    let control = engine.meeting_control().unwrap();
    let frame = |start| MediaOpenArgs {
        kind: MediaKind::Frame,
        ..audio(&id, start, None)
    };

    // A frame on disk is ready at once, at its own time.
    let ready = control.media_open(&frame(1500)).unwrap();
    assert_eq!(
        (
            &ready["ready"],
            &ready["type"],
            &ready["start"],
            &ready["end"]
        ),
        (
            &json!(true),
            &json!("image/jpeg"),
            &json!(FRAME_AT),
            &json!(FRAME_AT)
        ),
        "{ready}"
    );
    assert_eq!(read_all(&*control, &ready, 7_001), jpeg);
    assert_eq!(
        code_of(control.media_open(&frame(500))),
        ErrorCode::NotFound
    );

    // Gone from this computer: the same arguments prepare it again from the bucket.
    std::fs::remove_file(dir.join(FRAME)).unwrap();
    let again = control.media_open(&frame(1500)).unwrap();
    assert_eq!(again["ready"], false, "{again}");
    let back = open_ready(&*control, &frame(1500));
    assert_eq!(read_all(&*control, &back, 1 << 20), jpeg);
    assert!(dir.join(FRAME).is_file(), "the frame came back");

    // A clip removed mid-read answers not_found; opening it again cuts it again.
    let args = MediaOpenArgs {
        kind: MediaKind::Video,
        ..audio(&id, 0, Some(10_000))
    };
    let clip = open_ready(&*control, &args);
    let media = clip["media"].as_str().unwrap().to_string();
    control.media_read(&media, 0, 1000).unwrap();
    let clips: Vec<PathBuf> = std::fs::read_dir(dir.join("screen").join("clips"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(clips.len(), 1, "{clips:?}");
    std::fs::remove_file(&clips[0]).unwrap();
    assert_eq!(
        code_of(control.media_read(&media, 1000, 1000)),
        ErrorCode::NotFound
    );
    let reopened = open_ready(&*control, &args);
    assert_ne!(reopened["media"], clip["media"], "a new preparation");
    assert_eq!(read_all(&*control, &reopened, 1 << 20), video);

    // With sync off, a frame that isn't here isn't available.
    engine
        .settings_set("storage.sync.enabled", json!(false))
        .unwrap();
    std::fs::remove_file(dir.join(FRAME)).unwrap();
    assert_eq!(
        code_of(control.media_open(&frame(1700))),
        ErrorCode::NotAvailable
    );
    engine.shutdown();
    fake.stop();
}

// Restore ------------------------------------------------------------------------------------

fn a_restored_meeting_reaches_the_bridge() {
    let fake = fake_s3();
    let a = harness("restore-a", json!({"meetings_retention": "keep"}));
    let engine_a = a.start();
    connect_bucket(&engine_a, &fake.endpoint());
    let code = recovery_code(&engine_a);
    let id = record(&engine_a, &a.data(), 3000);
    wait_uploaded(&engine_a);
    let snapshot_a = engine_a.meeting_control().unwrap().get(&id).unwrap();
    assert!(!snapshot_a["passages"].as_array().unwrap().is_empty());
    engine_a.shutdown();

    // Another data folder with the saved key and the same bucket.
    let b = harness("restore-b", json!({"meetings_retention": "keep"}));
    let engine_b = b.start();
    engine_b
        .settings_sync_key("import", Some(code.clone()))
        .unwrap();
    connect_bucket(&engine_b, &fake.endpoint());
    let list = engine_b.settings_sync_restore("list").unwrap();
    let meetings = list["meetings"].as_array().unwrap();
    assert_eq!(meetings.len(), 1, "{list}");
    assert_eq!(meetings[0]["id"], id.as_str());
    assert_eq!(meetings[0]["startedAt"], snapshot_a["meeting"]["startedAt"]);
    engine_b.settings_sync_restore("start").unwrap();
    wait_done(&b.data(), &id);
    let snapshot_b = engine_b.meeting_control().unwrap().get(&id).unwrap();
    for part in ["passages", "speakers"] {
        assert_eq!(snapshot_b[part], snapshot_a[part], "{part}");
    }
    assert_eq!(
        snapshot_b["meeting"]["title"],
        snapshot_a["meeting"]["title"]
    );
    for file in ["meeting.json", "timeline.jsonl"] {
        assert_eq!(
            std::fs::read(meeting_dir(&b.data(), &id).join(file)).unwrap(),
            std::fs::read(meeting_dir(&a.data(), &id).join(file)).unwrap(),
            "{file}"
        );
    }
    // The audio comes back when a link needs it.
    let control = engine_b.meeting_control().unwrap();
    let wav = read_all(
        &*control,
        &open_ready(&*control, &audio(&id, 0, Some(20_000))),
        1 << 20,
    );
    let dir_a = meeting_dir(&a.data(), &id);
    assert_eq!(wav, expected_mix(&dir_a, 0, recorded_end(&dir_a)));
    assert_eq!(
        engine_b.settings_state()["sync"]["restore"]["state"],
        "done"
    );
    engine_b.shutdown();
    fake.stop();
}

// The real screen recording --------------------------------------------------------------------

/// The screen process's synthetic screen, a new slide every second, with Microsoft's software
/// encoder for video, as in tests/screen.rs.
const SYNTHETIC_SCREEN: &str =
    "synthetic:1280x720,change-every=1000,text=Slide,no-hardware,software-encoder";

/// `harness` that also records the screen: this binary as the screen process on the synthetic
/// screen, and Voice's default screen mode `video`.
fn screen_harness(name: &str, retention: &str) -> Harness {
    let mut h = harness(
        name,
        json!({"meetings_retention": retention, "meetings_screen": "video"}),
    );
    h.screen_exe = std::env::current_exe().unwrap();
    h.screen_timing = ScreenTiming {
        poll: Duration::from_millis(100),
        start_timeout: Duration::from_secs(20),
        backoff: vec![Duration::from_millis(100), Duration::from_millis(200)],
        stable_after: Duration::from_secs(60),
        ping_failures: 3,
        handshake: Duration::from_millis(500),
        end_wait: Duration::from_secs(10),
        reconnect: Duration::from_secs(5),
    };
    h.test_screen = Some(SYNTHETIC_SCREEN.into());
    h
}

fn journal(data: &Path, id: &str) -> Vec<ScreenLine> {
    voice_screen_files::journal::read_all(&meeting_dir(data, id)).unwrap_or_default()
}

fn frames_journaled(data: &Path, id: &str) -> usize {
    journal(data, id)
        .iter()
        .filter(|l| matches!(l, ScreenLine::Frame { .. }))
        .count()
}

/// Whether a decrypted object is the screen journal: its first line is the screen process's
/// `session`, which names its clock `anchor`; the recorder's doesn't.
fn is_journal(plain: &[u8]) -> bool {
    let text = String::from_utf8_lossy(plain);
    text.lines()
        .next()
        .and_then(|l| serde_json::from_str::<Value>(l).ok())
        .is_some_and(|l| l["kind"] == "session" && l.get("anchor").is_some())
}

/// The first time the fake took an upload of an object, in wall ms.
fn first_upload(fake: &FakeS3, object: &str) -> Option<u64> {
    fake.log()
        .iter()
        .filter(|l| matches!(l.op, Op::Put | Op::UploadPart) && l.status < 300)
        .filter(|l| l.key.ends_with(object))
        .map(|l| l.at)
        .min()
}

/// Records `before` ms, pauses for `paused` ms, resumes, records `after` ms more, and stops: two
/// video files, the first ended by the pause. Waits until the meeting is done.
fn record_with_pause(engine: &Engine, data: &Path, before: u64, paused: u64, after: u64) -> String {
    let id = engine.start_meeting().unwrap();
    wait_recorded(data, &id, before);
    wait_long(
        "a frame in the screen journal",
        Duration::from_secs(30),
        || frames_journaled(data, &id) > 0,
    );
    let control = engine.meeting_control().unwrap();
    control.pause(&id).unwrap();
    std::thread::sleep(Duration::from_millis(paused));
    control.resume(&id).unwrap();
    wait_long("the second video file", Duration::from_secs(30), || {
        journal(data, &id)
            .iter()
            .any(|l| matches!(l, ScreenLine::Video { seq: 2, .. }))
    });
    std::thread::sleep(Duration::from_millis(after));
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(data, &id);
    id
}

/// Frames go up while the meeting records, after audio; video only after the timeline's stop and
/// its `video_end`; the journal after its own `stop`; the clips and `screen.json` never (contract
/// 2.1, spec RUK-110 2.8).
fn screen_files_upload_by_their_rules() {
    if !cfg!(windows) {
        println!("skipped: the screen process records on Windows only");
        return;
    }
    let h = screen_harness("sync-screen", "keep");
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    let code = recovery_code(&engine);
    let id = engine.start_meeting().unwrap();
    let dir = meeting_dir(&h.data(), &id);
    wait_recorded(&h.data(), &id, 3000);
    wait_long(
        "frames in the bucket while it records",
        Duration::from_secs(60),
        || bucket_kinds(&fake, &code).contains(&"jpeg"),
    );
    let screen_json =
        std::fs::read(dir.join("screen").join("screen.json")).expect("a live screen.json");
    let live = decrypt_all(&fake, &code);
    assert!(
        live.iter().any(|(_, p)| support::kind_of(p) == "audio"),
        "audio goes up with the frames"
    );
    assert!(
        !live.iter().any(|(_, p)| support::kind_of(p) == "mp4"),
        "no video while the meeting records"
    );
    assert!(
        !live.iter().any(|(_, p)| is_journal(p)),
        "the journal waits for its stop"
    );
    let stopped_at = wall_ms();
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &id);
    wait_uploaded(&engine);

    let closed = voice_screen_files::media::closed_files(&dir).unwrap();
    let objects = decrypt_all(&fake, &code);
    let object_of = |rel: &str| {
        let bytes = std::fs::read(dir.join(rel)).unwrap();
        objects
            .iter()
            .find(|(_, plain)| *plain == bytes)
            .map(|(name, _)| name.clone())
    };
    let mut frames_before_stop = 0;
    let mut videos = 0;
    for (rel, kind) in &closed {
        let object = object_of(rel).unwrap_or_else(|| panic!("{rel} isn't in the bucket"));
        let first = first_upload(&fake, &object).expect("an upload");
        match kind {
            voice_screen_files::media::ScreenFileKind::Video => {
                videos += 1;
                assert!(first >= stopped_at, "{rel} went up before the stop");
            }
            voice_screen_files::media::ScreenFileKind::Frame if first < stopped_at => {
                frames_before_stop += 1
            }
            _ => {}
        }
    }
    assert!(videos >= 1, "the meeting recorded video: {closed:?}");
    assert!(frames_before_stop >= 1);
    assert!(
        closed
            .iter()
            .any(|(_, k)| *k == voice_screen_files::media::ScreenFileKind::Journal),
        "the journal is closed after the stop"
    );
    assert_eq!(objects.iter().filter(|(_, p)| is_journal(p)).count(), 1);
    assert!(!dir.join("screen").join("screen.json").exists());
    assert!(
        !objects.iter().any(|(_, p)| *p == screen_json),
        "screen.json never goes up"
    );

    // A clip cut for Rukoo stays on this computer.
    let control = engine.meeting_control().unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut clip = control.clip(&id, 0, 2000, deadline).unwrap();
    wait_long("a cut clip", Duration::from_secs(60), || {
        if clip["clips"].as_array().is_some_and(|c| !c.is_empty()) {
            return true;
        }
        clip = control
            .clip(&id, 0, 2000, Instant::now() + Duration::from_secs(4))
            .unwrap();
        false
    });
    let clip_path = PathBuf::from(clip["clips"][0]["path"].as_str().unwrap());
    let clip_bytes = std::fs::read(&clip_path).unwrap();
    engine
        .settings_set("storage.sync.enabled", json!(true))
        .unwrap();
    std::thread::sleep(Duration::from_secs(3));
    wait_uploaded(&engine);
    assert!(
        !decrypt_all(&fake, &code)
            .iter()
            .any(|(_, p)| *p == clip_bytes),
        "a clip never goes up"
    );
    engine.shutdown();
    fake.stop();
}

/// A frame and a stretch of video through `media.open` and `media.read`, from this computer and
/// again after retention removed them and sync fetched them back. A range across a pause gives
/// one stretch, with the shorter end (contract 2.3).
fn media_open_serves_a_frame_and_a_clip() {
    if !cfg!(windows) {
        println!("skipped: the screen process records on Windows only");
        return;
    }
    let h = screen_harness("media-screen", "after_transcription");
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    // Nothing reaches the bucket yet, so retention holds the media while the test reads it.
    fake.faults().refuse = true;
    let id = record_with_pause(&engine, &h.data(), 3000, 1500, 2500);
    let dir = meeting_dir(&h.data(), &id);
    let lines = journal(&h.data(), &id);
    let pause_end = lines
        .iter()
        .find_map(|l| match l {
            ScreenLine::VideoEnd {
                seq: 1, t, reason, ..
            } => {
                assert_eq!(reason, "pause");
                Some(*t)
            }
            _ => None,
        })
        .expect("the first video file ends at the pause");
    let stop = lines
        .iter()
        .find_map(|l| match l {
            ScreenLine::Stop { t, .. } => Some(*t),
            _ => None,
        })
        .unwrap();
    let frame = lines
        .iter()
        .find_map(|l| match l {
            ScreenLine::Frame { t, file, .. } => Some((*t, file.clone())),
            _ => None,
        })
        .unwrap();
    let jpeg = std::fs::read(dir.join(&frame.1)).unwrap();
    let control = engine.meeting_control().unwrap();
    let frame_args = MediaOpenArgs {
        kind: MediaKind::Frame,
        ..audio(&id, frame.0 + 10, None)
    };
    let video_args = MediaOpenArgs {
        kind: MediaKind::Video,
        ..audio(&id, 0, Some(stop))
    };

    // From this computer: the frame at once, the video once it is cut.
    let ready = control.media_open(&frame_args).unwrap();
    assert_eq!(ready["ready"], true, "{ready}");
    assert_eq!(
        (&ready["type"], &ready["start"]),
        (&json!("image/jpeg"), &json!(frame.0))
    );
    assert_eq!(read_all(&*control, &ready, 1 << 20), jpeg);
    let video = open_ready(&*control, &video_args);
    assert_eq!(video["type"], "video/mp4");
    let end = video["end"].as_u64().unwrap();
    assert!(
        end < stop && end <= pause_end + 100,
        "one stretch, ending at the pause ({pause_end}), not at the stop ({stop}): {video}"
    );
    let clip = read_all(&*control, &video, 1 << 20);
    assert_eq!(&clip[4..8], b"ftyp");

    // Retention removes the media once it is in the bucket; a link fetches it back.
    fake.faults().refuse = false;
    wait_uploaded(&engine);
    let frames_dir = dir.join("screen").join("frames");
    let video_dir = dir.join("screen").join("video");
    wait_long(
        "retention to remove the screen media",
        Duration::from_secs(60),
        || !frames_dir.exists() && !video_dir.exists(),
    );
    assert!(
        dir.join("screen").join("journal.jsonl").is_file(),
        "the journal stays"
    );
    let back = open_ready(&*control, &frame_args);
    assert_eq!(
        read_all(&*control, &back, 7_001),
        jpeg,
        "the frame came back from the bucket"
    );
    let again = open_ready(&*control, &video_args);
    assert_eq!(again["end"], video["end"], "the same stretch");
    assert_eq!(
        read_all(&*control, &again, 1 << 20),
        clip,
        "the same clip from the fetched video"
    );
    engine.shutdown();
    fake.stop();
}

/// A meeting whose video hasn't reached the bucket keeps `screen/video`, with its frames and audio,
/// past its retention; once it uploaded, the sweep removes them and keeps the journal (contract 2.6).
fn retention_holds_screen_media() {
    if !cfg!(windows) {
        println!("skipped: the screen process records on Windows only");
        return;
    }
    let h = screen_harness("retention-screen", "after_transcription");
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    let code = recovery_code(&engine);
    let id = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &id, 3000);
    wait_long("frames in the bucket", Duration::from_secs(60), || {
        bucket_kinds(&fake, &code).contains(&"jpeg")
    });
    // The bucket stops taking uploads before the video can go up.
    fake.faults().refuse = true;
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &id);
    let dir = meeting_dir(&h.data(), &id);
    // The engine sweeps right after it reads `done`, within a second or two.
    std::thread::sleep(Duration::from_millis(3000));
    let video = dir.join("screen").join("video");
    assert!(video.is_dir(), "video that isn't in the bucket stays");
    assert!(dir.join("screen").join("frames").is_dir());
    assert!(dir.join("audio").is_dir());
    assert!(!bucket_kinds(&fake, &code).contains(&"mp4"));

    fake.faults().refuse = false;
    wait_long("the uploaded media to go", Duration::from_secs(60), || {
        !video.exists()
            && !dir.join("screen").join("frames").exists()
            && !dir.join("audio").exists()
    });
    assert!(bucket_kinds(&fake, &code).contains(&"mp4"));
    assert!(dir.join("screen").join("journal.jsonl").is_file());
    engine.shutdown();
    fake.stop();
}

/// Speaker ids and labels come back with a restore: data folder A's `transcript.db` is seeded with
/// three call speakers, `speakers.mode = nemotron` and passages that name them, so no diarizer is
/// needed; a fresh data folder restores the meeting and sends the same speakers and passages
/// (contract 2.5).
fn a_restored_meeting_keeps_its_speakers() {
    let fake = fake_s3();
    let a = harness("speakers-a", json!({"meetings_retention": "keep"}));
    let engine_a = a.start();
    connect_bucket(&engine_a, &fake.endpoint());
    let code = recovery_code(&engine_a);
    let id = record(&engine_a, &a.data(), 3000);
    wait_uploaded(&engine_a);
    // The transcript snapshots in the bucket, by object name: a changed one gets a new name.
    let transcripts = |fake: &FakeS3| -> Vec<String> {
        decrypt_all(fake, &code)
            .into_iter()
            .filter(|(_, plain)| support::kind_of(plain) == "transcript")
            .map(|(name, _)| name)
            .collect()
    };
    let before = transcripts(&fake);
    engine_a.shutdown();

    // Labels as a Nemotron meeting has them: call-1 to call-3, each call passage naming one.
    let dir_a = meeting_dir(&a.data(), &id);
    let seeded = {
        let t = Transcript::open(&dir_a).unwrap();
        for n in 1..=3 {
            t.save_speaker(&Speaker {
                id: format!("call-{n}"),
                track: Track::Call,
                label: format!("Speaker {n}"),
            })
            .unwrap();
        }
        t.meta_set(voice_meetings::transcript::MODE_KEY, "nemotron")
            .unwrap();
        let mut n = 0;
        for stored in t.passages().unwrap() {
            let mut p = stored.passage;
            if p.track == Track::Call {
                p.speaker = Some(format!("call-{}", n % 3 + 1));
                p.revision += 1;
                n += 1;
                t.update_passage(&p).unwrap();
            }
        }
        assert!(n >= 1, "the meeting has call passages");
        t.passages()
            .unwrap()
            .into_iter()
            .map(|s| (s.passage.id, s.passage.speaker))
            .collect::<Vec<_>>()
    };
    // A starts again, sees the changed transcript and uploads its new snapshot.
    let engine_a = a.start();
    wait_long(
        "the new transcript snapshot",
        Duration::from_secs(60),
        || transcripts(&fake).iter().any(|name| !before.contains(name)),
    );
    wait_uploaded(&engine_a);
    engine_a.shutdown();

    let b = harness("speakers-b", json!({"meetings_retention": "keep"}));
    let engine_b = b.start();
    engine_b
        .settings_sync_key("import", Some(code.clone()))
        .unwrap();
    connect_bucket(&engine_b, &fake.endpoint());
    engine_b.settings_sync_restore("start").unwrap();
    wait_done(&b.data(), &id);
    let snapshot = engine_b.meeting_control().unwrap().get(&id).unwrap();
    let mut speakers: Vec<(String, String)> = snapshot["speakers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["id"].as_str().unwrap().to_string(),
                s["label"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    speakers.sort();
    assert_eq!(
        speakers
            .iter()
            .filter(|(id, _)| id.starts_with("call-"))
            .cloned()
            .collect::<Vec<_>>(),
        (1..=3)
            .map(|n| (format!("call-{n}"), format!("Speaker {n}")))
            .collect::<Vec<_>>(),
        "{snapshot}"
    );
    assert!(speakers.iter().any(|(id, _)| id == "mic"));
    let mut restored: Vec<(String, Option<String>)> = snapshot["passages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["id"].as_str().unwrap().to_string(),
                p["speaker"].as_str().map(str::to_string),
            )
        })
        .collect();
    restored.sort();
    let mut seeded = seeded;
    seeded.sort();
    assert_eq!(restored, seeded, "every passage keeps its speaker");
    engine_b.shutdown();
    fake.stop();
}

/// Voice refuses to show or import the recovery code while a meeting records the screen, and Settings
/// says so in `sync.screenRecording` (spec RUK-110 D18, contract 2.7): by the screen supervisor's
/// mode for the meeting that records, not by Voice's default.
fn the_key_dialogs_wait_while_the_screen_records() {
    if !cfg!(windows) {
        println!("skipped: the screen process records on Windows only");
        return;
    }
    let h = screen_harness("key-screen", "keep");
    let fake = fake_s3();
    let engine = h.start();
    connect_bucket(&engine, &fake.endpoint());
    assert_eq!(sync_state(&engine)["screenRecording"], false);
    let code = recovery_code(&engine);

    let id = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &id, 1000);
    wait_for("Settings to say the screen records", || {
        sync_state(&engine)["screenRecording"] == true
    });
    for (action, code) in [("show", None), ("import", Some(code.clone()))] {
        let refused = engine.settings_sync_key(action, code).unwrap_err();
        assert_eq!(
            refused.code, "recording_screen",
            "{action}: {}",
            refused.message
        );
    }
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &id);
    wait_for("Settings to say the screen stopped", || {
        sync_state(&engine)["screenRecording"] == false
    });
    assert_eq!(
        recovery_code(&engine),
        code,
        "the code shows once the meeting stopped"
    );

    // A meeting that records no screen leaves the dialogs alone, whatever Voice's default.
    let control = engine.meeting_control().unwrap();
    let audio_only = control
        .start(voice_protocol::StartArgs {
            screen: Some(voice_protocol::ScreenMode::Off),
            ..Default::default()
        })
        .unwrap();
    wait_recorded(&h.data(), &audio_only, 1000);
    assert_eq!(sync_state(&engine)["screenRecording"], false);
    assert_eq!(recovery_code(&engine), code);
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &audio_only);
    engine.shutdown();
    fake.stop();
}
