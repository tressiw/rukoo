//! The client for the `rukoo-voice-speakers` worker: one process per stream, in the kill-on-close
//! job on Windows and watching its parent on Unix, at below-normal priority.
//!
//! The protocol is voice-local's framing: one JSON request per line on stdin, raw audio bytes
//! after a request that names `bytes`, one JSON reply line per request on stdout, and the
//! worker's logs on stderr. A worker that crashes, hangs or answers garbage is ended, and the
//! stream reports why; the follower in voice-meetings decides whether to start a new one.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

use super::api::{
    DiarError, DiarErrorKind, DiarStream, Diarizer, Probs, StreamGeometry, CHUNK_FRAMES,
    RIGHT_CONTEXT_FRAMES,
};
use super::availability::SpeakerLabels;
use super::runtime::RuntimeInfo;
use crate::local::{Located, ModelErrorKind, ModelSpec};
use crate::providers::http::seconds_text;

const STDERR_LINES: usize = 20;
const QUIT_GRACE: Duration = Duration::from_secs(2);
/// The worker takes at most 10 s of audio (320,000 bytes) per request; longer pushes are split.
const MAX_PUSH_SAMPLES: usize = 160_000;
const SAMPLE_RATE: u32 = 16_000;
/// Nemotron 3 Diarization's streaming setup, as Ownkey runs it live: [`CHUNK_FRAMES`] frames of
/// 80 ms per chunk and [`RIGHT_CONTEXT_FRAMES`] of right context.
const PRESET: &str = "v3-streaming";

/// Opens streams in fresh workers with one verified runtime and model.
pub(crate) struct WorkerDiarizer {
    pub(crate) labels: SpeakerLabels,
    pub(crate) runtime: RuntimeInfo,
    pub(crate) model: Located,
}

impl Diarizer for WorkerDiarizer {
    fn open(&self) -> Result<Box<dyn DiarStream>, DiarError> {
        let opts = self.labels.options();
        if let Some(refusal) = self.labels.refusal(&opts.model, &self.model) {
            return Err(refusal);
        }
        let missing = opts.cpu.missing();
        if !missing.is_empty() {
            return Err(DiarError::new(
                DiarErrorKind::Cpu,
                format!("This processor lacks {}.", missing.join(", ")),
            ));
        }
        // Hashed once, then trusted through the stamp while the file stays the same.
        if let Err(error) = opts
            .registry
            .verify(&opts.model, &self.model, &AtomicBool::new(false))
        {
            return Err(match error.kind {
                ModelErrorKind::Damaged => {
                    self.labels
                        .remember_damaged_model(&opts.model, &self.model, &error.message);
                    DiarError::new(DiarErrorKind::Model, error.message)
                }
                ModelErrorKind::Missing => DiarError::new(DiarErrorKind::Model, error.message),
                _ => DiarError::new(DiarErrorKind::Failed, error.message),
            });
        }
        let exe = opts.worker().unwrap_or_default();
        if !exe.is_file() {
            let message = format!(
                "Voice's speaker label worker is missing ({}). Reinstall Rukoo Voice.",
                exe.display()
            );
            self.labels.remember_runtime_failure(&message);
            return Err(DiarError::new(DiarErrorKind::Runtime, message));
        }

        let mut slot = Some(Worker::spawn(&exe, &self.labels)?);
        call(&mut slot, "ping", json!({}), &[], opts.start_timeout)?;
        let load = json!({
            "runtime": self.runtime.dir.display().to_string(),
            "dllDirs": self
                .runtime
                .dll_dirs()
                .iter()
                .map(|dir| dir.display().to_string())
                .collect::<Vec<_>>(),
            "model": self.model.dir.join(gguf_name(&opts.model)).display().to_string(),
            "preset": PRESET,
            "chunkFrames": CHUNK_FRAMES,
            "rightContextFrames": RIGHT_CONTEXT_FRAMES,
        });
        let loaded = match call(&mut slot, "load", load, &[], opts.load_timeout) {
            Ok(reply) => reply,
            Err(error) => return Err(self.load_failed(error)),
        };
        let speakers = loaded.get("speakers").and_then(Value::as_u64).unwrap_or(0) as usize;
        let seconds_per_frame = loaded
            .get("secondsPerFrame")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if speakers == 0 || !(seconds_per_frame > 0.0 && seconds_per_frame.is_finite()) {
            return Err(garbage());
        }
        call(&mut slot, "open", json!({}), &[], opts.start_timeout)?;
        Ok(Box::new(WorkerStream {
            _labels: self.labels.clone(),
            worker: slot,
            geometry: StreamGeometry {
                speakers,
                seconds_per_frame,
            },
            push_timeout: opts.push_timeout,
            finish_timeout: opts.finish_timeout,
            next_frame: 0,
            finished: false,
        }))
    }
}

impl WorkerDiarizer {
    /// A load that can never work here is remembered until Voice restarts: the runtime or model
    /// refused, the processor lacks a feature, or the worker died while loading.
    fn load_failed(&self, error: DiarError) -> DiarError {
        let message = match error.kind {
            // Both already say what happened: the missing feature, or the worker's last words.
            DiarErrorKind::Cpu | DiarErrorKind::Crashed => error.message.clone(),
            DiarErrorKind::Runtime => format!(
                "Couldn't load the speaker label runtime from {}: {}",
                self.runtime.dir.display(),
                error.message
            ),
            DiarErrorKind::Model => {
                format!("Couldn't load the speaker model: {}", error.message)
            }
            _ => return error,
        };
        self.labels.remember_runtime_failure(&message);
        DiarError::new(error.kind, message)
    }
}

/// The model file the worker loads: the GGUF of the spec.
fn gguf_name(spec: &ModelSpec) -> &str {
    spec.files
        .iter()
        .find(|file| file.name.ends_with(".gguf"))
        .or(spec.files.first())
        .map_or("", |file| file.name.as_str())
}

/// One open stream in its own worker.
struct WorkerStream {
    /// Keeps the kill-on-close job open while the stream lives: closing its last handle would end
    /// this worker even when the caller has let go of the diarizer and `SpeakerLabels`.
    _labels: SpeakerLabels,
    worker: Option<Worker>,
    geometry: StreamGeometry,
    push_timeout: Duration,
    finish_timeout: Duration,
    /// The frame after the last one returned.
    next_frame: u64,
    finished: bool,
}

impl WorkerStream {
    fn empty(&self) -> Probs {
        Probs {
            first_frame: self.next_frame,
            frames: 0,
            speakers: self.geometry.speakers,
            values: Vec::new(),
        }
    }

    /// Adds one reply's frames to `into`. The frames must continue where the last ones ended:
    /// the worker reports dropped frames as `behind`, so a gap or an overlap means it lost track.
    fn take_frames(&mut self, reply: &Value, into: &mut Probs) -> Result<(), DiarError> {
        let first = reply.get("first").and_then(Value::as_u64);
        let frames = reply.get("frames").and_then(Value::as_u64);
        let encoded = reply.get("probs").and_then(Value::as_str).unwrap_or("");
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded);
        let (Some(first), Some(frames), Ok(bytes)) = (first, frames, decoded) else {
            return Err(self.garbage());
        };
        let expected = usize::try_from(frames)
            .ok()
            .and_then(|frames| frames.checked_mul(self.geometry.speakers))
            .and_then(|values| values.checked_mul(4));
        if expected != Some(bytes.len()) {
            return Err(self.garbage());
        }
        if frames == 0 {
            return Ok(());
        }
        if first != self.next_frame {
            return Err(self.garbage());
        }
        let Some(next_frame) = first.checked_add(frames) else {
            return Err(self.garbage());
        };
        if into.frames == 0 {
            into.first_frame = first;
        }
        into.frames += frames as usize;
        into.values.extend(
            bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        self.next_frame = next_frame;
        Ok(())
    }

    /// Ends the worker after an answer Voice can't use.
    fn garbage(&mut self) -> DiarError {
        drop(self.worker.take());
        garbage()
    }

    fn finished_error() -> DiarError {
        DiarError::new(
            DiarErrorKind::Failed,
            "This speaker label stream is finished.",
        )
    }
}

impl DiarStream for WorkerStream {
    fn geometry(&self) -> StreamGeometry {
        self.geometry
    }

    fn push(&mut self, pcm: &[i16]) -> Result<Probs, DiarError> {
        if self.finished {
            return Err(WorkerStream::finished_error());
        }
        let mut probs = self.empty();
        for chunk in pcm.chunks(MAX_PUSH_SAMPLES) {
            let mut bytes = Vec::with_capacity(chunk.len() * 2);
            for sample in chunk {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            let request = json!({"rate": SAMPLE_RATE, "bytes": bytes.len()});
            let reply = call(&mut self.worker, "push", request, &bytes, self.push_timeout)?;
            self.take_frames(&reply, &mut probs)?;
        }
        Ok(probs)
    }

    fn finish(&mut self) -> Result<Probs, DiarError> {
        if self.finished {
            return Err(WorkerStream::finished_error());
        }
        self.finished = true;
        let reply = call(
            &mut self.worker,
            "finish",
            json!({}),
            &[],
            self.finish_timeout,
        )?;
        let mut probs = self.empty();
        self.take_frames(&reply, &mut probs)?;
        Ok(probs)
    }

    fn worker_pid(&self) -> Option<u32> {
        self.worker.as_ref().map(|worker| worker.child.id())
    }
}

impl Drop for WorkerStream {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.stop();
        }
    }
}

/// The last lines the worker wrote to stderr; NeMo explains failures only there.
#[derive(Default)]
struct StderrTail {
    lines: Mutex<VecDeque<String>>,
    closed: AtomicBool,
}

impl StderrTail {
    /// The last non-empty line, once the worker's stderr has closed or `wait` has passed.
    fn last_line(&self, wait: Duration) -> Option<String> {
        let deadline = Instant::now() + wait;
        while !self.closed.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .rev()
            .find(|line| !line.trim().is_empty())
            .cloned()
    }
}

/// A running worker process and its pipes. Dropping it ends the process at once.
struct Worker {
    child: Child,
    to_worker: Option<Sender<Vec<u8>>>,
    from_worker: Receiver<String>,
    stderr: Arc<StderrTail>,
    next_id: u64,
}

impl Worker {
    fn spawn(exe: &Path, labels: &SpeakerLabels) -> Result<Worker, DiarError> {
        let mut command = Command::new(exe);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // The worker ends with Voice, also when Voice is hard-killed.
            command.env(
                crate::local::parent::PARENT_PID,
                std::process::id().to_string(),
            );
            // Below normal, like the Windows priority class: recognition and dictation come first
            // when the processor is busy. A refused call keeps the default priority.
            // SAFETY: setpriority is async-signal-safe, which is all a child may do between fork
            // and exec.
            unsafe {
                command.pre_exec(|| {
                    let _ = libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window from a tray app, and meetings' recognition and dictation come
            // first when the processor is busy.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            command.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        }
        let mut child = command.spawn().map_err(|error| {
            DiarError::new(
                DiarErrorKind::Failed,
                format!("Couldn't start Voice's speaker label worker: {error}"),
            )
        })?;
        #[cfg(windows)]
        labels.adopt(&child);
        #[cfg(not(windows))]
        let _ = labels;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(DiarError::new(
                DiarErrorKind::Failed,
                "Couldn't start Voice's speaker label worker.",
            ));
        };
        Ok(Worker {
            child,
            to_worker: Some(spawn_writer(stdin)),
            from_worker: spawn_reader(stdout),
            stderr: spawn_stderr(stderr),
            next_id: 0,
        })
    }

    /// Asks the worker to quit and gives it a moment before ending it.
    fn stop(mut self) {
        if let Some(sender) = self.to_worker.take() {
            let _ = sender.send(b"{\"op\":\"quit\"}\n".to_vec());
        }
        let deadline = Instant::now() + QUIT_GRACE;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        // Drop kills it.
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Requests go through a writer thread, so a worker that stops reading can't block the caller
/// past its timeout; ending the worker then ends the write.
fn spawn_writer(mut stdin: ChildStdin) -> Sender<Vec<u8>> {
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let _ = thread::Builder::new()
        .name("speakers-worker-in".into())
        .spawn(move || {
            for message in receiver {
                if stdin
                    .write_all(&message)
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
    sender
}

fn spawn_reader(stdout: impl Read + Send + 'static) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    let _ = thread::Builder::new()
        .name("speakers-worker-out".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if sender.send(line.trim_end().to_string()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    receiver
}

fn spawn_stderr(stderr: impl Read + Send + 'static) -> Arc<StderrTail> {
    let tail = Arc::new(StderrTail::default());
    let sink = tail.clone();
    let _ = thread::Builder::new()
        .name("speakers-worker-err".into())
        .spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let mut lines = sink.lines.lock().unwrap_or_else(PoisonError::into_inner);
                if lines.len() == STDERR_LINES {
                    lines.pop_front();
                }
                lines.push_back(line.chars().take(500).collect());
            }
            sink.closed.store(true, Ordering::SeqCst);
        });
    tail
}

fn garbage() -> DiarError {
    DiarError::new(
        DiarErrorKind::Failed,
        "Voice's speaker label worker sent an answer Voice couldn't read.",
    )
}

fn not_running() -> DiarError {
    DiarError::new(
        DiarErrorKind::Crashed,
        "Voice's speaker label worker isn't running.",
    )
}

/// The failure kind a refusing worker named.
fn reply_kind(reply: &Value) -> DiarErrorKind {
    match reply.get("kind").and_then(Value::as_str) {
        Some("cpu") => DiarErrorKind::Cpu,
        Some("runtime") => DiarErrorKind::Runtime,
        Some("model") => DiarErrorKind::Model,
        Some("behind") => DiarErrorKind::Behind,
        _ => DiarErrorKind::Failed,
    }
}

/// Sends one request and waits for its answer. A refusal comes back with the worker's kind and
/// message, and the worker keeps running. A worker that crashed, hung or answered garbage is
/// ended and removed from `slot`.
fn call(
    slot: &mut Option<Worker>,
    op: &str,
    mut request: Value,
    payload: &[u8],
    timeout: Duration,
) -> Result<Value, DiarError> {
    let Some(worker) = slot.as_mut() else {
        return Err(not_running());
    };
    worker.next_id += 1;
    let id = worker.next_id;
    request["op"] = json!(op);
    request["id"] = json!(id);
    let mut message = serde_json::to_vec(&request).unwrap_or_default();
    message.push(b'\n');
    message.extend_from_slice(payload);
    let sent = worker
        .to_worker
        .as_ref()
        .is_some_and(|sender| sender.send(message).is_ok());
    let answer = if sent {
        worker.from_worker.recv_timeout(timeout)
    } else {
        Err(RecvTimeoutError::Disconnected)
    };
    let line = match answer {
        Ok(line) => line,
        Err(RecvTimeoutError::Timeout) => {
            drop(slot.take());
            return Err(DiarError::new(
                DiarErrorKind::Timeout,
                format!(
                    "Voice's speaker label worker didn't answer within {}.",
                    seconds_text(timeout)
                ),
            ));
        }
        Err(RecvTimeoutError::Disconnected) => {
            let tail = slot.as_ref().map(|worker| worker.stderr.clone());
            drop(slot.take());
            // The worker is gone, so its stderr closes; wait briefly for its last words.
            let detail = tail
                .and_then(|tail| tail.last_line(Duration::from_secs(1)))
                .map(|line| format!(" ({})", line.chars().take(180).collect::<String>()))
                .unwrap_or_default();
            return Err(DiarError::new(
                DiarErrorKind::Crashed,
                format!("Voice's speaker label worker stopped unexpectedly{detail}."),
            ));
        }
    };
    let reply = match serde_json::from_str::<Value>(&line) {
        Ok(reply) if reply.get("id").and_then(Value::as_u64) == Some(id) => reply,
        _ => {
            drop(slot.take());
            return Err(garbage());
        }
    };
    if reply.get("ok") != Some(&Value::Bool(true)) {
        let error = reply
            .get("error")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|error| !error.is_empty())
            .unwrap_or("unknown error");
        return Err(DiarError::new(reply_kind(&reply), error));
    }
    Ok(reply)
}
