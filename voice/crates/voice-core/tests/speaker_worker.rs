//! The client for `rukoo-voice-speakers` against a fake worker. Like tests/local_worker.rs, this
//! binary has its own main (`harness = false`) so it can play three roles: the test runner, the
//! fake worker (role "speakers"), and a stand-in Voice that owns a worker and gets killed (role
//! "parent"), to prove the worker dies with it. The role travels in an environment variable that
//! children inherit.
//!
//! The fake speaks the worker protocol: `ping`, `load`, `open`, `push` with raw s16le audio,
//! `finish` and `quit`. It answers 8 speakers at 10 ms frames, with a probability per frame and
//! speaker the test can recompute. The first sample of a push picks a failure, and marker files
//! in the runtime folder make `load` refuse or crash.

mod support;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use support::TempDir;
use voice_core::local::{Located, Location, ModelFile, ModelRegistry, ModelSpec};
use voice_core::speakers::{
    CpuFeatures, DiarError, DiarErrorKind, DiarStream, Diarizer, Probs, RuntimeInfo,
    RuntimeLocation, SpeakerLabels, SpeakerOptions, StreamGeometry, Unavailable,
};

const ROLE: &str = "VOICE_CORE_TEST_ROLE";
const PARENT_DATA: &str = "VOICE_CORE_TEST_DATA";
const SPEAKERS: usize = 8;
const MODEL_BYTES: &[u8] = b"GGUF: a small stand-in for Nemotron 3 Diarization";

/// First samples that make the fake fail a push.
const CRASH: i16 = 7777;
const HANG: i16 = 8888;
const BEHIND: i16 = 9999;
const GARBAGE: i16 = 6666;
/// Answers with frames that skip ahead of where the stream is.
const SKIP: i16 = 5555;
/// Answers with a frame count no buffer could hold.
const HUGE: i16 = 4444;

fn main() {
    match std::env::var(ROLE).as_deref() {
        Ok("speakers") => fake_worker(),
        Ok("parent") => parent(),
        _ => {
            // Single-threaded here, so setting the variable is safe; every worker the client
            // starts from now on inherits it.
            std::env::set_var(ROLE, "speakers");
            run_tests();
        }
    }
}

// The fake worker -----------------------------------------------------------------------------

/// The probability the fake reports for a frame and speaker.
fn prob(frame: u64, speaker: usize) -> f32 {
    ((frame * SPEAKERS as u64 + speaker as u64) % 997) as f32 / 1000.0
}

fn frames_reply(id: &Value, first: u64, frames: u64) -> Value {
    let mut bytes = Vec::new();
    for frame in first..first + frames {
        for speaker in 0..SPEAKERS {
            bytes.extend_from_slice(&prob(frame, speaker).to_le_bytes());
        }
    }
    json!({
        "id": id,
        "ok": true,
        "first": first,
        "frames": frames,
        "count": first + frames,
        "probs": base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn fake_worker() {
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut output = std::io::stdout().lock();
    let mut line = String::new();
    let mut open = false;
    let mut frame = 0u64;
    loop {
        line.clear();
        if input.read_line(&mut line).unwrap_or(0) == 0 {
            std::process::exit(0);
        }
        let request: Value = serde_json::from_str(line.trim()).expect("a JSON request");
        let id = request["id"].clone();
        let refuse =
            |kind: &str, error: &str| json!({"id": id, "ok": false, "kind": kind, "error": error});
        let reply = match request["op"].as_str().unwrap_or_default() {
            "ping" => json!({
                "id": id,
                "ok": true,
                "pid": std::process::id(),
                "version": "0.1.0",
                "cpu": {"avx2": true, "fma": true, "f16c": true, "bmi2": true},
            }),
            "load" => {
                let runtime = PathBuf::from(request["runtime"].as_str().unwrap_or_default());
                let model = PathBuf::from(request["model"].as_str().unwrap_or_default());
                let mut log = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(runtime.join("loads.log"))
                    .unwrap();
                writeln!(log, "{request}").unwrap();
                if runtime.join("refuse-runtime").exists() {
                    refuse("runtime", "nemo_speech_asr_c.dll is missing.")
                } else if runtime.join("refuse-model").exists() {
                    refuse("model", "nemo_speech_diar_create failed.")
                } else if runtime.join("crash-on-load").exists() {
                    eprintln!("fake speakers: ggml-cpu.dll crashed");
                    std::process::exit(4);
                } else if !model.is_file() {
                    refuse("model", "The model file is missing.")
                } else {
                    json!({"id": id, "ok": true, "speakers": SPEAKERS, "secondsPerFrame": 0.01, "loadMs": 1, "runtimeVersion": "fake"})
                }
            }
            "open" => {
                open = true;
                frame = 0;
                json!({"id": id, "ok": true})
            }
            "push" => {
                let mut audio = vec![0u8; request["bytes"].as_u64().unwrap_or(0) as usize];
                input.read_exact(&mut audio).expect("the audio bytes");
                let first = audio
                    .get(..2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .unwrap_or(0);
                match first {
                    CRASH => {
                        eprintln!("fake speakers: crashing on purpose");
                        std::process::exit(3);
                    }
                    HANG => {
                        std::thread::sleep(Duration::from_secs(60));
                        std::process::exit(0);
                    }
                    GARBAGE => {
                        writeln!(output, "this is not JSON").unwrap();
                        output.flush().unwrap();
                        continue;
                    }
                    BEHIND => refuse("behind", "Frames were dropped before Voice read them."),
                    SKIP => {
                        let frames = (audio.len() / 2 / 160) as u64;
                        frames_reply(&id, frame + 5, frames)
                    }
                    HUGE => json!({
                        "id": id, "ok": true, "first": frame, "frames": u64::MAX,
                        "count": u64::MAX, "probs": "",
                    }),
                    _ if !open => refuse("not_open", "No stream is open."),
                    _ => {
                        let frames = (audio.len() / 2 / 160) as u64;
                        let reply = frames_reply(&id, frame, frames);
                        frame += frames;
                        reply
                    }
                }
            }
            "finish" => {
                let reply = frames_reply(&id, frame, 3);
                frame += 3;
                reply
            }
            "close" => json!({"id": id, "ok": true}),
            "quit" => std::process::exit(0),
            other => refuse("bad_request", &format!("unknown op {other}")),
        };
        writeln!(output, "{reply}").unwrap();
        output.flush().unwrap();
    }
}

// Shared setup --------------------------------------------------------------------------------

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn spec() -> ModelSpec {
    ModelSpec {
        id: "nemotron-test".into(),
        label: "Nemotron test".into(),
        revision: "rev1".into(),
        folder: "nemotron-test".into(),
        settings_page: "Meetings > Speaker labels".into(),
        custom_folder: false,
        archive: None,
        files: vec![ModelFile {
            name: "model.gguf".into(),
            size: MODEL_BYTES.len() as u64,
            sha256: sha256(MODEL_BYTES),
            url: None,
        }],
    }
}

/// A runtime folder (with only the fake's marker files in it), the model in Voice's own folder,
/// and labels whose worker is this test binary.
struct Setup {
    _temp: Option<TempDir>,
    opts: SpeakerOptions,
    labels: SpeakerLabels,
    runtime: RuntimeInfo,
    model: Located,
}

fn setup(label: &str, tweak: impl FnOnce(&mut SpeakerOptions)) -> Setup {
    let temp = TempDir::new(label);
    let mut setup = setup_at(temp.path(), tweak);
    setup._temp = Some(temp);
    setup
}

fn setup_at(dir: &Path, tweak: impl FnOnce(&mut SpeakerOptions)) -> Setup {
    let runtime_dir = dir.join("nemo");
    fs::create_dir_all(&runtime_dir).unwrap();
    let registry = ModelRegistry::new(dir.join("data")).with_ownkey_models(None);
    let mut opts = SpeakerOptions::new(dir.join("exe"), registry);
    opts.worker_exe = Some(std::env::current_exe().unwrap());
    opts.model = spec();
    opts.cpu = CpuFeatures {
        avx2: true,
        fma: true,
        f16c: true,
        bmi2: true,
        neon: true,
        dotprod: true,
        fp16: true,
    };
    opts.ownkey_runtime = None;
    opts.nemo_dir = None;
    opts.no_speakers = None;
    opts.start_timeout = Duration::from_secs(10);
    opts.load_timeout = Duration::from_secs(10);
    opts.push_timeout = Duration::from_secs(10);
    opts.finish_timeout = Duration::from_secs(10);
    tweak(&mut opts);
    let model_dir = opts.registry.own_dir(&opts.model);
    fs::create_dir_all(&model_dir).unwrap();
    fs::write(model_dir.join("model.gguf"), MODEL_BYTES).unwrap();
    Setup {
        _temp: None,
        labels: SpeakerLabels::new(opts.clone()),
        opts,
        runtime: RuntimeInfo {
            dir: runtime_dir,
            location: RuntimeLocation::Dev,
            commit: String::new(),
        },
        model: Located {
            dir: model_dir,
            location: Location::Voice,
        },
    }
}

impl Setup {
    fn diarizer(&self) -> Arc<dyn Diarizer> {
        self.labels
            .worker_diarizer(self.runtime.clone(), self.model.clone())
    }

    fn open(&self) -> Box<dyn DiarStream> {
        match self.diarizer().open() {
            Ok(stream) => stream,
            Err(error) => panic!("open failed: {error:?}"),
        }
    }

    fn open_err(&self) -> DiarError {
        match self.diarizer().open() {
            Ok(_) => panic!("the stream opened"),
            Err(error) => error,
        }
    }

    fn mark(&self, name: &str) {
        fs::write(self.runtime.dir.join(name), b"").unwrap();
    }

    /// The `load` requests the fake received, oldest first.
    fn loads(&self) -> Vec<Value> {
        fs::read_to_string(self.runtime.dir.join("loads.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn pcm(samples: usize) -> Vec<i16> {
    (0..samples).map(|i| (i % 100) as i16).collect()
}

/// A second of audio whose first sample makes the fake fail.
fn magic(first: i16) -> Vec<i16> {
    let mut samples = pcm(16_000);
    samples[0] = first;
    samples
}

fn check_frames(probs: &Probs, first: u64, frames: usize) {
    assert_eq!(probs.first_frame, first);
    assert_eq!(probs.frames, frames);
    assert_eq!(probs.speakers, SPEAKERS);
    assert_eq!(probs.values.len(), frames * SPEAKERS);
    for (i, value) in probs.values.iter().enumerate() {
        let frame = first + (i / SPEAKERS) as u64;
        assert_eq!(*value, prob(frame, i % SPEAKERS), "frame {frame}");
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: the handle is checked and closed; the exit code pointer is a local.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        alive
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists. The client reaps its workers, so an
    // ended one leaves no zombie behind.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

// The tests -----------------------------------------------------------------------------------

fn probabilities_decode() {
    let s = setup("decode", |_| {});
    let mut stream = s.open();
    assert_eq!(
        stream.geometry(),
        StreamGeometry {
            speakers: SPEAKERS,
            seconds_per_frame: 0.01
        }
    );
    let pid = stream.worker_pid().expect("a worker pid");
    assert_ne!(pid, std::process::id());
    assert!(process_alive(pid));

    check_frames(&stream.push(&pcm(1_600)).unwrap(), 0, 10);
    check_frames(&stream.push(&pcm(3_200)).unwrap(), 10, 20);
    // Over 10 s goes to the worker in pieces and comes back as one run of frames.
    check_frames(&stream.push(&pcm(161_600)).unwrap(), 30, 1_010);
    // Nothing to push asks nothing.
    let empty = stream.push(&[]).unwrap();
    assert_eq!((empty.first_frame, empty.frames), (1_040, 0));
    check_frames(&stream.finish().unwrap(), 1_040, 3);
    let after = stream.push(&pcm(160)).unwrap_err();
    assert_eq!(after.kind, DiarErrorKind::Failed);
    assert_eq!(stream.worker_pid(), Some(pid), "one worker for the stream");

    // The load named the runtime, the model file and Nemotron's streaming setup.
    let loads = s.loads();
    assert_eq!(loads.len(), 1);
    let load = &loads[0];
    assert_eq!(
        load["runtime"].as_str().unwrap(),
        s.runtime.dir.display().to_string()
    );
    assert_eq!(
        load["model"].as_str().unwrap(),
        s.model.dir.join("model.gguf").display().to_string()
    );
    assert_eq!(load["preset"], "v3-streaming");
    assert_eq!(load["chunkFrames"], 38);
    assert_eq!(load["rightContextFrames"], 1);
    // A development folder needs no other folder for its imports.
    assert_eq!(load["dllDirs"], serde_json::json!([]));
    assert_eq!(s.labels.failure(), None);
}

fn each_stream_gets_its_own_worker() {
    let s = setup("fresh", |_| {});
    let first = s.open();
    let second = s.open();
    assert_ne!(first.worker_pid(), second.worker_pid());
    assert_eq!(s.loads().len(), 2);
}

fn a_worker_that_exits_mid_push_is_crashed() {
    let s = setup("crash", |_| {});
    let mut stream = s.open();
    let pid = stream.worker_pid().unwrap();
    stream.push(&pcm(1_600)).unwrap();
    let error = stream.push(&magic(CRASH)).unwrap_err();
    assert_eq!(error.kind, DiarErrorKind::Crashed);
    assert_eq!(
        error.message,
        "Voice's speaker label worker stopped unexpectedly (fake speakers: crashing on purpose)."
    );
    assert_eq!(stream.worker_pid(), None);
    wait_for("the crashed worker to be gone", || !process_alive(pid));
    assert_eq!(
        stream.push(&pcm(160)).unwrap_err().kind,
        DiarErrorKind::Crashed
    );
    // A crash after loading isn't remembered: the follower's next stream starts.
    assert_eq!(s.labels.failure(), None);
    s.open();
}

fn a_worker_that_stops_answering_times_out_and_is_killed() {
    let s = setup("hang", |opts| {
        opts.push_timeout = Duration::from_millis(300)
    });
    let mut stream = s.open();
    let pid = stream.worker_pid().unwrap();
    let started = Instant::now();
    let error = stream.push(&magic(HANG)).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(error.kind, DiarErrorKind::Timeout);
    assert_eq!(
        error.message,
        "Voice's speaker label worker didn't answer within 1 second."
    );
    assert_eq!(stream.worker_pid(), None);
    wait_for("the hung worker to be killed", || !process_alive(pid));
    assert_eq!(s.labels.failure(), None);
}

fn behind_keeps_the_worker() {
    let s = setup("behind", |_| {});
    let mut stream = s.open();
    let pid = stream.worker_pid().unwrap();
    let error = stream.push(&magic(BEHIND)).unwrap_err();
    assert_eq!(error.kind, DiarErrorKind::Behind);
    assert_eq!(error.message, "Frames were dropped before Voice read them.");
    assert_eq!(stream.worker_pid(), Some(pid));
}

fn garbage_ends_the_worker() {
    let s = setup("garbage", |_| {});
    let mut stream = s.open();
    let pid = stream.worker_pid().unwrap();
    let error = stream.push(&magic(GARBAGE)).unwrap_err();
    assert_eq!(error.kind, DiarErrorKind::Failed);
    assert_eq!(
        error.message,
        "Voice's speaker label worker sent an answer Voice couldn't read."
    );
    assert_eq!(stream.worker_pid(), None);
    wait_for("the worker to be ended", || !process_alive(pid));
}

fn frames_that_dont_continue_the_stream_end_the_worker() {
    for (what, first_sample) in [("skipped frames", SKIP), ("an absurd frame count", HUGE)] {
        let s = setup("discontinuous", |_| {});
        let mut stream = s.open();
        let pid = stream.worker_pid().unwrap();
        check_frames(&stream.push(&pcm(1_600)).unwrap(), 0, 10);
        let error = stream.push(&magic(first_sample)).unwrap_err();
        assert_eq!(error.kind, DiarErrorKind::Failed, "{what}");
        assert_eq!(
            error.message, "Voice's speaker label worker sent an answer Voice couldn't read.",
            "{what}"
        );
        assert_eq!(stream.worker_pid(), None, "{what}");
        wait_for("the worker to be ended", || !process_alive(pid));
        // Not a reason to give up on the runtime: the follower starts a new stream.
        assert_eq!(s.labels.failure(), None, "{what}");
    }
}

fn a_stream_outlives_the_labels_that_opened_it() {
    let s = setup("outlives", |_| {});
    // Nothing but the stream is kept: the diarizer and this SpeakerLabels are temporaries. The
    // job they own must stay open, or Windows ends the worker with it.
    let mut stream = SpeakerLabels::new(s.opts.clone())
        .worker_diarizer(s.runtime.clone(), s.model.clone())
        .open()
        .map_err(|error| error.message)
        .unwrap();
    let pid = stream.worker_pid().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(process_alive(pid), "the worker ended with its labels");
    check_frames(&stream.push(&pcm(1_600)).unwrap(), 0, 10);
    check_frames(&stream.finish().unwrap(), 10, 3);
    drop(stream);
    wait_for("the worker to quit", || !process_alive(pid));
}

fn a_runtime_refusal_while_loading_is_remembered() {
    let s = setup("refuse-runtime", |_| {});
    s.mark("refuse-runtime");
    let error = s.open_err();
    assert_eq!(error.kind, DiarErrorKind::Runtime);
    assert_eq!(
        error.message,
        format!(
            "Couldn't load the speaker label runtime from {}: nemo_speech_asr_c.dll is missing.",
            s.runtime.dir.display()
        )
    );
    let failed = Unavailable::RuntimeFailed {
        detail: error.message.clone(),
    };
    assert_eq!(s.labels.failure(), Some(failed.clone()));
    assert_eq!(s.loads().len(), 1);
    // The next stream is refused at once, without starting a worker.
    let again = s.open_err();
    assert_eq!(again, error);
    assert_eq!(s.loads().len(), 1);
    // Settings says so too, where the runtime can run at all.
    let expected = if cfg!(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )) {
        failed
    } else {
        Unavailable::Platform
    };
    assert_eq!(s.labels.availability(), Err(expected.clone()));
    assert_eq!(s.labels.diarizer().err(), Some(expected));
}

fn a_model_refusal_while_loading_is_remembered() {
    let s = setup("refuse-model", |_| {});
    s.mark("refuse-model");
    let error = s.open_err();
    assert_eq!(error.kind, DiarErrorKind::Model);
    assert_eq!(
        error.message,
        "Couldn't load the speaker model: nemo_speech_diar_create failed."
    );
    assert_eq!(
        s.labels.failure(),
        Some(Unavailable::RuntimeFailed {
            detail: error.message.clone()
        })
    );
}

fn a_crash_while_loading_is_remembered() {
    let s = setup("crash-on-load", |_| {});
    s.mark("crash-on-load");
    let error = s.open_err();
    assert_eq!(error.kind, DiarErrorKind::Crashed);
    assert_eq!(
        error.message,
        "Voice's speaker label worker stopped unexpectedly (fake speakers: ggml-cpu.dll crashed)."
    );
    assert_eq!(
        s.labels.failure(),
        Some(Unavailable::RuntimeFailed {
            detail: error.message.clone()
        })
    );
    assert_eq!(s.open_err().kind, DiarErrorKind::Runtime);
    assert_eq!(s.loads().len(), 1);
}

fn a_restart_tries_the_runtime_again() {
    let s = setup("restart", |_| {});
    s.mark("refuse-runtime");
    s.open_err();
    fs::remove_file(s.runtime.dir.join("refuse-runtime")).unwrap();
    assert_eq!(s.open_err().kind, DiarErrorKind::Runtime);
    // A new SpeakerLabels is a new Voice: nothing is remembered.
    let restarted = SpeakerLabels::new(s.opts.clone());
    assert_eq!(restarted.failure(), None);
    let mut stream = restarted
        .worker_diarizer(s.runtime.clone(), s.model.clone())
        .open()
        .map_err(|e| e.message)
        .unwrap();
    check_frames(&stream.push(&pcm(160)).unwrap(), 0, 1);
}

fn a_damaged_model_is_caught_before_a_worker_starts() {
    let s = setup("damaged", |_| {});
    let file = s.model.dir.join("model.gguf");
    let mut bytes = MODEL_BYTES.to_vec();
    bytes[0] ^= 0x20;
    fs::write(&file, &bytes).unwrap();
    let error = s.open_err();
    assert_eq!(error.kind, DiarErrorKind::Model);
    assert!(
        error
            .message
            .starts_with("Model file is damaged: model.gguf."),
        "{}",
        error.message
    );
    assert_eq!(
        s.labels.failure(),
        Some(Unavailable::ModelDamaged {
            detail: error.message.clone()
        })
    );
    assert!(s.loads().is_empty(), "no worker loaded a damaged model");
    assert_eq!(s.open_err(), error);
    // Downloaded again: the memory goes and the stream opens.
    fs::write(&file, MODEL_BYTES).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    s.open();
    assert_eq!(s.labels.failure(), None);
}

fn a_missing_worker_is_a_runtime_problem() {
    let temp = TempDir::new("no-worker");
    let missing = temp.join("rukoo-voice-speakers-missing.exe");
    let target = missing.clone();
    let s = setup("no-worker", move |opts| opts.worker_exe = Some(target));
    let error = s.open_err();
    assert_eq!(error.kind, DiarErrorKind::Runtime);
    assert_eq!(
        error.message,
        format!(
            "Voice's speaker label worker is missing ({}). Reinstall Rukoo Voice.",
            missing.display()
        )
    );
    assert_eq!(
        s.labels.failure(),
        Some(Unavailable::RuntimeFailed {
            detail: error.message
        })
    );
}

fn an_old_processor_never_starts_a_worker() {
    let s = setup("old-cpu", |opts| opts.cpu = CpuFeatures::default());
    let error = s.open_err();
    assert_eq!(error.kind, DiarErrorKind::Cpu);
    let missing = CpuFeatures::default().missing();
    assert_eq!(
        error.message,
        format!("This processor lacks {}.", missing.join(", "))
    );
    assert!(s.loads().is_empty());
}

fn drop_ends_the_worker() {
    let s = setup("drop", |_| {});
    let mut stream = s.open();
    let pid = stream.worker_pid().unwrap();
    stream.push(&pcm(1_600)).unwrap();
    let started = Instant::now();
    drop(stream);
    // The worker quits when asked, so nobody waits for the grace period.
    assert!(started.elapsed() < Duration::from_millis(1_900));
    wait_for("the worker to quit", || !process_alive(pid));
}

/// Role "parent": a stand-in Voice that opens a stream, reports its worker's pid and waits to be
/// killed.
fn parent() {
    std::env::set_var(ROLE, "speakers");
    let data = PathBuf::from(std::env::var(PARENT_DATA).expect("data folder"));
    let s = setup_at(&data, |opts| opts.push_timeout = Duration::from_secs(120));
    let mut stream = s.open();
    let pid = stream.worker_pid().expect("a worker");
    // Leave the worker busy in a long push, so it won't notice its stdin closing: only the job
    // object can end it when this process dies.
    std::thread::spawn(move || stream.push(&magic(HANG)).map(|_| ()));
    std::thread::sleep(Duration::from_millis(300));
    println!("pid {pid}");
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

#[cfg(windows)]
fn the_worker_dies_with_voice() {
    use std::process::{Command, Stdio};

    let data = TempDir::new("job");
    let mut voice = Command::new(std::env::current_exe().unwrap())
        .env(ROLE, "parent")
        .env(PARENT_DATA, data.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(voice.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let pid: u32 = line
        .trim()
        .strip_prefix("pid ")
        .expect("the worker pid")
        .parse()
        .unwrap();
    assert!(process_alive(pid));
    // A hard kill, like a crash: Voice gets no chance to stop its worker itself.
    voice.kill().unwrap();
    voice.wait().unwrap();
    wait_for("the worker to end with its job", || !process_alive(pid));
}

/// Kill-on-close job objects are Windows-only; elsewhere the client stops its workers itself.
#[cfg(windows)]
fn platform_tests() -> Vec<(&'static str, fn())> {
    vec![("the_worker_dies_with_voice", the_worker_dies_with_voice)]
}

#[cfg(not(windows))]
fn platform_tests() -> Vec<(&'static str, fn())> {
    Vec::new()
}

// The runner ----------------------------------------------------------------------------------

fn run_tests() {
    let mut tests: Vec<(&str, fn())> = vec![
        ("probabilities_decode", probabilities_decode),
        (
            "each_stream_gets_its_own_worker",
            each_stream_gets_its_own_worker,
        ),
        (
            "a_worker_that_exits_mid_push_is_crashed",
            a_worker_that_exits_mid_push_is_crashed,
        ),
        (
            "a_worker_that_stops_answering_times_out_and_is_killed",
            a_worker_that_stops_answering_times_out_and_is_killed,
        ),
        ("behind_keeps_the_worker", behind_keeps_the_worker),
        ("garbage_ends_the_worker", garbage_ends_the_worker),
        (
            "frames_that_dont_continue_the_stream_end_the_worker",
            frames_that_dont_continue_the_stream_end_the_worker,
        ),
        (
            "a_stream_outlives_the_labels_that_opened_it",
            a_stream_outlives_the_labels_that_opened_it,
        ),
        (
            "a_runtime_refusal_while_loading_is_remembered",
            a_runtime_refusal_while_loading_is_remembered,
        ),
        (
            "a_model_refusal_while_loading_is_remembered",
            a_model_refusal_while_loading_is_remembered,
        ),
        (
            "a_crash_while_loading_is_remembered",
            a_crash_while_loading_is_remembered,
        ),
        (
            "a_restart_tries_the_runtime_again",
            a_restart_tries_the_runtime_again,
        ),
        (
            "a_damaged_model_is_caught_before_a_worker_starts",
            a_damaged_model_is_caught_before_a_worker_starts,
        ),
        (
            "a_missing_worker_is_a_runtime_problem",
            a_missing_worker_is_a_runtime_problem,
        ),
        (
            "an_old_processor_never_starts_a_worker",
            an_old_processor_never_starts_a_worker,
        ),
        ("drop_ends_the_worker", drop_ends_the_worker),
    ];
    tests.extend(platform_tests());
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let selected: Vec<_> = tests
        .iter()
        .filter(|(name, _)| {
            filters.is_empty() || filters.iter().any(|filter| name.contains(filter.as_str()))
        })
        .collect();
    let plural = if selected.len() == 1 { "" } else { "s" };
    println!("\nrunning {} test{plural}", selected.len());
    let mut failed = Vec::new();
    for (name, test) in &selected {
        let started = Instant::now();
        let outcome = panic::catch_unwind(AssertUnwindSafe(test));
        let verdict = if outcome.is_ok() { "ok" } else { "FAILED" };
        println!(
            "test {name} ... {verdict} ({} ms)",
            started.elapsed().as_millis()
        );
        if outcome.is_err() {
            failed.push(*name);
        }
    }
    let passed = selected.len() - failed.len();
    if failed.is_empty() {
        println!("\ntest result: ok. {passed} passed; 0 failed");
    } else {
        println!(
            "\nfailures: {failed:?}\ntest result: FAILED. {passed} passed; {} failed",
            failed.len()
        );
        std::process::exit(1);
    }
}
