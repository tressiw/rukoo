//! A segment's lock dies with the process that holds it, so a recorder that was hard-killed
//! leaves its meeting writable (`voice/RECORDING.md`).
//!
//! The test binary plays both roles: started with `VOICE_AUDIO_TEST_ROLE=holder` it holds a
//! segment's lock until its parent kills it. It is a separate test binary on purpose: a process
//! that forks hands its open files, and their locks, to the child for the moment between `fork`
//! and `exec`, and other tests in the same binary spawn children all the time.

#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use voice_audio::{read_wav_i16, repair_segment, SegmentWriter};

const ROLE: &str = "VOICE_AUDIO_TEST_ROLE";
const FILE: &str = "VOICE_AUDIO_TEST_LOCK_FILE";
const READY: &str = "VOICE_AUDIO_TEST_LOCK_READY";

/// A folder for one test: removed afterwards, or kept when the test failed to look at it.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "voice-audio-lock-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("kept the test folder {}", self.0.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Holds a segment in a child process until its parent kills it.
fn hold(path: &Path, ready: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("a_child_holds_a_segment_for_its_parent")
        .arg("--include-ignored")
        .env(ROLE, "holder")
        .env(FILE, path)
        .env(READY, ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for_ready(ready: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "the holder never locked the segment"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The holder's role. Without the role this test does nothing, and it is `#[ignore]` so a normal
/// run doesn't count it.
#[test]
#[ignore]
fn a_child_holds_a_segment_for_its_parent() {
    if std::env::var_os(ROLE).is_none() {
        return;
    }
    let path = PathBuf::from(std::env::var_os(FILE).unwrap());
    let mut writer = SegmentWriter::create(&path).unwrap();
    writer.append(&[1, 2, 3, 4]).unwrap();
    std::fs::write(std::env::var_os(READY).unwrap(), b"").unwrap();
    // The parent kills this process; the sleep is only a guard against outliving it.
    std::thread::sleep(Duration::from_secs(60));
    drop(writer);
}

#[test]
fn a_hard_kill_frees_the_segment_for_the_next_writer() {
    let dir = TempDir::new("kill");
    let path = dir.file("000001.wav");
    let ready = dir.file("ready");
    let mut child = hold(&path, &ready);
    wait_for_ready(&ready);

    // While the holder lives, a second writer is refused and a reader still opens the segment.
    let err = repair_segment(&path).unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::WouldBlock,
        "a second writer opened a live segment: {err}"
    );
    assert_eq!(read_wav_i16(&path).unwrap().0, vec![1, 2, 3, 4]);

    // SIGKILL: nothing of the holder runs after it, as after a hard kill.
    child.kill().unwrap();
    child.wait().unwrap();

    // And the next session repairs it as it would after a crash.
    assert_eq!(repair_segment(&path).unwrap(), 4);
}
