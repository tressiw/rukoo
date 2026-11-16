//! What voice-sync's queue, upload, resume, restore and prune tests share: meeting folders written
//! in voice/RECORDING.md's format with stand-in audio (never real audio), a fake bucket, a
//! `BucketSync` with short timings, and decryption of what reached the bucket.
#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use voice_meetings::transcript::{Outcome, StoredPassage};
use voice_meetings::Transcript;
use voice_protocol::{passage_id, MeetingId, MeetingState, Passage, Track};
use voice_s3_fake::FakeS3;
use voice_sync::format::{cipher_len, Header, Sealer, CHUNK, HEADER_LEN, TAG};
use voice_sync::keys::{KeyStore, MasterKey};
use voice_sync::protect::TestProtector;
use voice_sync::{
    names, BucketRetention, BucketSync, ScreenFile, ScreenFileKind, ScreenFiles, Service,
    SyncConfig, SyncOptions, SyncState, SyncTiming, UrlStyle, SYNC_DIR,
};

pub const BUCKET: &str = "test";
pub const ACCESS_KEY: &str = "test";
pub const SECRET: &str = "test-secret";
pub const PREFIX: &str = "voice";

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Appends `<crate> <test>: <reason>` to the file `RUKOO_VOICE_SKIP_LOG` names. A skip prints to
/// stderr, which libtest hides for a passing test; CI's `skip-log.mjs report` step reads this file
/// back. Without the variable nothing is written.
pub fn log_skip(test: &str, reason: &str) {
    use std::io::Write;
    let Some(path) = std::env::var_os("RUKOO_VOICE_SKIP_LOG").filter(|p| !p.is_empty()) else {
        return;
    };
    // One write of the whole line, so tests running in parallel threads don't interleave.
    let line = format!("{} {test}: {reason}\n", env!("CARGO_PKG_NAME"));
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// A fresh folder under the system's temp folder.
pub fn scratch(name: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "voice-sync-{name}-{}-{}-{}",
        std::process::id(),
        now_ms(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Fast clocks for tests: scans every 50 ms, backoff from 20 ms, the index 50 ms after a manifest.
pub fn timing() -> SyncTiming {
    SyncTiming {
        scan_live: Duration::from_millis(50),
        scan_all: Duration::from_millis(300),
        live_rate: 1 << 40,
        backoff_min: Duration::from_millis(20),
        backoff_max: Duration::from_millis(200),
        blocked: Duration::from_secs(15 * 60),
        manifest_live: Duration::from_millis(100),
        index_debounce: Duration::from_millis(50),
        prune_every: Duration::from_secs(3600),
        connect_timeout: Duration::from_secs(2),
        request_timeout: Duration::from_secs(20),
        part_size: 8 << 20,
        single_max: 16 << 20,
    }
}

pub fn fake() -> FakeS3 {
    FakeS3::start(BUCKET, ACCESS_KEY, None).unwrap()
}

pub fn config(endpoint: &str) -> SyncConfig {
    SyncConfig {
        service: Service::Other,
        endpoint: endpoint.to_owned(),
        region: "us-east-1".into(),
        bucket: BUCKET.into(),
        prefix: PREFIX.into(),
        access_key_id: ACCESS_KEY.into(),
        url_style: UrlStyle::Path,
        retention: BucketRetention::Keep,
    }
}

/// A data folder with its sync.
pub struct Voice {
    pub data: PathBuf,
    pub sync: BucketSync,
}

pub struct Setup {
    pub timing: SyncTiming,
    pub screen: Arc<dyn ScreenFiles>,
    pub reserve: Arc<dyn Fn(u64) -> bool + Send + Sync>,
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl Default for Setup {
    fn default() -> Setup {
        Setup {
            timing: timing(),
            screen: Arc::new(voice_sync::NoScreen),
            reserve: Arc::new(|_| true),
            now: Arc::new(now_ms),
        }
    }
}

impl Voice {
    pub fn open(data: &Path) -> Voice {
        Voice::with(data, Setup::default())
    }

    pub fn with(data: &Path, setup: Setup) -> Voice {
        fs::create_dir_all(data.join("meetings")).unwrap();
        let sync = BucketSync::open(SyncOptions {
            data_dir: data.to_owned(),
            meetings_dir: data.join("meetings"),
            protector: Arc::new(TestProtector),
            screen: setup.screen,
            now: setup.now,
            reserve: setup.reserve,
            timing: setup.timing,
            on_change: Arc::new(|| {}),
        })
        .unwrap();
        Voice {
            data: data.to_owned(),
            sync,
        }
    }

    pub fn meetings(&self) -> PathBuf {
        self.data.join("meetings")
    }

    /// Saves the secret and turns sync on against `endpoint`.
    pub fn connect(&self, endpoint: &str) {
        self.sync.save_secret(SECRET).unwrap();
        self.sync.configure(Some(config(endpoint)));
    }

    /// The master key this data folder uses.
    pub fn key(&self) -> MasterKey {
        key_of(&self.data)
    }

    /// Waits until nothing waits, the worker is idle and the index went up after the last manifest.
    pub fn settle(&self, fake: &FakeS3, timeout: Duration) {
        let ok = wait_for(timeout, || {
            let s = self.sync.status();
            s.state == SyncState::Idle && s.waiting == 0 && index_after_last_manifest(fake)
        });
        assert!(
            ok,
            "sync didn't settle: {:?}\nlog: {:?}",
            self.sync.status(),
            fake.log()
        );
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.sync.shutdown();
    }
}

pub fn key_of(data: &Path) -> MasterKey {
    KeyStore::new(&data.join(SYNC_DIR), Arc::new(TestProtector))
        .load()
        .unwrap()
        .expect("a key")
}

/// Whether the last manifest PUT was followed by an index PUT.
pub fn index_after_last_manifest(fake: &FakeS3) -> bool {
    let log = fake.log();
    let put = |l: &&voice_s3_fake::Logged| l.op == voice_s3_fake::Op::Put && l.status == 200;
    let last_manifest = log
        .iter()
        .rposition(|l| put(&l) && l.key.ends_with("/manifest"));
    let last_index = log
        .iter()
        .rposition(|l| put(&l) && l.key.contains("/index/"));
    matches!((last_manifest, last_index), (Some(m), Some(i)) if i > m)
}

pub fn wait_for(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    loop {
        if f() {
            return true;
        }
        if Instant::now() > end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// The bucket's objects with keys without the user's prefix.
pub fn objects(fake: &FakeS3) -> Vec<(String, Vec<u8>)> {
    fake.objects()
        .into_iter()
        .map(|(k, v)| (unprefixed(&k).to_owned(), v))
        .collect()
}

pub fn unprefixed(key: &str) -> &str {
    key.strip_prefix(&format!("{PREFIX}/"))
        .unwrap_or(key)
        .trim_start_matches('/')
}

pub fn has_object(fake: &FakeS3, object: &str) -> bool {
    objects(fake).iter().any(|(k, _)| k == object)
}

/// Opens an `RKV1` object.
pub fn decrypt(key: &MasterKey, object: &str, bytes: &[u8]) -> Vec<u8> {
    assert!(bytes.len() as u64 >= HEADER_LEN, "{object} is too short");
    let header = Header::parse(&bytes[..HEADER_LEN as usize]).unwrap();
    let sealer = Sealer::open(key, object, &header).unwrap();
    let total = cipher_len(header.plain_len());
    assert_eq!(bytes.len() as u64, total, "{object} has the wrong length");
    let chunks = header.plain_len().div_ceil(CHUNK).max(1);
    let mut out = Vec::new();
    for i in 0..chunks {
        let from = HEADER_LEN + i * (CHUNK + TAG);
        let to = (from + CHUNK + TAG).min(total);
        out.extend(
            sealer
                .open_chunk(&header, i, &bytes[from as usize..to as usize])
                .unwrap(),
        );
    }
    out
}

/// Encrypts `plain` as the object `object`.
pub fn seal(key: &MasterKey, object: &str, plain: &[u8]) -> Vec<u8> {
    let (header, sealer) = Sealer::new(key, object, plain.len() as u64).unwrap();
    sealer
        .read_cipher(
            &header,
            &mut Cursor::new(plain),
            0,
            cipher_len(plain.len() as u64) as usize,
        )
        .unwrap()
}

/// The object a file of a meeting goes to.
pub fn file_object(key: &MasterKey, meeting: &str, path: &str, bytes: &[u8]) -> String {
    let name_key = key.name_key();
    let mhash = names::meeting_hash(&name_key, meeting);
    names::file_object(
        &key.kid(),
        &mhash,
        &name_key,
        meeting,
        path,
        &sha256_hex(bytes),
    )
}

pub fn manifest_object(key: &MasterKey, meeting: &str) -> String {
    names::manifest_object(&key.kid(), &names::meeting_hash(&key.name_key(), meeting))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The newest manifest of a meeting in the bucket.
pub fn manifest(
    fake: &FakeS3,
    key: &MasterKey,
    meeting: &str,
) -> Option<voice_sync::manifest::Manifest> {
    let object = manifest_object(key, meeting);
    let (_, bytes) = objects(fake).into_iter().find(|(k, _)| *k == object)?;
    Some(serde_json::from_slice(&decrypt(key, &object, &bytes)).unwrap())
}

/// Every index in the bucket.
pub fn indexes(fake: &FakeS3, key: &MasterKey) -> Vec<voice_sync::manifest::Index> {
    let prefix = names::index_prefix(&key.kid());
    objects(fake)
        .into_iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .map(|(k, v)| serde_json::from_slice(&decrypt(key, &k, &v)).unwrap())
        .collect()
}

/// A meeting folder written by hand in voice/RECORDING.md's format.
pub struct Meeting {
    pub id: String,
    pub dir: PathBuf,
    pub created_at: u64,
    pub title: String,
}

static SUFFIX: AtomicU64 = AtomicU64::new(1);

impl Meeting {
    /// A meeting that started `ago_ms` before now, with `meeting.json` and a `session` line.
    pub fn new(meetings: &Path, title: &str, ago_ms: u64) -> Meeting {
        let created_at = now_ms() - ago_ms;
        let suffix = (SUFFIX.fetch_add(1, Ordering::Relaxed) % 0xffff) as u16;
        let id = MeetingId::with_suffix(created_at, suffix).into_string();
        let dir = meetings.join(&id);
        fs::create_dir_all(&dir).unwrap();
        let file = serde_json::json!({
            "id": id, "title": title, "createdAt": created_at, "tracks": ["mic", "call"],
            "sources": { "mic": "default-input", "call": "default-output" }, "screen": "off",
            "startedBy": "voice", "context": null, "retention": "keep", "version": 1
        });
        fs::write(dir.join("meeting.json"), serde_json::to_vec(&file).unwrap()).unwrap();
        let m = Meeting {
            id,
            dir,
            created_at,
            title: title.to_owned(),
        };
        m.line(
            serde_json::json!({"kind":"session","t":0,"wall":created_at,"session":1,
            "pid":1,"resumed":false,"version":1,"recorder":"0.1.0"}),
        );
        m
    }

    pub fn line(&self, line: serde_json::Value) {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("timeline.jsonl"))
            .unwrap();
        writeln!(f, "{line}").unwrap();
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    pub fn read(&self, rel: &str) -> Vec<u8> {
        fs::read(self.path(rel)).unwrap()
    }

    /// Writes a closed segment of stand-in samples (a ramp, not audio) and its two lines.
    pub fn segment(&self, track: &str, seq: u64, t: u64, samples: u64) -> String {
        let file = self.open_segment(track, seq, t, samples);
        self.line(
            serde_json::json!({"kind":"segment_end","t":t + samples / 16,"track":track,
            "seq":seq,"samples":samples}),
        );
        file
    }

    /// Writes a segment and its `segment` line, without `segment_end`.
    pub fn open_segment(&self, track: &str, seq: u64, t: u64, samples: u64) -> String {
        let file = format!("audio/{track}/{seq:06}.wav");
        let path = self.path(&file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut w = voice_audio::SegmentWriter::create(&path).unwrap();
        let step = if track == "mic" { 7 } else { 11 };
        let pcm: Vec<i16> = (0..samples)
            .map(|i| ((i * step + seq * 13) % 4000) as i16 - 2000)
            .collect();
        w.append(&pcm).unwrap();
        w.close().unwrap();
        self.line(serde_json::json!({"kind":"segment","t":t,"track":track,"seq":seq,"file":file}));
        file
    }

    pub fn stop(&self, t: u64) {
        self.line(
            serde_json::json!({"kind":"stop","t":t,"wall":self.created_at + t,"reason":"user"}),
        );
    }

    /// A `transcript.db` with one passage per entry and the outcome `done` at `done_at`.
    pub fn transcript(&self, passages: &[(&str, u64, u64, &str)], done_at: u64) {
        let mut t = Transcript::open(&self.dir).unwrap();
        for (track, start, end, text) in passages {
            let track = Track::parse(track).unwrap();
            let cursor = t.cursor(track).unwrap();
            let passage = Passage {
                id: passage_id(track.as_str(), *start),
                track,
                speaker: Some(track.as_str().to_owned()),
                start: *start,
                end: *end,
                text: (*text).to_owned(),
                revision: 1,
                dictation: false,
                language: Some("en".into()),
            };
            t.commit_window(
                track,
                cursor,
                *end * 16,
                "silence",
                vec![StoredPassage {
                    passage,
                    tokens: Vec::new(),
                }],
            )
            .unwrap()
            .expect("the window is stored");
        }
        t.set_outcome(&Outcome {
            state: MeetingState::Done,
            at: 1_000,
            ended_at: done_at,
            reason: None,
            done_at: Some(done_at),
        })
        .unwrap();
    }

    /// The text of every passage in its `transcript.db`.
    pub fn passages(dir: &Path) -> Vec<(String, u64, String)> {
        Transcript::open(dir)
            .unwrap()
            .passages()
            .unwrap()
            .into_iter()
            .map(|p| (p.passage.id, p.passage.start, p.passage.text))
            .collect()
    }
}

/// Screen files for tests, from what is on disk: every `screen/frames/f-*.jpg` is closed, a
/// `screen/video/*.mp4` is closed once `<name>.closed` exists next to it, and the journal once its
/// last line has `"stop"`.
pub struct TestScreen;

impl ScreenFiles for TestScreen {
    fn list(&self, meeting_dir: &Path, _stopped: bool) -> std::io::Result<Vec<ScreenFile>> {
        let mut out = Vec::new();
        let names = |rel: &str| -> Vec<String> {
            let mut v: Vec<String> = fs::read_dir(meeting_dir.join(rel))
                .map(|e| {
                    e.filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            v.sort();
            v
        };
        for name in names("screen/frames") {
            if name.ends_with(".jpg") {
                out.push(ScreenFile {
                    path: format!("screen/frames/{name}"),
                    kind: ScreenFileKind::Frame,
                    closed: true,
                });
            }
        }
        let video = names("screen/video");
        for name in &video {
            if name.ends_with(".mp4") {
                out.push(ScreenFile {
                    path: format!("screen/video/{name}"),
                    kind: ScreenFileKind::Video,
                    closed: video.contains(&format!("{name}.closed")),
                });
            }
        }
        if let Ok(text) = fs::read_to_string(meeting_dir.join("screen/journal.jsonl")) {
            out.push(ScreenFile {
                path: "screen/journal.jsonl".into(),
                kind: ScreenFileKind::Journal,
                closed: text.lines().last().is_some_and(|l| l.contains("\"stop\"")),
            });
        }
        Ok(out)
    }
}

/// The screen folder's own closed-file rules (`voice_screen_files::media::closed_files`), as the engine's
/// `RecordedScreen` lists them: a frame once the journal names it, a video file after its `video_end`, the
/// journal after its `stop`; never clips, `screen.json` or `*.tmp`.
pub struct JournalScreen;

impl ScreenFiles for JournalScreen {
    fn list(&self, meeting_dir: &Path, _stopped: bool) -> std::io::Result<Vec<ScreenFile>> {
        use voice_screen_files::media::{closed_files, ScreenFileKind as Kind};
        Ok(closed_files(meeting_dir)?
            .into_iter()
            .map(|(path, kind)| ScreenFile {
                path,
                kind: match kind {
                    Kind::Journal => ScreenFileKind::Journal,
                    Kind::Frame => ScreenFileKind::Frame,
                    Kind::Video => ScreenFileKind::Video,
                },
                closed: true,
            })
            .collect())
    }
}

/// A JPEG's structure around stand-in bytes.
pub fn jpeg(seed: u8) -> Vec<u8> {
    let mut v = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 1, 1,
    ];
    v.extend((0..6000u32).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)));
    v.extend([0xFF, 0xD9]);
    v
}

/// An MP4's boxes (`ftyp`, `moov`, `mdat`) around `len` stand-in bytes.
pub fn mp4(len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(24u32.to_be_bytes());
    v.extend(b"ftypisom");
    v.extend(0x200u32.to_be_bytes());
    v.extend(b"isomiso2");
    v.extend(8u32.to_be_bytes());
    v.extend(b"moov");
    v.extend(((len + 8) as u32).to_be_bytes());
    v.extend(b"mdat");
    v.extend((0..len).map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1]));
    v
}

/// A frame and its folder.
pub fn write(meeting: &Meeting, rel: &str, bytes: &[u8]) {
    let path = meeting.path(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Objects whose decrypted bytes equal a file of a meeting folder.
pub fn uploaded_files(
    fake: &FakeS3,
    key: &MasterKey,
    meeting: &Meeting,
    rels: &[&str],
) -> HashSet<String> {
    let all: Vec<(String, Vec<u8>)> = objects(fake);
    let mut found = HashSet::new();
    for rel in rels {
        let bytes = meeting.read(rel);
        let object = file_object(key, &meeting.id, rel, &bytes);
        if let Some((_, cipher)) = all.iter().find(|(k, _)| *k == object) {
            assert_eq!(
                decrypt(key, &object, cipher),
                bytes,
                "{rel} decrypts to its file"
            );
            found.insert((*rel).to_owned());
        }
    }
    found
}

/// Removes the folder at the end of a test, quietly.
pub struct Cleanup(pub PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The meetings an `on_restored` callback was given.
pub type Seen = Arc<Mutex<Vec<String>>>;
/// What `BucketSync::restore` calls for each meeting it put in place.
pub type OnRestored = Box<dyn Fn(&str) + Send>;

/// Records which meetings `on_restored` named.
pub fn recorder() -> (Seen, OnRestored) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (
        seen,
        Box::new(move |id: &str| s.lock().unwrap().push(id.to_owned())),
    )
}
