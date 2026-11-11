//! The worker protocol, without the runtime or the model.

mod common;

use std::fs;
use std::process::Command;

use common::{assert_failure, TempDir, Worker, EXE};
use serde_json::{json, Value};

/// The processor of the machine running the tests can't run the runtime; loads answer `cpu`.
fn processor_refused() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        !(std::arch::is_x86_feature_detected!("avx2")
            && std::arch::is_x86_feature_detected!("fma")
            && std::arch::is_x86_feature_detected!("f16c")
            && std::arch::is_x86_feature_detected!("bmi2"))
    }
    // Every Apple Silicon Mac has the arm64 runtime's baseline.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        false
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        all(target_os = "macos", target_arch = "aarch64")
    )))]
    {
        true
    }
}

/// The C ABI library of this platform's runtime.
fn library_name() -> &'static str {
    if cfg!(windows) {
        "nemo_speech_asr_c.dll"
    } else {
        "libnemo_speech_asr_c.dylib"
    }
}

fn assert_alive(worker: &mut Worker) {
    let pong = worker.request(&json!({"op": "ping"}));
    assert_eq!(pong["ok"], json!(true), "{pong}");
}

fn load(runtime: &std::path::Path) -> Value {
    json!({"op": "load", "id": 2, "runtime": runtime, "model": runtime.join("model.gguf"),
           "preset": "v3-streaming", "chunkFrames": 38, "rightContextFrames": 1})
}

#[test]
fn ping_answers_with_the_pid_and_the_processor() {
    let mut worker = Worker::spawn();
    let pong = worker.request(&json!({"op": "ping", "id": 1}));
    assert_eq!(pong["id"], json!(1));
    assert_eq!(pong["ok"], json!(true));
    assert_eq!(pong["pid"], json!(worker.pid()));
    assert_eq!(pong["version"], json!(env!("CARGO_PKG_VERSION")));
    for feature in ["avx2", "fma", "f16c", "bmi2", "neon", "dotprod", "fp16"] {
        assert!(pong["cpu"][feature].is_boolean(), "{pong}");
    }
}

#[test]
fn load_from_a_missing_folder_is_a_missing_runtime() {
    let temp = TempDir::new("missing");
    let mut worker = Worker::spawn();
    let response = worker.request(&load(&temp.path().join("nemo_speech")));
    assert_eq!(response["id"], json!(2));
    if processor_refused() {
        assert_failure(&response, "cpu");
        return;
    }
    assert_failure(&response, "runtime");
    // Elsewhere the worker refuses before it looks for the runtime.
    let supported = cfg!(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    ));
    let expected = if supported {
        library_name()
    } else {
        "Apple Silicon"
    };
    assert!(
        response["error"].as_str().unwrap().contains(expected),
        "{response}"
    );
    assert_alive(&mut worker);
    // Nothing loaded, so there is nothing to open.
    assert_failure(&worker.request(&json!({"op": "open"})), "bad_request");
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
#[test]
fn a_damaged_runtime_library_fails_the_load_and_the_worker_lives_on() {
    if processor_refused() {
        return;
    }
    let temp = TempDir::new("damaged");
    fs::write(
        temp.path().join(library_name()),
        b"MZ\x90\0 this is not a library",
    )
    .unwrap();
    let mut worker = Worker::spawn();
    let response = worker.request(&load(temp.path()));
    assert_failure(&response, "runtime");
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .contains("could not load"),
        "{response}"
    );
    assert_alive(&mut worker);
}

#[test]
fn push_and_finish_before_open_are_not_open() {
    let mut worker = Worker::spawn();
    let response = worker.request_with(
        &json!({"op": "push", "id": 4, "rate": 16000, "bytes": 3200}),
        &[0; 3200],
    );
    assert_failure(&response, "not_open");
    assert_eq!(response["id"], json!(4));
    assert_failure(
        &worker.request(&json!({"op": "finish", "id": 5})),
        "not_open",
    );
    assert_eq!(
        worker.request(&json!({"op": "close", "id": 6})),
        json!({"id": 6, "ok": true})
    );
}

#[test]
fn a_payload_over_ten_seconds_is_refused_and_the_next_request_works() {
    let mut worker = Worker::spawn();
    let bytes = 320_002;
    let response = worker.request_with(
        &json!({"op": "push", "id": 7, "rate": 16000, "bytes": bytes}),
        &vec![0; bytes],
    );
    assert_failure(&response, "bad_request");
    assert_eq!(response["id"], json!(7));
    assert!(response["error"]
        .as_str()
        .unwrap()
        .contains("longer than 10 seconds"));
    assert_alive(&mut worker);

    // Exactly 10 s is a well-formed push; without a stream it is not_open.
    let response = worker.request_with(
        &json!({"op": "push", "id": 8, "bytes": 320_000}),
        &vec![0; 320_000],
    );
    assert_failure(&response, "not_open");
}

#[test]
fn malformed_lines_get_bad_request_and_the_worker_reads_on() {
    let mut worker = Worker::spawn();
    worker.send_raw(b"not json\n[1,2]\n{}\n\n{\"op\":\"nope\",\"id\":5}\r\n");
    for _ in 0..3 {
        assert_failure(&worker.recv(), "bad_request");
    }
    let unknown = worker.recv();
    assert_failure(&unknown, "bad_request");
    assert_eq!(unknown["id"], json!(5));

    worker.send_raw(&vec![b'x'; 100_000]);
    worker.send_raw(b"\n");
    assert_failure(&worker.recv(), "bad_request");

    let response = worker.request_with(&json!({"op": "push", "id": 9, "bytes": 3}), &[1, 2, 3]);
    assert_failure(&response, "bad_request");
    assert_failure(
        &worker.request(&json!({"op": "load", "runtime": "C:\\x"})),
        "bad_request",
    );
    assert_alive(&mut worker);
}

#[test]
fn quit_exits_cleanly() {
    let mut worker = Worker::spawn();
    assert_alive(&mut worker);
    worker.send(&json!({"op": "quit"}));
    assert_eq!(worker.wait().code(), Some(0));
}

#[test]
fn closing_stdin_exits_cleanly_also_inside_a_payload() {
    let mut worker = Worker::spawn();
    worker.close_stdin();
    assert_eq!(worker.wait().code(), Some(0));

    let mut worker = Worker::spawn();
    worker.send_with(&json!({"op": "push", "bytes": 100}), &[0; 10]);
    worker.close_stdin();
    assert_eq!(worker.wait().code(), Some(0));
    assert!(
        worker
            .stderr()
            .contains("stdin closed after 10 of 100 payload bytes"),
        "{}",
        worker.stderr()
    );
}

#[test]
fn version_and_bad_arguments() {
    let output = Command::new(EXE).arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("rukoo-voice-speakers {}", env!("CARGO_PKG_VERSION"))
    );

    let output = Command::new(EXE)
        .args(["--self-test", "--runtime", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--model is missing"));

    let output = Command::new(EXE).arg("--frobnicate").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_self_test_without_the_runtime_fails_with_a_reason() {
    let temp = TempDir::new("selftest");
    let wav = temp.path().join("silence.wav");
    let mut bytes = b"RIFF\x24\x7d\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0data\0\x7d\0\0".to_vec();
    bytes.resize(44 + 32_000, 0);
    fs::write(&wav, bytes).unwrap();
    let output = Command::new(EXE)
        .arg("--self-test")
        .arg("--runtime")
        .arg(temp.path().join("nowhere"))
        .arg("--model")
        .arg(temp.path().join("model.gguf"))
        .arg("--wav")
        .arg(&wav)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("self-test failed"), "{stderr}");
}
