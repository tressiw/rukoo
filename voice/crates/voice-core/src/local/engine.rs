//! The client for `rukoo-voice-local`, the local speech worker.
//!
//! The model runs in its own process so a crash in the recognizer can't take dictation down.
//! The worker starts on first use, loads the model once, unloads after the idle timeout and is
//! started again after a crash. A local model never falls back to a cloud provider: every
//! failure here is an error for the caller.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

#[cfg(windows)]
use super::job::Job;
use super::registry::{ModelRegistry, ModelSpec};
use crate::paths;
use crate::providers::http::seconds_text;
use crate::providers::{ErrorKind, Hints, ProviderError, Recognized, SpeechToText};

const STDERR_LINES: usize = 20;
const QUIT_GRACE: Duration = Duration::from_secs(2);
const MISSING_MODEL: &str = "Model missing. Download it in Settings > Transcription.";

#[derive(Clone, Debug)]
pub struct LocalOptions {
    /// None: `RUKOO_VOICE_LOCAL_EXE`, then `rukoo-voice-local` next to the running exe.
    pub worker_exe: Option<PathBuf>,
    pub registry: ModelRegistry,
    pub model: ModelSpec,
    /// Four threads outperformed two and all cores in Ownkey's measurements.
    pub threads: u32,
    /// Zero keeps the model loaded.
    pub idle_timeout: Duration,
    pub start_timeout: Duration,
    pub load_timeout: Duration,
    /// Allowed for a transcription on top of the audio's own length.
    pub transcribe_timeout: Duration,
}

impl LocalOptions {
    pub fn new(registry: ModelRegistry) -> LocalOptions {
        LocalOptions {
            worker_exe: None,
            registry,
            model: ModelSpec::orukeet(),
            threads: 4,
            idle_timeout: Duration::from_secs(20 * 60),
            start_timeout: Duration::from_secs(10),
            load_timeout: Duration::from_secs(180),
            transcribe_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalState {
    #[default]
    Unloaded,
    Loading,
    Loaded,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LocalStatus {
    pub state: LocalState,
    /// The last load or transcription error; cleared by the next success.
    pub error: Option<String>,
    pub model_dir: Option<PathBuf>,
    /// A transcription or load is running or waiting.
    pub busy: bool,
    pub worker_pid: Option<u32>,
}

/// A transcription with Orukeet's word pieces, their start times and their lengths in seconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timed {
    pub text: String,
    pub tokens: Vec<String>,
    pub timestamps: Vec<f64>,
    /// Aligned with `tokens`; empty when the worker doesn't send them.
    pub durations: Vec<f64>,
    pub ms: u64,
}

/// Dictionary words as sherpa-onnx hotwords: whitespace collapsed, empty terms dropped, and `/`
/// inside a term replaced by a space because `/` separates hotwords (`local_transcription.py:38-41`).
pub fn hotword_terms(vocabulary: &[String]) -> Vec<String> {
    vocabulary
        .iter()
        .map(|term| crate::cleanup::collapse_whitespace(term))
        .filter(|term| !term.is_empty())
        .map(|term| term.replace('/', " "))
        .collect()
}

/// The hotwords as the single string sherpa-onnx takes.
pub fn hotwords_string(vocabulary: &[String]) -> String {
    hotword_terms(vocabulary).join("/")
}

/// The last lines the worker wrote to stderr; sherpa-onnx explains bad configs only there.
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
        lock(&self.lines)
            .iter()
            .rev()
            .find(|line| !line.trim().is_empty())
            .cloned()
    }
}

struct Worker {
    child: Child,
    to_worker: Option<Sender<Vec<u8>>>,
    from_worker: Receiver<String>,
    stderr: Arc<StderrTail>,
    loaded: Option<PathBuf>,
    next_id: u64,
}

impl Worker {
    fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
        self.kill();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Inner {
    options: Mutex<LocalOptions>,
    worker: Mutex<Option<Worker>>,
    status: Mutex<LocalStatus>,
    last_used: Mutex<Instant>,
    in_flight: AtomicUsize,
    closed: AtomicBool,
    stop: Arc<(Mutex<bool>, Condvar)>,
    #[cfg(windows)]
    job: Option<Job>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let (flag, wake) = &*self.stop;
        *lock(flag) = true;
        wake.notify_all();
        if let Some(worker) = lock(&self.worker).take() {
            worker.stop();
        }
    }
}

/// The local speech engine. Cheap to clone; every clone talks to the same worker.
#[derive(Clone)]
pub struct LocalEngine {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for LocalEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalEngine")
            .field("status", &self.status())
            .finish()
    }
}

struct BusyGuard<'a>(&'a Inner);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
        *lock(&self.0.last_used) = Instant::now();
    }
}

impl LocalEngine {
    pub fn new(options: LocalOptions) -> LocalEngine {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let inner = Arc::new(Inner {
            options: Mutex::new(options),
            worker: Mutex::new(None),
            status: Mutex::new(LocalStatus::default()),
            last_used: Mutex::new(Instant::now()),
            in_flight: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            stop: stop.clone(),
            #[cfg(windows)]
            job: Job::new(),
        });
        let weak = Arc::downgrade(&inner);
        let _ = thread::Builder::new()
            .name("local-idle".into())
            .spawn(move || idle_loop(weak, stop));
        LocalEngine { inner }
    }

    pub fn status(&self) -> LocalStatus {
        let mut status = lock(&self.inner.status).clone();
        status.busy = self.inner.in_flight.load(Ordering::SeqCst) > 0;
        status
    }

    /// Uses another model location from now on, for example after the "Model folder" setting
    /// changed. A loaded model from another folder is replaced on the next use.
    pub fn set_registry(&self, registry: ModelRegistry) {
        lock(&self.inner.options).registry = registry;
    }

    /// Where the model is looked for now.
    pub fn registry(&self) -> ModelRegistry {
        lock(&self.inner.options).registry.clone()
    }

    pub fn set_idle_timeout(&self, timeout: Duration) {
        lock(&self.inner.options).idle_timeout = timeout;
    }

    /// Starts the worker and loads the model now, waiting for the answer. Settings uses it to
    /// check a model before it becomes the dictation provider, so a broken model never does.
    pub fn load(&self) -> Result<(), ProviderError> {
        self.with_worker(|_, _| Ok(()))
    }

    /// Starts the worker and loads the model in the background, so loading overlaps a
    /// recording that has just started. A failure shows in `status()`; the next use retries.
    pub fn preload(&self) {
        let engine = self.clone();
        let _ = thread::Builder::new()
            .name("local-preload".into())
            .spawn(move || {
                let _ = engine.with_worker(|_, _| Ok(()));
            });
    }

    /// Stops the worker now, which frees the model's memory. The next use starts it again.
    pub fn unload(&self) {
        if let Some(worker) = lock(&self.inner.worker).take() {
            worker.stop();
        }
        self.set_state(LocalState::Unloaded, None, None, None);
    }

    /// Stops the worker for good; later calls fail. For quitting the app.
    pub fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        self.unload();
    }

    /// A meeting window with its word timings. The caller has cut the audio already (windows of
    /// at most 28 s plus context, as Ownkey's `transcribe_timed`), so the worker doesn't split it
    /// again at 15 s.
    pub fn transcribe_timed(&self, pcm: &[i16], hints: &Hints) -> Result<Timed, ProviderError> {
        self.transcribe_with(pcm, hints, false)
    }

    /// `split`: let the worker cut audio over 15 s itself, as dictation needs.
    fn transcribe_with(
        &self,
        pcm: &[i16],
        hints: &Hints,
        split: bool,
    ) -> Result<Timed, ProviderError> {
        let audio_seconds = pcm.len() as f64 / 16_000.0;
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for sample in pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let hotwords = hotword_terms(&hints.vocabulary);
        self.with_worker(|slot, options| {
            let worker = slot.as_mut().expect("with_worker provides a loaded worker");
            worker.next_id += 1;
            let id = worker.next_id;
            let timeout = options.transcribe_timeout + Duration::from_secs_f64(audio_seconds);
            let mut request = json!({"op": "transcribe", "id": id, "rate": 16000, "bytes": bytes.len(), "hotwords": hotwords});
            if !split {
                request["split"] = Value::Bool(false);
            }
            let reply = request_reply(slot, &request, &bytes, timeout)?;
            if reply.get("id").and_then(Value::as_u64) != Some(id) {
                kill(slot);
                return Err(protocol_error());
            }
            if reply.get("ok") != Some(&Value::Bool(true)) {
                return Err(worker_error(&reply, "Orukeet couldn't transcribe this recording"));
            }
            Ok(Timed {
                text: reply.get("text").and_then(Value::as_str).unwrap_or_default().trim().to_string(),
                tokens: reply
                    .get("tokens")
                    .and_then(Value::as_array)
                    .map(|tokens| tokens.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default(),
                timestamps: reply
                    .get("timestamps")
                    .and_then(Value::as_array)
                    .map(|stamps| stamps.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default(),
                durations: reply
                    .get("durations")
                    .and_then(Value::as_array)
                    .map(|lengths| lengths.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default(),
                ms: reply.get("ms").and_then(Value::as_u64).unwrap_or(0),
            })
        })
    }

    /// Runs `job` with a started worker that has the model loaded.
    fn with_worker<T>(
        &self,
        job: impl FnOnce(&mut Option<Worker>, &LocalOptions) -> Result<T, ProviderError>,
    ) -> Result<T, ProviderError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(ProviderError::provider(
                "Local transcription is shutting down.",
            ));
        }
        self.inner.in_flight.fetch_add(1, Ordering::SeqCst);
        let _busy = BusyGuard(&self.inner);
        let mut slot = lock(&self.inner.worker);
        // Settings are copied so a Settings change doesn't wait for a long transcription.
        let options = lock(&self.inner.options).clone();
        let result = self
            .ensure_loaded(&mut slot, &options)
            .and_then(|()| job(&mut slot, &options));
        match &result {
            Ok(_) => {
                let dir = slot.as_ref().and_then(|w| w.loaded.clone());
                let pid = slot.as_ref().map(|w| w.child.id());
                self.set_state(LocalState::Loaded, None, dir, pid);
            }
            Err(error) => {
                let state = if slot.as_ref().is_some_and(|w| w.loaded.is_some()) {
                    LocalState::Loaded
                } else {
                    LocalState::Error
                };
                let dir = slot.as_ref().and_then(|w| w.loaded.clone());
                let pid = slot.as_ref().map(|w| w.child.id());
                self.set_state(state, Some(error.message.clone()), dir, pid);
            }
        }
        result
    }

    fn set_state(
        &self,
        state: LocalState,
        error: Option<String>,
        dir: Option<PathBuf>,
        pid: Option<u32>,
    ) {
        let mut status = lock(&self.inner.status);
        status.state = state;
        status.error = error;
        status.model_dir = dir;
        status.worker_pid = pid;
    }

    fn ensure_loaded(
        &self,
        slot: &mut Option<Worker>,
        options: &LocalOptions,
    ) -> Result<(), ProviderError> {
        if slot.as_mut().is_some_and(Worker::exited) {
            // It died while idle; a new one starts below.
            *slot = None;
        }
        let Some(located) = options.registry.locate(&options.model) else {
            return Err(ProviderError::new(
                ErrorKind::LocalMissingModel,
                MISSING_MODEL,
            ));
        };
        if slot.as_ref().and_then(|w| w.loaded.as_deref()) == Some(located.dir.as_path()) {
            return Ok(());
        }
        {
            let mut status = lock(&self.inner.status);
            status.state = LocalState::Loading;
        }
        options
            .registry
            .verify(&options.model, &located, &AtomicBool::new(false))
            .map_err(|error| ProviderError::new(ErrorKind::LocalMissingModel, error.message))?;
        if slot.is_none() {
            *slot = Some(self.spawn(options)?);
        }
        if slot.as_ref().is_some_and(|w| w.loaded.is_some()) {
            request_reply(slot, &json!({"op": "unload"}), &[], options.start_timeout)?;
            if let Some(worker) = slot.as_mut() {
                worker.loaded = None;
            }
        }
        let request = json!({
            "op": "load",
            "model": "orukeet",
            "dir": located.dir.display().to_string(),
            "threads": options.threads,
        });
        let reply = request_reply(slot, &request, &[], options.load_timeout)?;
        if reply.get("ok") != Some(&Value::Bool(true)) {
            let error = reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            let kind = match reply.get("kind").and_then(Value::as_str) {
                Some("missing_model" | "bad_model") => ErrorKind::LocalMissingModel,
                _ => ErrorKind::Provider,
            };
            return Err(ProviderError::new(
                kind,
                format!(
                    "Could not load Orukeet: {}. Try again in Settings > Transcription.",
                    error.trim_end_matches('.')
                ),
            ));
        }
        if let Some(worker) = slot.as_mut() {
            worker.loaded = Some(located.dir);
        }
        Ok(())
    }

    fn spawn(&self, options: &LocalOptions) -> Result<Worker, ProviderError> {
        let exe = options
            .worker_exe
            .clone()
            .or_else(paths::local_worker_exe)
            .unwrap_or_default();
        if !exe.is_file() {
            return Err(ProviderError::provider(format!(
                "Voice's local speech worker is missing ({}). Reinstall Rukoo Voice.",
                exe.display()
            )));
        }
        let mut child = worker_command(&exe).spawn().map_err(|error| {
            ProviderError::provider(format!(
                "Couldn't start Voice's local speech worker: {error}"
            ))
        })?;
        #[cfg(windows)]
        if let Some(job) = &self.inner.job {
            job.assign(&child);
        }
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProviderError::provider(
                "Couldn't start Voice's local speech worker.",
            ));
        };
        let worker = Worker {
            child,
            to_worker: Some(spawn_writer(stdin)),
            from_worker: spawn_reader(stdout),
            stderr: spawn_stderr(stderr),
            loaded: None,
            next_id: 0,
        };
        let mut slot = Some(worker);
        let reply = request_reply(
            &mut slot,
            &json!({"op": "ping"}),
            &[],
            options.start_timeout,
        )?;
        if reply.get("ok") != Some(&Value::Bool(true)) {
            kill(&mut slot);
            return Err(ProviderError::provider(
                "Voice's local speech worker didn't start properly.",
            ));
        }
        Ok(slot.take().expect("the worker answered the ping"))
    }
}

impl SpeechToText for LocalEngine {
    fn transcribe(&self, pcm: &[i16], hints: &Hints) -> Result<Recognized, ProviderError> {
        let timed = self.transcribe_with(pcm, hints, true)?;
        Ok(Recognized {
            text: timed.text,
            language: None,
        })
    }
}

fn worker_command(exe: &Path) -> Command {
    let mut command = Command::new(exe);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The worker ends with Voice, also when Voice is hard-killed. Windows does that with the job
    // object instead.
    #[cfg(unix)]
    command.env(super::parent::PARENT_PID, std::process::id().to_string());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up from a tray app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Requests go through a writer thread, so a worker that stops reading can't block the caller
/// past its timeout; killing the worker then ends the write.
fn spawn_writer(mut stdin: ChildStdin) -> Sender<Vec<u8>> {
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    let _ = thread::Builder::new()
        .name("local-worker-in".into())
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
        .name("local-worker-out".into())
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
        .name("local-worker-err".into())
        .spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let mut lines = lock(&sink.lines);
                if lines.len() == STDERR_LINES {
                    lines.pop_front();
                }
                lines.push_back(line.chars().take(500).collect());
            }
            sink.closed.store(true, Ordering::SeqCst);
        });
    tail
}

fn kill(slot: &mut Option<Worker>) {
    if let Some(worker) = slot.take() {
        worker.kill();
    }
}

fn protocol_error() -> ProviderError {
    ProviderError::provider(
        "Voice's local speech worker sent an answer Voice couldn't read. Try again.",
    )
}

fn crashed(last_line: Option<String>) -> ProviderError {
    let detail = last_line
        .map(|line| format!(" ({})", line.chars().take(180).collect::<String>()))
        .unwrap_or_default();
    ProviderError::provider(format!("Orukeet stopped unexpectedly{detail}. Try again."))
}

fn worker_error(reply: &Value, context: &str) -> ProviderError {
    let error = reply
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    let kind = match reply.get("kind").and_then(Value::as_str) {
        Some("missing_model" | "bad_model") => ErrorKind::LocalMissingModel,
        _ => ErrorKind::Provider,
    };
    ProviderError::new(kind, format!("{context}: {}", error.trim()))
}

/// Sends one request and waits for its answer. A worker that crashed, hung or answered garbage
/// is ended and removed from `slot`, so the next request starts a new one.
fn request_reply(
    slot: &mut Option<Worker>,
    request: &Value,
    payload: &[u8],
    timeout: Duration,
) -> Result<Value, ProviderError> {
    let worker = slot.as_mut().expect("a running worker");
    let mut message = serde_json::to_vec(request).unwrap_or_default();
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
    match answer {
        Ok(line) => serde_json::from_str(&line).map_err(|_| {
            kill(slot);
            protocol_error()
        }),
        Err(RecvTimeoutError::Timeout) => {
            kill(slot);
            Err(ProviderError::new(
                ErrorKind::Timeout,
                format!(
                    "Orukeet didn't finish within {}. Try again.",
                    seconds_text(timeout)
                ),
            ))
        }
        Err(RecvTimeoutError::Disconnected) => {
            let tail = slot.as_ref().map(|worker| worker.stderr.clone());
            kill(slot);
            // The worker is gone, so its stderr closes; wait briefly for its last words.
            let last_line = tail.and_then(|tail| tail.last_line(Duration::from_secs(1)));
            Err(crashed(last_line))
        }
    }
}

fn idle_loop(weak: Weak<Inner>, stop: Arc<(Mutex<bool>, Condvar)>) {
    loop {
        let interval = {
            let Some(inner) = weak.upgrade() else { return };
            let timeout = lock(&inner.options).idle_timeout;
            unload_if_idle(&inner, timeout);
            if timeout.is_zero() {
                Duration::from_secs(1)
            } else {
                (timeout / 4).clamp(Duration::from_millis(50), Duration::from_secs(1))
            }
        };
        let (flag, wake) = &*stop;
        let stopped = lock(flag);
        if *stopped {
            return;
        }
        let (stopped, _) = wake
            .wait_timeout(stopped, interval)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *stopped {
            return;
        }
    }
}

fn unload_if_idle(inner: &Inner, timeout: Duration) {
    if timeout.is_zero() || inner.in_flight.load(Ordering::SeqCst) > 0 {
        return;
    }
    if lock(&inner.last_used).elapsed() < timeout {
        return;
    }
    let mut slot = match inner.worker.try_lock() {
        Ok(slot) => slot,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if let Some(worker) = slot.take() {
        // Stopping the process releases everything the model held, which unloading inside a
        // long-lived process can't promise.
        worker.stop();
        let mut status = lock(&inner.status);
        status.state = LocalState::Unloaded;
        status.model_dir = None;
        status.worker_pid = None;
    }
}
