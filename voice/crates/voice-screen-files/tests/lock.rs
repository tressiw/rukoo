//! A journal's one writer survives a hard kill: the lock goes with the process, so the next
//! screen process can write into the same meeting (`voice/RECORDING.md`).
//!
//! The test binary plays both roles, as the screen process's own crash tests do: started with
//! `VOICE_SCREEN_FILES_TEST_ROLE=holder` it holds a journal's lock until it is killed.

#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use voice_screen_files::journal::{self, JournalWriter, Line, OpenError};

const ROLE: &str = "VOICE_SCREEN_FILES_TEST_ROLE";
const MEETING: &str = "VOICE_SCREEN_FILES_TEST_MEETING";
const READY: &str = "VOICE_SCREEN_FILES_TEST_READY";

/// A folder for one test: removed afterwards, or kept when the test failed to look at it.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "voice-screen-files-lock-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
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

/// Holds the journal in a child process until its parent kills it.
fn hold(meeting: &Path, ready: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("a_child_holds_the_journal_for_its_parent")
        .arg("--include-ignored")
        .env(ROLE, "holder")
        .env(MEETING, meeting)
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
            "the holder never locked the journal"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The holder's role. Without the role this test does nothing, and it is `#[ignore]` so a normal
/// run doesn't count it.
#[test]
#[ignore]
fn a_child_holds_the_journal_for_its_parent() {
    if std::env::var_os(ROLE).is_none() {
        return;
    }
    let meeting = PathBuf::from(std::env::var_os(MEETING).unwrap());
    let writer = JournalWriter::open(&meeting).unwrap();
    std::fs::write(std::env::var_os(READY).unwrap(), b"").unwrap();
    // The parent kills this process; the sleep is only a guard against outliving it.
    std::thread::sleep(Duration::from_secs(60));
    drop(writer);
}

#[test]
fn a_hard_kill_frees_the_journal_for_the_next_writer() {
    let dir = TempDir::new("kill");
    let ready = dir.path().join("ready");
    let mut child = hold(dir.path(), &ready);
    wait_for_ready(&ready);

    // While the holder lives, a second writer is refused and a reader still follows the journal.
    assert!(matches!(
        JournalWriter::open(dir.path()),
        Err(OpenError::Busy)
    ));
    assert!(journal::read_all(dir.path()).unwrap().is_empty());

    // SIGKILL: nothing of the holder runs after it, as after a hard kill.
    child.kill().unwrap();
    child.wait().unwrap();

    let mut w = JournalWriter::open(dir.path()).unwrap();
    w.write(&Line::Lock { t: 1 }).unwrap();
    drop(w);
    assert_eq!(
        journal::read_all(dir.path()).unwrap(),
        vec![Line::Lock { t: 1 }]
    );
}
