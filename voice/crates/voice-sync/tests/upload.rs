//! Uploads through the worker against the fake bucket (spec 2.8, 2.9; acceptance criteria 2, 3
//! and 5). Platform-neutral: screen files come from a test `ScreenFiles`.

mod harness;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use harness::*;
use voice_s3_fake::Op;
use voice_sync::{names, SyncState};

const WAIT: Duration = Duration::from_secs(30);

#[test]
fn audio_and_frames_go_up_while_live_video_after() {
    let fake = fake();
    let data = scratch("upload-live");
    let _c = Cleanup(data.clone());
    let voice = Voice::with(
        &data,
        Setup {
            screen: Arc::new(TestScreen),
            ..Setup::default()
        },
    );
    let m = Meeting::new(&voice.meetings(), "Live", 0);
    let mic1 = m.segment("mic", 1, 0, 16_000);
    let call1 = m.segment("call", 1, 2, 16_000);
    let mic2 = m.open_segment("mic", 2, 1_000, 8_000);
    write(&m, "screen/frames/f-000000500.jpg", &jpeg(1));
    write(&m, "screen/frames/f-000000900.jpg", &jpeg(2));
    write(&m, "screen/video/000001.mp4", &mp4(40_000));
    write(&m, "screen/video/000001.mp4.closed", b"");
    voice.connect(&fake.endpoint());
    let key_wait = wait_for(WAIT, || voice.sync.status().key.exists);
    assert!(key_wait);
    let key = voice.key();

    let live_files = [
        "meeting.json",
        mic1.as_str(),
        call1.as_str(),
        "screen/frames/f-000000500.jpg",
        "screen/frames/f-000000900.jpg",
    ];
    let ok = wait_for(WAIT, || {
        uploaded_files(&fake, &key, &m, &live_files).len() == live_files.len()
    });
    assert!(
        ok,
        "audio, frames and meeting.json go up during the meeting: {:?}",
        fake.log()
    );
    assert_eq!(voice.sync.status().state, SyncState::Live);
    // Give the worker time to send anything it would wrongly send.
    std::thread::sleep(Duration::from_millis(400));
    let early = uploaded_files(
        &fake,
        &key,
        &m,
        &[mic2.as_str(), "screen/video/000001.mp4", "timeline.jsonl"],
    );
    assert!(early.is_empty(), "went up while live: {early:?}");
    let live_manifest = manifest(&fake, &key, &m.id).expect("a manifest during the meeting");
    assert_eq!(live_manifest.state, "live");
    assert!(
        fake.log()
            .iter()
            .all(|l| l.op != Op::Put || !l.key.contains("/index/")),
        "the index waits for the meeting's end"
    );

    // The segment closes and the meeting stops: the rest goes up, then the manifest and the index.
    m.line(
        serde_json::json!({"kind":"segment_end","t":1_500,"track":"mic","seq":2,"samples":8_000}),
    );
    m.stop(1_600);
    voice.sync.nudge();
    voice.settle(&fake, WAIT);
    let all = [
        "meeting.json",
        mic1.as_str(),
        call1.as_str(),
        mic2.as_str(),
        "screen/frames/f-000000500.jpg",
        "screen/frames/f-000000900.jpg",
        "screen/video/000001.mp4",
        "timeline.jsonl",
    ];
    assert_eq!(uploaded_files(&fake, &key, &m, &all).len(), all.len());
    let final_manifest = manifest(&fake, &key, &m.id).unwrap();
    assert_eq!(final_manifest.state, "processing");
    assert!(final_manifest.gen > live_manifest.gen);
    let mut listed: Vec<&str> = final_manifest
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    listed.sort_unstable();
    let mut want = all.to_vec();
    want.sort_unstable();
    assert_eq!(listed, want);
    let audio = final_manifest
        .files
        .iter()
        .find(|f| f.path == mic2)
        .unwrap();
    assert_eq!(
        (
            audio.kind.as_str(),
            audio.track.as_deref(),
            audio.seq,
            audio.t,
            audio.samples
        ),
        ("audio", Some("mic"), Some(2), Some(1_000), Some(8_000))
    );
    let index = indexes(&fake, &key);
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].meetings.len(), 1);
    assert_eq!(index[0].meetings[0].id, m.id);
    assert_eq!(index[0].meetings[0].title, "Live");
    assert_eq!(index[0].meetings[0].gen, final_manifest.gen);
    // The order in the log: the video after the stop, after the audio and frames.
    let log = fake.log();
    let pos = |object: &str| {
        log.iter()
            .position(|l| unprefixed(&l.key) == object && l.status == 200)
            .unwrap()
    };
    let video = file_object(
        &key,
        &m.id,
        "screen/video/000001.mp4",
        &m.read("screen/video/000001.mp4"),
    );
    let frame = file_object(
        &key,
        &m.id,
        "screen/frames/f-000000500.jpg",
        &m.read("screen/frames/f-000000500.jpg"),
    );
    let first_audio = file_object(&key, &m.id, &mic1, &m.read(&mic1));
    assert!(pos(&first_audio) < pos(&frame), "audio before frames");
    assert!(pos(&frame) < pos(&video));
}

/// Whether `hay` holds `needle` anywhere.
fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// `RIFF` and 4 bytes and `WAVEfmt `.
fn wav_signature(bytes: &[u8]) -> bool {
    bytes
        .windows(16)
        .any(|w| &w[0..4] == b"RIFF" && &w[8..16] == b"WAVEfmt ")
}

/// A 4-byte box size, `ftyp` and four printable ASCII bytes. Any four bytes count as the size, as
/// the spec and `check-encrypted.mjs` have it.
fn mp4_signature(bytes: &[u8]) -> bool {
    bytes
        .windows(12)
        .any(|w| &w[4..8] == b"ftyp" && w[8..12].iter().all(|b| (0x20..0x7f).contains(b)))
}

#[test]
fn nothing_reaches_the_bucket_unencrypted() {
    let fake = fake();
    let data = scratch("upload-encrypted");
    let _c = Cleanup(data.clone());
    let voice = Voice::with(
        &data,
        Setup {
            screen: Arc::new(TestScreen),
            ..Setup::default()
        },
    );
    let title = "Weekly with Anna about the budget";
    let m = Meeting::new(&voice.meetings(), title, 60_000);
    let segments = [
        m.segment("mic", 1, 0, 48_000),
        m.segment("call", 1, 0, 48_000),
        m.segment("mic", 2, 3_000, 32_000),
    ];
    write(&m, "screen/frames/f-000001000.jpg", &jpeg(7));
    write(&m, "screen/video/000001.mp4", &mp4(100_000));
    write(&m, "screen/video/000001.mp4.closed", b"");
    write(
        &m,
        "screen/journal.jsonl",
        b"{\"kind\":\"start\",\"t\":0}\n{\"kind\":\"stop\",\"t\":5000}\n",
    );
    m.stop(5_000);
    m.transcript(
        &[
            ("mic", 0, 2_000, "Anna talks about the budget"),
            ("call", 2_000, 4_000, "Fine"),
        ],
        now_ms(),
    );
    voice.connect(&fake.endpoint());
    voice.settle(&fake, WAIT);
    let key = voice.key();

    let objects = objects(&fake);
    assert!(objects.len() >= 10, "{} objects", objects.len());
    let raw_jfif = [
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00,
    ];
    let mut kinds = HashSet::new();
    let mut snapshot = None;
    for (object, bytes) in &objects {
        assert!(bytes.starts_with(b"RKV1"), "{object} starts with RKV1");
        assert!(!wav_signature(bytes), "{object} holds a WAV header");
        assert!(!contains(bytes, &raw_jfif), "{object} holds a JPEG header");
        assert!(!mp4_signature(bytes), "{object} holds an MP4 box");
        assert!(
            !contains(bytes, b"SQLite format 3\0"),
            "{object} holds SQLite"
        );
        assert!(
            !contains(bytes, title.as_bytes()),
            "{object} holds the title"
        );
        assert!(
            !contains(bytes, m.id.as_bytes()),
            "{object} holds the meeting id"
        );
        for word in ["m-20", ".wav", ".jpg", ".mp4", title] {
            assert!(!object.contains(word), "the key {object} shows {word}");
        }
        if bytes.len() > 4096 {
            let mut counts = [0usize; 256];
            for b in bytes {
                counts[usize::from(*b)] += 1;
            }
            let max = counts.iter().max().unwrap();
            assert!(
                (*max as f64) <= bytes.len() as f64 * 0.015,
                "{object}: one byte value makes up {max} of {} bytes",
                bytes.len()
            );
        }
        // Every object opens with the key and is a file, a snapshot, a manifest or the index.
        let plain = decrypt(&key, object, bytes);
        let kind: String = if object.ends_with("/manifest") {
            serde_json::from_slice::<voice_sync::manifest::Manifest>(&plain).unwrap();
            "manifest".into()
        } else if object.contains("/index/") {
            serde_json::from_slice::<voice_sync::manifest::Index>(&plain).unwrap();
            "index".into()
        } else if plain.starts_with(b"SQLite format 3\0") {
            snapshot = Some(plain.clone());
            "transcript".into()
        } else {
            let rels = [
                "meeting.json",
                "timeline.jsonl",
                "screen/frames/f-000001000.jpg",
                "screen/video/000001.mp4",
                "screen/journal.jsonl",
                segments[0].as_str(),
                segments[1].as_str(),
                segments[2].as_str(),
            ];
            let rel = rels
                .iter()
                .find(|rel| m.read(rel) == plain)
                .unwrap_or_else(|| panic!("{object} opens to no file of the meeting"));
            assert_eq!(*object, file_object(&key, &m.id, rel, &plain));
            (*rel).to_owned()
        };
        kinds.insert(kind);
    }
    for want in [
        "meeting.json",
        "timeline.jsonl",
        "transcript",
        "manifest",
        "index",
        "screen/frames/f-000001000.jpg",
        "screen/video/000001.mp4",
        "screen/journal.jsonl",
    ] {
        assert!(kinds.contains(want), "{want} reached the bucket");
    }
    // The snapshot has the passages.
    let plain = snapshot.expect("a transcript snapshot");
    let probe = scratch("upload-snapshot");
    let _p = Cleanup(probe.clone());
    std::fs::write(probe.join("transcript.db"), plain).unwrap();
    let texts: Vec<String> = Meeting::passages(&probe).into_iter().map(|p| p.2).collect();
    assert_eq!(texts, ["Anna talks about the budget", "Fine"]);
    // Uploads carry no metadata about their content.
    assert!(voice.sync.status().last_upload.is_some());
}

#[test]
fn the_live_lane_keeps_its_rate() {
    let fake = fake();
    let data = scratch("upload-rate");
    let _c = Cleanup(data.clone());
    let rate = 64 * 1024;
    let voice = Voice::with(
        &data,
        Setup {
            timing: voice_sync::SyncTiming {
                live_rate: rate,
                ..timing()
            },
            ..Setup::default()
        },
    );
    let m = Meeting::new(&voice.meetings(), "Rate", 0);
    let mut segments = Vec::new();
    for seq in 1..=20 {
        segments.push(m.segment("mic", seq, (seq - 1) * 500, 8_000));
    }
    voice.connect(&fake.endpoint());
    let ok = wait_for(WAIT, || voice.sync.status().key.exists);
    assert!(ok);
    let key = voice.key();
    let objects: HashSet<String> = segments
        .iter()
        .map(|s| file_object(&key, &m.id, s, &m.read(s)))
        .collect();
    let ok = wait_for(WAIT, || {
        fake.log()
            .iter()
            .filter(|l| l.status == 200 && objects.contains(unprefixed(&l.key)))
            .count()
            == 20
    });
    assert!(ok, "all 20 segments went up");
    let puts: Vec<_> = fake
        .log()
        .into_iter()
        .filter(|l| l.status == 200 && objects.contains(unprefixed(&l.key)))
        .collect();
    let bytes: u64 = puts.iter().map(|l| l.bytes).sum();
    let last = puts.last().unwrap();
    let elapsed = last.at - puts[0].at;
    let least = (bytes - last.bytes) * 1000 / rate;
    assert!(
        elapsed + 50 >= least,
        "{bytes} bytes at {rate} B/s took {elapsed} ms, at least {least} ms expected"
    );
}

#[test]
fn a_failing_bucket_backs_off_and_recovers() {
    let fake = fake();
    let data = scratch("upload-backoff");
    let _c = Cleanup(data.clone());
    let voice = Voice::open(&data);
    let m = Meeting::new(&voice.meetings(), "Backoff", 60_000);
    m.segment("mic", 1, 0, 16_000);
    m.stop(1_000);
    m.transcript(&[("mic", 0, 900, "hello")], now_ms());
    {
        let mut faults = fake.faults();
        for _ in 0..5 {
            faults.fail.push((Op::Put, 500, "InternalError"));
        }
    }
    voice.connect(&fake.endpoint());
    let ok = wait_for(WAIT, || {
        fake.log().iter().filter(|l| l.status == 500).count() >= 5
    });
    assert!(ok, "five failures: {:?}", fake.log());
    // Then the bucket refuses connections for a while, then it is healthy again.
    fake.faults().refuse = true;
    let status = voice.sync.status();
    assert_eq!(status.state, SyncState::Retrying);
    let error = status.last_error.expect("the last error");
    assert!(
        matches!(error.code.as_str(), "failed" | "network"),
        "{error:?}"
    );
    assert!(status.failing_since.is_some());
    std::thread::sleep(Duration::from_millis(400));
    let status = voice.sync.status();
    assert_eq!(status.state, SyncState::Retrying);
    assert_eq!(
        status.last_error.map(|e| e.code).as_deref(),
        Some("network")
    );
    fake.faults().refuse = false;
    voice.settle(&fake, WAIT);
    let status = voice.sync.status();
    assert_eq!(status.state, SyncState::Idle);
    assert!(status.last_error.is_none());
    assert!(status.failing_since.is_none());

    let log = fake.log();
    // Attempts of one object follow the backoff: each wait at least 0.8 × min(200, 20 · 2^n) ms.
    let mut by_key: HashMap<&str, Vec<(u64, u16)>> = HashMap::new();
    for l in log.iter().filter(|l| l.op == Op::Put) {
        by_key
            .entry(l.key.as_str())
            .or_default()
            .push((l.at, l.status));
    }
    for (key, attempts) in &by_key {
        let failed: Vec<u64> = attempts
            .iter()
            .filter(|a| a.1 == 500)
            .map(|a| a.0)
            .collect();
        for (n, pair) in failed.windows(2).enumerate() {
            let want = (20u64 << n).min(200) * 8 / 10;
            assert!(
                pair[1] - pair[0] + 5 >= want,
                "{key}: attempt {} came {} ms after the one before, want {want}",
                n + 2,
                pair[1] - pair[0]
            );
        }
        // Nothing went up twice.
        assert!(
            attempts.iter().filter(|a| a.1 == 200).count() <= 1
                || key.ends_with("/manifest")
                || key.contains("/index/"),
            "{key} went up twice"
        );
    }
}

#[test]
fn auth_errors_stop_until_the_config_changes() {
    let fake = fake();
    let data = scratch("upload-auth");
    let _c = Cleanup(data.clone());
    let voice = Voice::open(&data);
    let m = Meeting::new(&voice.meetings(), "Auth", 60_000);
    m.segment("mic", 1, 0, 16_000);
    m.stop(1_000);
    voice.sync.save_secret(SECRET).unwrap();
    let mut wrong = config(&fake.endpoint());
    wrong.access_key_id = "someone-else".into();
    voice.sync.configure(Some(wrong));
    let ok = wait_for(WAIT, || voice.sync.status().state == SyncState::Failing);
    assert!(ok, "{:?}", voice.sync.status());
    let status = voice.sync.status();
    assert_eq!(status.last_error.as_ref().unwrap().code, "auth");
    let tried = fake.log().len();
    assert!(tried >= 1);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(fake.log().len(), tried, "nothing else is tried");
    assert_eq!(voice.sync.status().state, SyncState::Failing);

    voice.sync.configure(Some(config(&fake.endpoint())));
    voice.settle(&fake, WAIT);
    assert!(voice.sync.status().last_error.is_none());
}

#[test]
fn a_clock_set_right_after_a_clock_error_lets_uploads_go_on() {
    const DAY: u64 = 86_400_000;
    let fake = fake();
    let data = scratch("upload-clock");
    let _c = Cleanup(data.clone());
    // This computer's clock runs two days ahead.
    let ahead = Arc::new(AtomicU64::new(2 * DAY));
    let offset = ahead.clone();
    let voice = Voice::with(
        &data,
        Setup {
            timing: voice_sync::SyncTiming {
                blocked: Duration::from_millis(500),
                ..timing()
            },
            now: Arc::new(move || now_ms() + offset.load(Ordering::SeqCst)),
            ..Setup::default()
        },
    );
    let m = Meeting::new(&voice.meetings(), "Clock", 60_000);
    m.segment("mic", 1, 0, 16_000);
    m.stop(1_000);
    m.transcript(&[("mic", 0, 900, "hello")], now_ms());
    fake.faults()
        .fail
        .push((Op::Put, 403, "RequestTimeTooSkewed"));
    voice.connect(&fake.endpoint());
    let ok = wait_for(WAIT, || voice.sync.status().state == SyncState::Failing);
    assert!(ok, "{:?}", voice.sync.status());
    let status = voice.sync.status();
    assert_eq!(status.last_error.as_ref().unwrap().code, "clock");
    assert!(
        status.failing_since.unwrap() > now_ms() + DAY,
        "stamped two days ahead"
    );

    // The user sets the clock right. The stop ends after its own time, not two days later, and a
    // failure now restarts the "failing since" the clock ran ahead for.
    ahead.store(0, Ordering::SeqCst);
    fake.faults().fail.push((Op::Put, 403, "AccessDenied"));
    let ok = wait_for(WAIT, || {
        voice
            .sync
            .status()
            .last_error
            .is_some_and(|e| e.code == "permission")
    });
    assert!(ok, "{:?}", voice.sync.status());
    assert!(voice.sync.status().failing_since.unwrap() <= now_ms());
    voice.settle(&fake, WAIT);
    let status = voice.sync.status();
    assert_eq!(status.state, SyncState::Idle);
    assert!(status.failing_since.is_none());
}

#[test]
fn a_passed_test_ends_a_stop() {
    let fake = fake();
    let data = scratch("upload-unblock");
    let _c = Cleanup(data.clone());
    let voice = Voice::open(&data);
    let m = Meeting::new(&voice.meetings(), "Unblock", 60_000);
    m.segment("mic", 1, 0, 16_000);
    m.stop(1_000);
    fake.faults()
        .fail
        .push((Op::Put, 403, "RequestTimeTooSkewed"));
    voice.connect(&fake.endpoint());
    let ok = wait_for(WAIT, || voice.sync.status().state == SyncState::Failing);
    assert!(ok, "{:?}", voice.sync.status());
    // Uploads would wait 15 minutes; the clock error asks the user to set the clock and test.
    voice.sync.test(&config(&fake.endpoint())).unwrap();
    voice.settle(&fake, WAIT);
    assert_eq!(voice.sync.status().state, SyncState::Idle);
}

/// A wall clock an hour ahead until the test sets it right.
fn fast_clock() -> (Arc<AtomicU64>, Arc<dyn Fn() -> u64 + Send + Sync>) {
    let ahead = Arc::new(AtomicU64::new(3_600_000));
    let offset = ahead.clone();
    (
        ahead,
        Arc::new(move || now_ms() + offset.load(Ordering::SeqCst)),
    )
}

/// Whether the meeting's manifest in the bucket lists `path`.
fn names_file(fake: &voice_s3_fake::FakeS3, voice: &Voice, meeting: &str, path: &str) -> bool {
    let keys = voice_sync::keys::KeyStore::new(
        &voice.data.join(voice_sync::SYNC_DIR),
        Arc::new(voice_sync::protect::TestProtector),
    );
    let Ok(Some(key)) = keys.load() else {
        return false;
    };
    manifest(fake, &key, meeting).is_some_and(|mf| mf.files.iter().any(|f| f.path == path))
}

#[test]
fn a_live_manifest_follows_a_fast_clock_set_right() {
    let fake = fake();
    let data = scratch("upload-live-clock");
    let _c = Cleanup(data.clone());
    let (ahead, now) = fast_clock();
    let voice = Voice::with(
        &data,
        Setup {
            now,
            ..Setup::default()
        },
    );
    // A meeting that records: its manifest goes up every `manifest_live` (100 ms here).
    let m = Meeting::new(&voice.meetings(), "Live clock", 60_000);
    m.segment("mic", 1, 0, 16_000);
    voice.connect(&fake.endpoint());
    let ok = wait_for(WAIT, || {
        names_file(&fake, &voice, &m.id, "audio/mic/000001.wav")
    });
    assert!(ok, "{:?}", fake.log());

    // The clock is set right; the next segment closes and goes up. Its manifest must not wait an
    // hour for the clock to reach the stamp the first one left.
    ahead.store(0, Ordering::SeqCst);
    m.segment("mic", 2, 1_000, 16_000);
    let ok = wait_for(Duration::from_secs(10), || {
        names_file(&fake, &voice, &m.id, "audio/mic/000002.wav")
    });
    assert!(ok, "no manifest names segment 2: {:?}", fake.log());
}

#[test]
fn the_index_follows_a_fast_clock_set_right() {
    let fake = fake();
    let data = scratch("upload-index-clock");
    let _c = Cleanup(data.clone());
    let (ahead, now) = fast_clock();
    let voice = Voice::with(
        &data,
        Setup {
            timing: voice_sync::SyncTiming {
                index_debounce: Duration::from_secs(2),
                ..timing()
            },
            now,
            ..Setup::default()
        },
    );
    let m = Meeting::new(&voice.meetings(), "Index clock", 60_000);
    m.segment("mic", 1, 0, 16_000);
    m.stop(1_000);
    voice.connect(&fake.endpoint());
    // The manifest goes up while the clock runs an hour ahead, so the index is due then plus 2 s.
    let ok = wait_for(WAIT, || names_file(&fake, &voice, &m.id, "timeline.jsonl"));
    assert!(ok, "{:?}", fake.log());
    assert!(!index_after_last_manifest(&fake), "the index waits its 2 s");
    ahead.store(0, Ordering::SeqCst);

    let ok = wait_for(Duration::from_secs(10), || {
        index_after_last_manifest(&fake) && voice.sync.status().state == SyncState::Idle
    });
    assert!(
        ok,
        "the index waits for the clock: {:?}",
        voice.sync.status()
    );
}

/// A file nobody else can open while this lives, as backup or antivirus software can hold one:
/// opened without sharing on Windows, its permissions taken away elsewhere.
struct Locked {
    #[cfg(unix)]
    path: std::path::PathBuf,
    #[cfg(windows)]
    _file: std::fs::File,
}

/// `None` when the system lets the test read it anyway, as it does for root.
fn lock(path: &std::path::Path) -> Option<Locked> {
    #[cfg(windows)]
    let locked = {
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap();
        Locked { _file: file }
    };
    #[cfg(unix)]
    let locked = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        Locked {
            path: path.to_owned(),
        }
    };
    if std::fs::File::open(path).is_ok() {
        return None;
    }
    Some(locked)
}

#[cfg(unix)]
impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o644));
    }
}

#[test]
fn a_segment_that_cant_be_read_keeps_its_meeting_held() {
    let fake = fake();
    let data = scratch("upload-unreadable");
    let _c = Cleanup(data.clone());
    let voice = Voice::open(&data);
    let m = Meeting::new(&voice.meetings(), "Locked", 60_000);
    m.segment("mic", 1, 0, 16_000);
    let second = m.segment("mic", 2, 1_000, 16_000);
    m.stop(2_000);
    m.transcript(&[("mic", 0, 900, "hello")], now_ms());
    let Some(locked) = lock(&m.path(&second)) else {
        println!("skipped: this system lets the test read a locked file");
        return;
    };
    voice.connect(&fake.endpoint());
    // Everything else and a manifest go up; the scans that follow still can't read segment 2.
    voice.settle(&fake, WAIT);
    std::thread::sleep(Duration::from_millis(1_000));
    assert!(!voice.sync.in_bucket(&m.id, &second));
    assert!(
        voice.sync.holds_media(&m.id, true),
        "a segment that never reached the bucket keeps the meeting's media held"
    );

    drop(locked);
    let ok = wait_for(WAIT, || {
        voice.sync.in_bucket(&m.id, &second) && !voice.sync.holds_media(&m.id, true)
    });
    assert!(ok, "{:?}", voice.sync.status());
}

#[test]
fn a_new_bucket_gets_everything_still_on_this_computer() {
    let (a, b) = (fake(), fake());
    let root = scratch("upload-move");
    let _c = Cleanup(root.clone());
    let voice = Voice::open(&root.join("first"));
    let before = Meeting::new(&voice.meetings(), "Before the move", 7_200_000);
    let segment = before.segment("mic", 1, 0, 16_000);
    before.stop(1_000);
    before.transcript(&[("mic", 0, 900, "hello")], now_ms());
    voice.connect(&a.endpoint());
    voice.settle(&a, WAIT);
    let key = voice.key();
    assert!(voice.sync.in_bucket(&before.id, &segment));
    assert!(!voice.sync.holds_media(&before.id, true));

    // The user moves to bucket B, which can't be reached yet. Nothing counts as in the bucket
    // meanwhile, so retention keeps the audio only bucket A has.
    b.faults().refuse = true;
    voice.sync.configure(Some(config(&b.endpoint())));
    assert!(!voice.sync.in_bucket(&before.id, &segment));
    assert!(voice.sync.holds_media(&before.id, true));
    let ok = wait_for(WAIT, || voice.sync.status().waiting >= 3);
    assert!(ok, "everything waits again: {:?}", voice.sync.status());
    assert!(!voice.sync.in_bucket(&before.id, &segment));
    assert!(voice.sync.holds_media(&before.id, true));
    // This computer uploaded with its key, in bucket A: it still keeps that key.
    let stranger = Voice::open(&root.join("stranger"));
    let (code, _) = stranger.sync.show_key().unwrap();
    assert_eq!(
        voice.sync.import_key(&code).unwrap_err().kind,
        voice_sync::SyncErrorKind::KeyInUse
    );

    b.faults().refuse = false;
    let after = Meeting::new(&voice.meetings(), "After the move", 60_000);
    after.segment("mic", 1, 0, 16_000);
    after.stop(1_000);
    after.transcript(&[("mic", 0, 900, "again")], now_ms());
    voice.settle(&b, WAIT);
    for m in [&before, &after] {
        let files = ["meeting.json", "audio/mic/000001.wav", "timeline.jsonl"];
        assert_eq!(
            uploaded_files(&b, &key, m, &files).len(),
            files.len(),
            "{} is in bucket B",
            m.title
        );
        let mf = manifest(&b, &key, &m.id).expect("a manifest in bucket B");
        assert!(mf.files.iter().any(|f| f.path == "transcript.db"));
    }
    let ok = wait_for(WAIT, || {
        voice.sync.in_bucket(&before.id, &segment) && !voice.sync.holds_media(&before.id, true)
    });
    assert!(ok, "the hold lets go once bucket B has it");
    let index = indexes(&b, &key);
    assert_eq!(index.len(), 1);
    assert_eq!(index[0].meetings.len(), 2);

    // Another computer with the key restores both from bucket B.
    let (code, _) = voice.sync.show_key().unwrap();
    let other = Voice::open(&root.join("second"));
    other.sync.import_key(&code).unwrap();
    other.connect(&b.endpoint());
    assert_eq!(other.sync.remote_meetings().unwrap().len(), 2);
    let (seen, on_restored) = recorder();
    other.sync.restore(Vec::new(), on_restored).unwrap();
    let ok = wait_for(WAIT, || other.sync.status().restore.state != "restoring");
    assert!(ok);
    assert_eq!(
        other.sync.status().restore.state,
        "done",
        "{:?}",
        other.sync.status().restore
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    for m in [&before, &after] {
        assert_eq!(
            std::fs::read(other.meetings().join(&m.id).join("meeting.json")).unwrap(),
            m.read("meeting.json")
        );
    }
}

#[test]
fn a_changed_file_gets_a_new_object() {
    let fake = fake();
    let data = scratch("upload-changed");
    let _c = Cleanup(data.clone());
    let voice = Voice::open(&data);
    let m = Meeting::new(&voice.meetings(), "Changed", 60_000);
    m.segment("mic", 1, 0, 16_000);
    m.stop(1_000);
    m.transcript(&[("mic", 0, 900, "hello")], now_ms());
    voice.connect(&fake.endpoint());
    voice.settle(&fake, WAIT);
    let key = voice.key();
    let old = m.read("timeline.jsonl");
    let old_object = file_object(&key, &m.id, "timeline.jsonl", &old);
    assert!(has_object(&fake, &old_object));
    let first_gen = manifest(&fake, &key, &m.id).unwrap().gen;

    m.line(serde_json::json!({"kind":"mark","t":900,"id":"k-1","label":null}));
    let new = m.read("timeline.jsonl");
    let new_object = file_object(&key, &m.id, "timeline.jsonl", &new);
    let ok = wait_for(WAIT, || {
        has_object(&fake, &new_object)
            && !has_object(&fake, &old_object)
            && manifest(&fake, &key, &m.id).is_some_and(|mf| mf.gen > first_gen)
    });
    assert!(ok, "{:?}", fake.log());
    voice.settle(&fake, WAIT);
    let mf = manifest(&fake, &key, &m.id).unwrap();
    let entry = mf
        .files
        .iter()
        .find(|f| f.path == "timeline.jsonl")
        .unwrap();
    assert_eq!(entry.object, new_object);
    assert_eq!(
        mf.files
            .iter()
            .filter(|f| f.path == "timeline.jsonl")
            .count(),
        1
    );
    // The old object went only after the manifest that names the new one.
    let log = fake.log();
    let manifest_key = manifest_object(&key, &m.id);
    let new_manifest = log
        .iter()
        .rposition(|l| unprefixed(&l.key) == manifest_key && l.status == 200 && l.op == Op::Put)
        .unwrap();
    let delete = log
        .iter()
        .position(|l| matches!(l.op, Op::Delete | Op::DeleteMany) && l.status == 200)
        .expect("a delete");
    assert!(delete > new_manifest);
}

#[test]
fn backfill_goes_newest_first() {
    let fake = fake();
    let data = scratch("upload-backfill");
    let _c = Cleanup(data.clone());
    let voice = Voice::open(&data);
    let mut meetings = Vec::new();
    for (title, ago) in [
        ("Oldest", 3 * 86_400_000),
        ("Middle", 2 * 86_400_000),
        ("Newest", 86_400_000),
    ] {
        let m = Meeting::new(&voice.meetings(), title, ago);
        m.segment("mic", 1, 0, 16_000);
        m.segment("call", 1, 0, 16_000);
        m.stop(1_000);
        m.transcript(&[("mic", 0, 900, "hello")], now_ms());
        meetings.push(m);
    }
    voice.connect(&fake.endpoint());
    voice.settle(&fake, WAIT);
    let key = voice.key();
    let name_key = key.name_key();
    let folder = |m: &Meeting| {
        format!(
            "rkv1/{}/m/{}/o/",
            key.kid(),
            names::meeting_hash(&name_key, &m.id)
        )
    };
    let log = fake.log();
    let span = |m: &Meeting| {
        let f = folder(m);
        let at: Vec<usize> = log
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                l.op == Op::Put && l.status == 200 && unprefixed(&l.key).starts_with(&f)
            })
            .map(|(i, _)| i)
            .collect();
        assert!(!at.is_empty());
        (at[0], *at.last().unwrap())
    };
    let (oldest, middle, newest) = (span(&meetings[0]), span(&meetings[1]), span(&meetings[2]));
    assert!(
        newest.1 < middle.0,
        "newest {newest:?} before middle {middle:?}"
    );
    assert!(
        middle.1 < oldest.0,
        "middle {middle:?} before oldest {oldest:?}"
    );
    let index = indexes(&fake, &key);
    assert_eq!(index[0].meetings.len(), 3);
}

/// The screen folder through its real journal (written with `JournalWriter`, stand-in JPEG and MP4
/// bytes): a frame goes up during the meeting once its `frame` line exists, a video file only
/// after its `video_end` and the timeline's `stop`, the journal after its own `stop`, and the
/// clips, `screen.json` and `*.tmp` files never (contract 2.1).
#[test]
fn screen_files_upload_by_their_journal() {
    use voice_screen_files::journal::{JournalWriter, Line};
    let fake = fake();
    let data = scratch("upload-journal");
    let _c = Cleanup(data.clone());
    let voice = Voice::with(
        &data,
        Setup {
            screen: Arc::new(JournalScreen),
            ..Setup::default()
        },
    );
    let m = Meeting::new(&voice.meetings(), "Journal", 0);
    let mic1 = m.segment("mic", 1, 0, 16_000);
    let dir = m.path("");
    let mut journal = JournalWriter::open(&dir).unwrap();
    let frame = |t: u64| Line::Frame {
        t,
        id: format!("f-{t:09}"),
        file: format!("screen/frames/f-{t:09}.jpg"),
        width: 1920,
        height: 1080,
        text: "Slide".into(),
        ocr: "ok".into(),
        language: None,
    };
    journal
        .write(&Line::Session {
            t: 400,
            wall: now_ms(),
            boot: None,
            session: 1,
            pid: 1,
            resumed: false,
            version: 1,
            mode: "video".into(),
            anchor: "wall".into(),
            screen: "0.1.0".into(),
        })
        .unwrap();
    journal
        .write(&Line::Video {
            t: 450,
            seq: 1,
            file: "screen/video/000001.mp4".into(),
            width: 1920,
            height: 1080,
            fps: 15,
            encoder: "test".into(),
            hardware: false,
        })
        .unwrap();
    write(&m, "screen/frames/f-000000500.jpg", &jpeg(1));
    journal.write(&frame(500)).unwrap();
    // A JPEG the journal doesn't name yet (its OCR runs), the open video file, and files that
    // never leave this computer.
    write(&m, "screen/frames/f-000000900.jpg", &jpeg(2));
    write(&m, "screen/video/000001.mp4", &mp4(40_000));
    write(&m, "screen/frames/f-000001300.jpg.tmp", &jpeg(3));
    write(
        &m,
        "screen/clips/c-000000000-000001000-000001.mp4",
        &mp4(9_000),
    );
    write(
        &m,
        "screen/screen.json",
        br#"{"pid":1,"port":1,"token":"t","session":1}"#,
    );
    voice.connect(&fake.endpoint());
    assert!(wait_for(WAIT, || voice.sync.status().key.exists));
    let key = voice.key();

    let ok = wait_for(WAIT, || {
        uploaded_files(
            &fake,
            &key,
            &m,
            &["screen/frames/f-000000500.jpg", mic1.as_str()],
        )
        .len()
            == 2
    });
    assert!(
        ok,
        "a journaled frame goes up while the meeting records: {:?}",
        fake.log()
    );
    let never = [
        "screen/frames/f-000000900.jpg",
        "screen/video/000001.mp4",
        "screen/journal.jsonl",
        "screen/frames/f-000001300.jpg.tmp",
        "screen/clips/c-000000000-000001000-000001.mp4",
        "screen/screen.json",
    ];
    std::thread::sleep(Duration::from_millis(400));
    assert!(uploaded_files(&fake, &key, &m, &never).is_empty());

    // The video file closes, but the meeting still records: video waits for the timeline's stop.
    journal
        .write(&Line::VideoEnd {
            t: 1_400,
            seq: 1,
            reason: "pause".into(),
            bytes: 40_000,
        })
        .unwrap();
    journal.write(&frame(900)).unwrap();
    voice.sync.nudge();
    let ok = wait_for(WAIT, || {
        uploaded_files(&fake, &key, &m, &["screen/frames/f-000000900.jpg"]).len() == 1
    });
    assert!(ok, "the frame goes up once the journal names it");
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        uploaded_files(&fake, &key, &m, &["screen/video/000001.mp4"]).is_empty(),
        "video goes up only after the stop"
    );

    // The timeline stops: the video goes; the journal waits for its own stop.
    m.stop(1_600);
    voice.sync.nudge();
    let ok = wait_for(WAIT, || {
        uploaded_files(
            &fake,
            &key,
            &m,
            &["screen/video/000001.mp4", "timeline.jsonl"],
        )
        .len()
            == 2
    });
    assert!(
        ok,
        "the closed video goes up after the stop: {:?}",
        fake.log()
    );
    assert!(uploaded_files(&fake, &key, &m, &["screen/journal.jsonl"]).is_empty());

    journal
        .write(&Line::Stop {
            t: 1_600,
            reason: "meeting".into(),
        })
        .unwrap();
    journal.sync();
    voice.sync.nudge();
    // Wait for the journal itself. `settle` alone can return before the worker has looked at the
    // nudge: the queue is idle with nothing waiting until that scan finds the journal.
    let ok = wait_for(WAIT, || {
        uploaded_files(&fake, &key, &m, &["screen/journal.jsonl"]).len() == 1
    });
    assert!(ok, "the journal goes up after its stop: {:?}", fake.log());
    voice.settle(&fake, WAIT);
    assert!(
        uploaded_files(
            &fake,
            &key,
            &m,
            &[
                "screen/frames/f-000001300.jpg.tmp",
                "screen/clips/c-000000000-000001000-000001.mp4",
                "screen/screen.json",
            ],
        )
        .is_empty(),
        "clips, screen.json and .tmp files never go up"
    );
    let listed: Vec<String> = manifest(&fake, &key, &m.id)
        .unwrap()
        .files
        .iter()
        .map(|f| f.path.clone())
        .filter(|p| p.starts_with("screen/"))
        .collect();
    let mut listed = listed;
    listed.sort();
    assert_eq!(
        listed,
        [
            "screen/frames/f-000000500.jpg",
            "screen/frames/f-000000900.jpg",
            "screen/journal.jsonl",
            "screen/video/000001.mp4",
        ]
    );
}

/// A screen journal that can't be read keeps its meeting from counting as synced, so retention
/// holds its media until a later scan reads it.
#[test]
fn an_unreadable_screen_journal_keeps_its_meeting_held() {
    use std::sync::atomic::AtomicBool;
    struct Flaky(Arc<AtomicBool>);
    impl voice_sync::ScreenFiles for Flaky {
        fn list(
            &self,
            _dir: &std::path::Path,
            _stopped: bool,
        ) -> std::io::Result<Vec<voice_sync::ScreenFile>> {
            if self.0.load(Ordering::SeqCst) {
                Err(std::io::Error::other("the journal is locked"))
            } else {
                Ok(Vec::new())
            }
        }
    }
    let broken = Arc::new(AtomicBool::new(true));
    let fake = fake();
    let data = scratch("upload-journal-broken");
    let _c = Cleanup(data.clone());
    let voice = Voice::with(
        &data,
        Setup {
            screen: Arc::new(Flaky(Arc::clone(&broken))),
            ..Setup::default()
        },
    );
    let m = Meeting::new(&voice.meetings(), "Broken journal", 60_000);
    m.segment("mic", 1, 0, 16_000);
    m.stop(1_000);
    m.transcript(&[("mic", 0, 900, "hello")], now_ms());
    voice.connect(&fake.endpoint());
    // Everything else and a manifest go up; the scans that follow still can't read the journal.
    voice.settle(&fake, WAIT);
    std::thread::sleep(Duration::from_millis(1_000));
    assert!(
        voice.sync.holds_media(&m.id, true),
        "the meeting stays held while its screen journal can't be read"
    );

    broken.store(false, Ordering::SeqCst);
    voice.sync.nudge();
    let ok = wait_for(WAIT, || !voice.sync.holds_media(&m.id, true));
    assert!(
        ok,
        "the hold lets go once a scan reads the journal: {:?}",
        voice.sync.status()
    );
}
