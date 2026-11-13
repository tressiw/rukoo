//! The real `rukoo-voice-local`, started the way Voice starts it, ends when Voice dies.
//!
//! Voice is a stand-in here because `kill -9` of a process this test owns would take the test
//! runner with it: a shell that names itself in the variable Voice writes, prints its pid and then
//! waits to be killed. The worker needs no model for this: it sits in its read loop.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_rukoo-voice-local");
/// What a worker exits with when its app is gone; see voice-core's `local::parent`.
const PARENT_GONE: i32 = 70;

/// A stand-in for Voice: a shell that prints its own pid and then waits to be killed.
fn stand_in_voice() -> (Child, u32) {
    let mut voice = Command::new("sh")
        .arg("-c")
        .arg("echo $$; exec sleep 60")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the stand-in Voice");
    let mut line = String::new();
    BufReader::new(voice.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .unwrap();
    let pid = line.trim().parse().expect("the shell's pid");
    (voice, pid)
}

fn worker(parent: u32) -> Child {
    Command::new(EXE)
        .env("RUKOO_VOICE_PARENT_PID", parent.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start rukoo-voice-local")
}

/// What the worker wrote to stderr, once it has ended.
fn read(mut stderr: impl Read) -> String {
    let mut text = String::new();
    stderr.read_to_string(&mut text).unwrap();
    text
}

#[test]
fn the_worker_ends_when_the_app_that_started_it_dies() {
    let (mut voice, voice_pid) = stand_in_voice();
    let mut child = worker(voice_pid);
    let stderr = child.stderr.take().expect("stderr");
    // Long enough to reach its read loop, from where nothing ends it but the watch.
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        matches!(child.try_wait(), Ok(None)),
        "the worker ended before Voice did"
    );

    voice.kill().expect("kill the stand-in Voice");
    voice.wait().unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(child.try_wait(), Ok(None)) {
        assert!(
            Instant::now() < deadline,
            "the worker is still running 2 s after Voice was killed: {}",
            read(stderr)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let status = child.wait().unwrap();
    let stderr = read(stderr);
    assert_eq!(status.code(), Some(PARENT_GONE), "{stderr}");
    assert!(stderr.contains("is gone"), "{stderr}");
}

#[test]
fn the_worker_does_not_start_with_a_parent_that_is_already_gone() {
    // Any process that has certainly ended.
    let mut gone = Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap();
    let gone_pid = gone.id();
    gone.wait().unwrap();

    let started = Instant::now();
    let output = worker(gone_pid).wait_with_output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(output.status.code(), Some(PARENT_GONE), "{output:?}");
    // Nothing answered a request, because nothing read one.
    assert!(output.stdout.is_empty(), "{output:?}");
}
