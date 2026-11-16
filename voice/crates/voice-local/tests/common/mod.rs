//! Drives the real `rukoo-voice-local` binary over its stdin and stdout.

#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Loading Orukeet takes about 2 s on the maintainer's PC; slow machines get plenty of room.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
const EXIT_TIMEOUT: Duration = Duration::from_secs(30);

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

pub struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    stderr: Arc<Mutex<String>>,
}

impl Worker {
    pub fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rukoo-voice-local"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start rukoo-voice-local");

        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let mut stderr_pipe = child.stderr.take().unwrap();
        let stderr = Arc::new(Mutex::new(String::new()));
        let collected = Arc::clone(&stderr);
        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(read) = stderr_pipe.read(&mut chunk) {
                if read == 0 {
                    break;
                }
                collected
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&chunk[..read]));
            }
        });

        Self {
            stdin: child.stdin.take(),
            child,
            lines,
            stderr,
        }
    }

    pub fn send_raw(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }

    pub fn send(&mut self, request: &Value) {
        self.send_with(request, &[]);
    }

    pub fn send_with(&mut self, request: &Value, payload: &[u8]) {
        let mut bytes = serde_json::to_vec(request).unwrap();
        bytes.push(b'\n');
        bytes.extend_from_slice(payload);
        self.send_raw(&bytes);
    }

    /// The next response line, parsed. Every line on stdout must be JSON.
    pub fn recv(&mut self) -> Value {
        match self.lines.recv_timeout(RESPONSE_TIMEOUT) {
            Ok(line) => serde_json::from_str(&line)
                .unwrap_or_else(|error| panic!("stdout line is not JSON ({error}): {line:?}")),
            Err(RecvTimeoutError::Timeout) => panic!("no response within {RESPONSE_TIMEOUT:?}"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("stdout closed; stderr: {}", self.stderr())
            }
        }
    }

    pub fn request(&mut self, request: &Value) -> Value {
        self.send(request);
        self.recv()
    }

    pub fn request_with(&mut self, request: &Value, payload: &[u8]) -> Value {
        self.send_with(request, payload);
        self.recv()
    }

    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    pub fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    /// Waits for the worker to exit and checks that it wrote nothing more on stdout.
    pub fn wait(&mut self) -> ExitStatus {
        let started = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                started.elapsed() < EXIT_TIMEOUT,
                "worker still running after {EXIT_TIMEOUT:?}"
            );
            thread::sleep(Duration::from_millis(20));
        };
        let extra: Vec<String> = self.lines.try_iter().collect();
        assert!(extra.is_empty(), "unexpected output: {extra:?}");
        status
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Only this test's own child, by handle.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rukoo-voice-local-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Names, sizes and modification times of a folder's entries, and of the folder itself.
pub fn listing(dir: &Path) -> Vec<(String, u64, SystemTime)> {
    let meta = fs::metadata(dir).unwrap();
    let mut entries = vec![(".".to_owned(), 0, meta.modified().unwrap())];
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let meta = entry.metadata().unwrap();
        entries.push((
            entry.file_name().to_string_lossy().into_owned(),
            meta.len(),
            meta.modified().unwrap(),
        ));
    }
    entries.sort();
    entries
}
