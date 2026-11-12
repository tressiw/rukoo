//! The local worker client against a fake worker that speaks the protocol in architecture.md.
//!
//! This test binary has its own main (`harness = false`) so it can play three roles: the test
//! runner, the fake `rukoo-voice-local` (role "worker"), and a stand-in Voice that owns a worker
//! and gets killed (role "parent"), to prove the worker dies with it. The role travels in an
//! environment variable that children inherit.

mod support;

use std::io::{BufRead, BufReader, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use support::{FakeServer, Reply, TempDir};
use voice_core::local::{
    LocalEngine, LocalOptions, LocalState, ModelFile, ModelRegistry, ModelSpec,
};
use voice_core::providers::{speech_to_text, ErrorKind, Hints, ProviderConfig, SpeechToText};

const ROLE: &str = "VOICE_CORE_TEST_ROLE";
const PARENT_DATA: &str = "VOICE_CORE_TEST_DATA";
const GOOD_TOKENS: &[u8] = b"<blk> 0\nhello 1\n";
const BAD_TOKENS: &[u8] = b"BAD tokens file!\n";

fn main() {
    match std::env::var(ROLE).as_deref() {
        Ok("worker") => fake_worker(),
        Ok("parent") => parent(),
        #[cfg(unix)]
        Ok("watched") => watched(),
        _ => {
            // Single-threaded here, so setting the variable is safe; every worker the engines
            // start from now on inherits it.
            std::env::set_var(ROLE, "worker");
            run_tests();
        }
    }
}

// The fake worker -----------------------------------------------------------------------------

fn fake_worker() {
    // What the real worker's `main` does first: when the app that started it is gone, end. The
    // engine sets the variable, so this is the same path Voice takes.
    voice_core::local::parent::watch(|line| eprintln!("fake worker: {line}"));
    let stdin = std::io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let mut output = std::io::stdout().lock();
    let mut loads = 0;
    let mut loaded: Option<(String, u64)> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).unwrap_or(0) == 0 {
            std::process::exit(0);
        }
        let request: Value = serde_json::from_str(line.trim()).expect("a JSON request");
        let reply = match request["op"].as_str().unwrap_or_default() {
            "ping" => json!({"ok": true, "version": "0.1.0"}),
            "load" => {
                let dir = request["dir"].as_str().unwrap_or_default().to_string();
                match std::fs::read(Path::new(&dir).join("tokens.txt")) {
                    Err(_) => {
                        json!({"ok": false, "kind": "missing_model", "error": "tokens.txt is missing"})
                    }
                    Ok(tokens) if tokens.starts_with(b"BAD") => {
                        eprintln!("c-api.cc:SherpaOnnxCreateOfflineRecognizer: Errors in config");
                        json!({"ok": false, "kind": "bad_model", "error": "Errors in config."})
                    }
                    Ok(_) => {
                        loads += 1;
                        loaded = Some((dir, request["threads"].as_u64().unwrap_or(0)));
                        json!({"ok": true, "loadMs": 5})
                    }
                }
            }
            "transcribe" => {
                let mut audio = vec![0u8; request["bytes"].as_u64().unwrap_or(0) as usize];
                input.read_exact(&mut audio).expect("the audio bytes");
                let id = request["id"].clone();
                let hotwords: Vec<String> = request["hotwords"]
                    .as_array()
                    .map(|words| {
                        words
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let magic = |word: &str| hotwords.iter().any(|h| h == word);
                if magic("__crash__") {
                    eprintln!("fake worker: crashing on purpose");
                    std::process::exit(3);
                }
                if magic("__hang__") {
                    std::thread::sleep(Duration::from_secs(60));
                    std::process::exit(0);
                }
                if magic("__garbage__") {
                    writeln!(output, "this is not JSON").unwrap();
                    output.flush().unwrap();
                    continue;
                }
                match &loaded {
                    None => {
                        json!({"ok": false, "id": id, "kind": "failed", "error": "no model loaded"})
                    }
                    Some(_) if magic("__fail__") => {
                        json!({"ok": false, "id": id, "kind": "failed", "error": "decode failed on purpose"})
                    }
                    Some((_, threads)) => json!({
                        "ok": true,
                        "id": id,
                        "text": format!(
                            " {} samples|{}|loads {loads}|pid {}|threads {threads}|split {} ",
                            audio.len() / 2,
                            hotwords.join(","),
                            std::process::id(),
                            // Left out means split, as in the real worker.
                            request["split"].as_bool().unwrap_or(true)
                        ),
                        "tokens": ["\u{2581}hello", "\u{2581}world"],
                        "timestamps": [0.12, 0.48],
                        "durations": [0.3, 0.25],
                        "ms": 3
                    }),
                }
            }
            "unload" => {
                loaded = None;
                json!({"ok": true})
            }
            "quit" => std::process::exit(0),
            other => json!({"ok": false, "kind": "failed", "error": format!("unknown op {other}")}),
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

fn model_files(tokens: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("tokens.txt", tokens.to_vec()),
        ("encoder.int8.onnx", b"fake encoder".to_vec()),
    ]
}

fn spec(tokens: &[u8]) -> ModelSpec {
    ModelSpec {
        id: "orukeet-onnx-int8".into(),
        label: "Orukeet".into(),
        revision: "test-revision".into(),
        folder: "orukeet".into(),
        settings_page: "Transcription".into(),
        custom_folder: true,
        archive: None,
        files: model_files(tokens)
            .into_iter()
            .map(|(name, bytes)| ModelFile {
                name: name.into(),
                size: bytes.len() as u64,
                sha256: sha256(&bytes),
                url: None,
            })
            .collect(),
    }
}

fn install_model(dir: &Path, tokens: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, bytes) in model_files(tokens) {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
}

fn options(data: &Path, tokens: &[u8]) -> LocalOptions {
    let mut options = LocalOptions::new(ModelRegistry::new(data).with_ownkey_models(None));
    options.worker_exe = Some(std::env::current_exe().unwrap());
    options.model = spec(tokens);
    options.threads = 2;
    options.start_timeout = Duration::from_secs(10);
    options.load_timeout = Duration::from_secs(10);
    options.transcribe_timeout = Duration::from_secs(10);
    options
}

/// A data folder with the synthetic model in Voice's own models folder, and an engine for it.
fn engine_with_model(label: &str) -> (TempDir, LocalEngine) {
    let data = TempDir::new(label);
    let options = options(data.path(), GOOD_TOKENS);
    install_model(&options.registry.own_dir(&options.model), GOOD_TOKENS);
    (data, LocalEngine::new(options))
}

fn pcm(samples: usize) -> Vec<i16> {
    (0..samples).map(|i| (i % 100) as i16).collect()
}

fn words(list: &[&str]) -> Hints {
    Hints {
        vocabulary: list.iter().map(|w| w.to_string()).collect(),
        language: None,
    }
}

/// The worker's echo: samples, hotwords, loads, pid, threads.
fn echo(engine: &LocalEngine, hints: &Hints) -> Vec<String> {
    let text = engine
        .transcribe(&pcm(1600), hints)
        .expect("transcribe")
        .text;
    text.split('|').map(str::to_string).collect()
}

fn wait_for(what: &str, done: impl FnMut() -> bool) {
    // Generous: CI machines are slow, and the idle check runs on its own clock.
    wait_until(Duration::from_secs(10), what, done);
}

fn wait_until(within: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// The tests -----------------------------------------------------------------------------------

fn loads_lazily_once_and_passes_hotwords() {
    let (_data, engine) = engine_with_model("lazy");
    let status = engine.status();
    assert_eq!(status.state, LocalState::Unloaded);
    assert_eq!(
        status.worker_pid, None,
        "nothing starts before the first use"
    );

    let first = echo(&engine, &words(&["Bart", "a/b", "  Rukoo \t Mail ", ""]));
    // The text is trimmed; the hotwords had their whitespace collapsed and "/" replaced.
    assert_eq!(first[0], "1600 samples");
    assert_eq!(first[1], "Bart,a b,Rukoo Mail");
    assert_eq!(first[2], "loads 1");
    assert_eq!(first[4], "threads 2");
    let second = echo(&engine, &Hints::default());
    assert_eq!(second[2], "loads 1", "the model loads once");
    assert_eq!(second[3], first[3], "the same worker serves both");

    let status = engine.status();
    assert_eq!(status.state, LocalState::Loaded);
    assert_eq!(status.error, None);
    assert!(status
        .model_dir
        .unwrap()
        .ends_with(Path::new("orukeet").join("test-revision")));
    assert_eq!(format!("pid {}", status.worker_pid.unwrap()), first[3]);
}

fn timed_results_carry_tokens() {
    let (_data, engine) = engine_with_model("timed");
    let timed = engine
        .transcribe_timed(&pcm(800), &Hints::default())
        .unwrap();
    assert!(timed.text.starts_with("800 samples"));
    assert_eq!(timed.tokens, vec!["\u{2581}hello", "\u{2581}world"]);
    assert_eq!(timed.timestamps, vec![0.12, 0.48]);
    assert_eq!(timed.durations, vec![0.3, 0.25]);
    assert_eq!(timed.ms, 3);
    // A meeting window is cut already; dictation lets the worker split long audio.
    assert!(timed.text.ends_with("split false"), "{}", timed.text);
    assert_eq!(echo(&engine, &Hints::default())[5], "split true");
}

fn a_missing_model_never_starts_the_worker() {
    let data = TempDir::new("missing");
    let engine = LocalEngine::new(options(data.path(), GOOD_TOKENS));
    let error = engine
        .transcribe(&pcm(1600), &Hints::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::LocalMissingModel);
    assert_eq!(
        error.message,
        "Model missing. Download it in Settings > Transcription."
    );
    let status = engine.status();
    assert_eq!(status.state, LocalState::Error);
    assert_eq!(status.worker_pid, None);
}

fn a_damaged_model_is_caught_before_loading() {
    let data = TempDir::new("damaged");
    let options = options(data.path(), GOOD_TOKENS);
    let dir = options.registry.own_dir(&options.model);
    // Same size as the pinned tokens file, other bytes.
    let mut tokens = GOOD_TOKENS.to_vec();
    tokens[0] = b'#';
    install_model(&dir, &tokens);
    let engine = LocalEngine::new(options);
    let error = engine
        .transcribe(&pcm(1600), &Hints::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::LocalMissingModel);
    assert!(
        error
            .message
            .starts_with("Model file is damaged: tokens.txt."),
        "{}",
        error.message
    );
    assert_eq!(engine.status().worker_pid, None);
}

fn load_errors_are_reported() {
    let data = TempDir::new("bad-model");
    let options = options(data.path(), BAD_TOKENS);
    install_model(&options.registry.own_dir(&options.model), BAD_TOKENS);
    let engine = LocalEngine::new(options);
    let error = engine
        .transcribe(&pcm(1600), &Hints::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::LocalMissingModel);
    assert_eq!(
        error.message,
        "Could not load Orukeet: Errors in config. Try again in Settings > Transcription."
    );
    let status = engine.status();
    assert_eq!(status.state, LocalState::Error);
    assert_eq!(status.error.as_deref(), Some(error.message.as_str()));
}

fn a_crash_is_reported_and_the_next_use_restarts() {
    let (_data, engine) = engine_with_model("crash");
    let before = echo(&engine, &Hints::default());
    let error = engine
        .transcribe(&pcm(1600), &words(&["__crash__"]))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Provider);
    assert_eq!(
        error.message,
        "Orukeet stopped unexpectedly (fake worker: crashing on purpose). Try again."
    );
    assert_eq!(engine.status().worker_pid, None);
    let after = echo(&engine, &Hints::default());
    assert_ne!(after[3], before[3], "a new worker");
    assert_eq!(after[2], "loads 1");
}

fn a_hung_worker_times_out_and_is_replaced() {
    let data = TempDir::new("hang");
    let mut options = options(data.path(), GOOD_TOKENS);
    options.transcribe_timeout = Duration::from_millis(300);
    install_model(&options.registry.own_dir(&options.model), GOOD_TOKENS);
    let engine = LocalEngine::new(options);
    let before = echo(&engine, &Hints::default());
    let started = Instant::now();
    let error = engine
        .transcribe(&pcm(1600), &words(&["__hang__"]))
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(error.kind, ErrorKind::Timeout);
    assert_eq!(
        error.message,
        "Orukeet didn't finish within 1 second. Try again."
    );
    let after = echo(&engine, &Hints::default());
    assert_ne!(after[3], before[3]);
}

fn a_failed_decode_keeps_the_worker() {
    let (_data, engine) = engine_with_model("fail");
    let before = echo(&engine, &Hints::default());
    let error = engine
        .transcribe(&pcm(1600), &words(&["__fail__"]))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Provider);
    assert_eq!(
        error.message,
        "Orukeet couldn't transcribe this recording: decode failed on purpose"
    );
    let after = echo(&engine, &Hints::default());
    assert_eq!(after[3], before[3]);
    assert_eq!(after[2], "loads 1");
}

fn garbage_from_the_worker_replaces_it() {
    let (_data, engine) = engine_with_model("garbage");
    let before = echo(&engine, &Hints::default());
    let error = engine
        .transcribe(&pcm(1600), &words(&["__garbage__"]))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Provider);
    assert_eq!(
        error.message,
        "Voice's local speech worker sent an answer Voice couldn't read. Try again."
    );
    let after = echo(&engine, &Hints::default());
    assert_ne!(after[3], before[3]);
}

fn idle_unload_stops_the_worker() {
    let data = TempDir::new("idle");
    let mut options = options(data.path(), GOOD_TOKENS);
    options.idle_timeout = Duration::from_millis(300);
    install_model(&options.registry.own_dir(&options.model), GOOD_TOKENS);
    let engine = LocalEngine::new(options);
    let before = echo(&engine, &Hints::default());
    wait_for("the idle unload", || {
        engine.status().state == LocalState::Unloaded
    });
    assert_eq!(engine.status().worker_pid, None);
    let after = echo(&engine, &Hints::default());
    assert_ne!(after[3], before[3], "the model loads again in a new worker");
    assert_eq!(after[2], "loads 1");
}

fn zero_idle_keeps_the_model_loaded() {
    let data = TempDir::new("no-idle");
    let mut options = options(data.path(), GOOD_TOKENS);
    options.idle_timeout = Duration::ZERO;
    install_model(&options.registry.own_dir(&options.model), GOOD_TOKENS);
    let engine = LocalEngine::new(options);
    let before = echo(&engine, &Hints::default());
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(engine.status().state, LocalState::Loaded);
    assert_eq!(echo(&engine, &Hints::default())[3], before[3]);
}

fn preload_starts_loading_before_the_audio_arrives() {
    let (_data, engine) = engine_with_model("preload");
    engine.preload();
    wait_for("the preload", || {
        engine.status().state == LocalState::Loaded && !engine.status().busy
    });
    let pid = engine.status().worker_pid.unwrap();
    let echo = echo(&engine, &Hints::default());
    assert_eq!(echo[2], "loads 1");
    assert_eq!(echo[3], format!("pid {pid}"));
}

fn load_checks_the_model_before_any_audio() {
    let (_data, engine) = engine_with_model("load-now");
    engine.load().unwrap();
    let status = engine.status();
    assert_eq!(status.state, LocalState::Loaded);
    assert!(status.worker_pid.is_some());
    // The next transcription reuses that load.
    assert_eq!(echo(&engine, &Hints::default())[2], "loads 1");

    let data = TempDir::new("load-bad");
    let bad = options(data.path(), BAD_TOKENS);
    install_model(&bad.registry.own_dir(&bad.model), BAD_TOKENS);
    let broken = LocalEngine::new(bad);
    let error = broken.load().unwrap_err();
    assert_eq!(error.kind, ErrorKind::LocalMissingModel);
    assert_eq!(broken.status().state, LocalState::Error);

    let empty = TempDir::new("load-missing");
    let without_model = options(empty.path(), GOOD_TOKENS);
    let registry = without_model.registry.clone();
    let missing = LocalEngine::new(without_model);
    assert_eq!(
        missing.load().unwrap_err().kind,
        ErrorKind::LocalMissingModel
    );
    assert_eq!(missing.registry(), registry);
}

fn a_new_model_folder_reloads_in_the_same_worker() {
    let (data, engine) = engine_with_model("folder");
    let before = echo(&engine, &Hints::default());
    let chosen = data.join("chosen");
    install_model(&chosen, GOOD_TOKENS);
    engine.set_registry(
        ModelRegistry::new(data.path())
            .with_ownkey_models(None)
            .with_custom_dir(Some(chosen.clone())),
    );
    let after = echo(&engine, &Hints::default());
    assert_eq!(after[3], before[3]);
    assert_eq!(after[2], "loads 2");
    assert_eq!(engine.status().model_dir, Some(chosen));
}

fn shutdown_refuses_more_work() {
    let (_data, engine) = engine_with_model("shutdown");
    echo(&engine, &Hints::default());
    engine.shutdown();
    assert_eq!(engine.status().worker_pid, None);
    let error = engine.transcribe(&pcm(10), &Hints::default()).unwrap_err();
    assert_eq!(error.message, "Local transcription is shutting down.");
}

fn a_missing_worker_exe_is_named() {
    let data = TempDir::new("no-exe");
    let mut options = options(data.path(), GOOD_TOKENS);
    let missing = data.join("rukoo-voice-local-missing.exe");
    options.worker_exe = Some(missing.clone());
    install_model(&options.registry.own_dir(&options.model), GOOD_TOKENS);
    let engine = LocalEngine::new(options);
    let error = engine
        .transcribe(&pcm(1600), &Hints::default())
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Provider);
    assert_eq!(
        error.message,
        format!(
            "Voice's local speech worker is missing ({}). Reinstall Rukoo Voice.",
            missing.display()
        )
    );
}

fn a_local_failure_never_reaches_the_cloud() {
    let server = FakeServer::always(Reply::json(200, json!({"text": "from the cloud"})));
    let (_data, engine) = engine_with_model("no-fallback");
    let config = ProviderConfig {
        provider: "orukeet".into(),
        api_key: "key".into(),
        endpoint: server.url("/v1/audio/transcriptions"),
        model: "orukeet-onnx-int8".into(),
        ..ProviderConfig::default()
    };
    let stt = match speech_to_text(&config, &engine) {
        Ok(stt) => stt,
        Err(error) => panic!("{error}"),
    };
    assert!(stt.transcribe(&pcm(1600), &words(&["__fail__"])).is_err());
    assert!(stt.transcribe(&pcm(1600), &words(&["__crash__"])).is_err());
    assert!(stt
        .transcribe(&pcm(1600), &Hints::default())
        .unwrap()
        .text
        .starts_with("1600 samples"));
    assert!(server.requests().is_empty(), "local audio went to a server");
}

/// Role "parent": a stand-in Voice that starts a worker, reports its pid and waits to be killed.
fn parent() {
    std::env::set_var(ROLE, "worker");
    let data = PathBuf::from(std::env::var(PARENT_DATA).expect("data folder"));
    let engine = LocalEngine::new(options(&data, GOOD_TOKENS));
    engine
        .transcribe(&pcm(160), &Hints::default())
        .expect("parent transcribe");
    let pid = engine.status().worker_pid.expect("a worker");
    // Leave the worker busy in a long decode, so it won't notice its stdin closing: only the
    // job object can end it when this process dies.
    let busy = engine.clone();
    std::thread::spawn(move || busy.transcribe(&pcm(160), &words(&["__hang__"])));
    std::thread::sleep(Duration::from_millis(300));
    println!("pid {pid}");
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(60));
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
    // SAFETY: signal 0 only checks that the process exists. Voice reaps the workers it started,
    // and a worker orphaned by a killed Voice is reaped by launchd or init.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// Role "watched": a worker that does nothing but watch the parent named in the environment, so
/// the "the app was already gone" rule can be tested without a model or a protocol.
#[cfg(unix)]
fn watched() {
    voice_core::local::parent::watch(|line| eprintln!("{line}"));
    println!("watching");
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

#[cfg(windows)]
fn the_worker_dies_with_voice() {
    use std::process::{Command, Stdio};

    let data = TempDir::new("job");
    let options = options(data.path(), GOOD_TOKENS);
    install_model(&options.registry.own_dir(&options.model), GOOD_TOKENS);
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

/// The same on Unix, where the job object doesn't exist: the worker watches Voice's pid, because
/// the engine named it in the environment. Two seconds is all a worker may outlive its app.
#[cfg(unix)]
fn the_worker_dies_with_voice() {
    use std::process::{Command, Stdio};

    let data = TempDir::new("parent");
    let options = options(data.path(), GOOD_TOKENS);
    install_model(&options.registry.own_dir(&options.model), GOOD_TOKENS);
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
    wait_until(
        Duration::from_secs(2),
        "the worker to end with Voice",
        || !process_alive(pid),
    );
}

/// The pid Voice wrote down is what counts: a worker whose app is already gone when its `main`
/// runs must not start, rather than watch a pid that may belong to something else by now.
#[cfg(unix)]
fn a_worker_whose_parent_is_already_gone_does_not_start() {
    use std::process::{Command, Stdio};
    use voice_core::local::parent::{PARENT_GONE, PARENT_PID};

    // Any process that has certainly ended: a shell that exits at once.
    let mut gone = Command::new("sh").arg("-c").arg("exit 0").spawn().unwrap();
    let gone_pid = gone.id();
    gone.wait().unwrap();

    let started = Instant::now();
    let output = Command::new(std::env::current_exe().unwrap())
        .env(ROLE, "watched")
        .env(PARENT_PID, gone_pid.to_string())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(output.status.code(), Some(PARENT_GONE), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "the worker started watching: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("is gone"),
        "{output:?}"
    );
}

/// The watch leaves a worker whose app is alive alone, so the tests above prove something.
#[cfg(unix)]
fn a_worker_with_a_live_parent_keeps_running() {
    use std::process::{Command, Stdio};
    use voice_core::local::parent::PARENT_PID;

    let mut worker = Command::new(std::env::current_exe().unwrap())
        .env(ROLE, "watched")
        .env(PARENT_PID, std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(worker.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "watching");
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(
        matches!(worker.try_wait(), Ok(None)),
        "the watch ended a worker whose parent is alive"
    );
    worker.kill().unwrap();
    worker.wait().unwrap();
}

#[cfg(windows)]
fn platform_tests() -> Vec<(&'static str, fn())> {
    vec![("the_worker_dies_with_voice", the_worker_dies_with_voice)]
}

#[cfg(unix)]
fn platform_tests() -> Vec<(&'static str, fn())> {
    vec![
        ("the_worker_dies_with_voice", the_worker_dies_with_voice),
        (
            "a_worker_whose_parent_is_already_gone_does_not_start",
            a_worker_whose_parent_is_already_gone_does_not_start,
        ),
        (
            "a_worker_with_a_live_parent_keeps_running",
            a_worker_with_a_live_parent_keeps_running,
        ),
    ]
}

#[cfg(not(any(unix, windows)))]
fn platform_tests() -> Vec<(&'static str, fn())> {
    Vec::new()
}

// The runner ----------------------------------------------------------------------------------

fn run_tests() {
    let mut tests: Vec<(&str, fn())> = vec![
        (
            "loads_lazily_once_and_passes_hotwords",
            loads_lazily_once_and_passes_hotwords,
        ),
        ("timed_results_carry_tokens", timed_results_carry_tokens),
        (
            "a_missing_model_never_starts_the_worker",
            a_missing_model_never_starts_the_worker,
        ),
        (
            "a_damaged_model_is_caught_before_loading",
            a_damaged_model_is_caught_before_loading,
        ),
        ("load_errors_are_reported", load_errors_are_reported),
        (
            "a_crash_is_reported_and_the_next_use_restarts",
            a_crash_is_reported_and_the_next_use_restarts,
        ),
        (
            "a_hung_worker_times_out_and_is_replaced",
            a_hung_worker_times_out_and_is_replaced,
        ),
        (
            "a_failed_decode_keeps_the_worker",
            a_failed_decode_keeps_the_worker,
        ),
        (
            "garbage_from_the_worker_replaces_it",
            garbage_from_the_worker_replaces_it,
        ),
        ("idle_unload_stops_the_worker", idle_unload_stops_the_worker),
        (
            "zero_idle_keeps_the_model_loaded",
            zero_idle_keeps_the_model_loaded,
        ),
        (
            "preload_starts_loading_before_the_audio_arrives",
            preload_starts_loading_before_the_audio_arrives,
        ),
        (
            "load_checks_the_model_before_any_audio",
            load_checks_the_model_before_any_audio,
        ),
        (
            "a_new_model_folder_reloads_in_the_same_worker",
            a_new_model_folder_reloads_in_the_same_worker,
        ),
        ("shutdown_refuses_more_work", shutdown_refuses_more_work),
        (
            "a_missing_worker_exe_is_named",
            a_missing_worker_exe_is_named,
        ),
        (
            "a_local_failure_never_reaches_the_cloud",
            a_local_failure_never_reaches_the_cloud,
        ),
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
