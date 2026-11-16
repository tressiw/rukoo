//! The worker with the real Orukeet model. Runs only where the model is installed:
//! `RUKOO_VOICE_TEST_ORUKEET`, or else the folder Ownkey downloads it to. The model folder is only
//! read; the test checks that its listing doesn't change.
//!
//! Speech comes from `RUKOO_VOICE_TEST_SPEECH`, or else `tmp/voice-spike/audio/en-16k.wav` in the
//! repository: Windows SAPI saying "… The quick brown fox jumps over the lazy dog. Please schedule
//! the meeting with Bart for Thursday at three o'clock." as 16 kHz mono 16-bit PCM.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{listing, log_skip, TempDir, Worker};
use serde_json::{json, Value};

const ORUKEET_REVISION: &str = "55a984d46f68323301837194ce647c702f55facc";

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The model folder, or `None` after printing why `test` skips. The skip log gets a line only
/// when `RUKOO_VOICE_TEST_ORUKEET` names a folder that isn't there: no CI job provides the model,
/// so an unasked-for skip would warn on every run and say nothing new.
fn model_dir(test: &str) -> Option<PathBuf> {
    if let Some(dir) = env_path("RUKOO_VOICE_TEST_ORUKEET") {
        if dir.is_dir() {
            return Some(dir);
        }
        eprintln!("skipped: no Orukeet model at {}", dir.display());
        log_skip(
            test,
            &format!(
                "RUKOO_VOICE_TEST_ORUKEET names {}, which isn't a folder",
                dir.display()
            ),
        );
        return None;
    }
    let Some(local) = env_path("LOCALAPPDATA") else {
        eprintln!("skipped: set RUKOO_VOICE_TEST_ORUKEET to Orukeet's model folder");
        return None;
    };
    let dir = local
        .join("Ownkey")
        .join("models")
        .join("orukeet")
        .join(ORUKEET_REVISION);
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skipped: no Orukeet model at {}", dir.display());
        None
    }
}

/// The speech to transcribe, or `None` after printing and logging why `test` skips: with the
/// model there, the test was meant to run.
fn speech_file(test: &str) -> Option<PathBuf> {
    let file = env_path("RUKOO_VOICE_TEST_SPEECH").unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tmp/voice-spike/audio/en-16k.wav")
    });
    if file.is_file() {
        Some(file)
    } else {
        eprintln!("skipped: no test speech at {}", file.display());
        log_skip(test, &format!("no test speech at {}", file.display()));
        None
    }
}

/// The samples of a 16 kHz mono 16-bit PCM WAV file.
fn read_wav(path: &Path) -> Vec<i16> {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " {
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let bits = u16::from_le_bytes([body[14], body[15]]);
            format = Some((u16::from_le_bytes([body[0], body[1]]), channels, rate, bits));
        }
        if id == b"data" {
            assert_eq!(
                format,
                Some((1, 1, 16_000, 16)),
                "{} must be 16 kHz mono PCM",
                path.display()
            );
            return body
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect();
        }
        at += 8 + size + size % 2;
    }
    panic!("{} has no data chunk", path.display());
}

fn bytes(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

fn transcribe(worker: &mut Worker, id: u64, pcm: &[i16], extra: Value) -> Value {
    let mut request = json!({"op": "transcribe", "id": id, "rate": 16000, "bytes": pcm.len() * 2});
    for (key, value) in extra.as_object().unwrap() {
        request[key] = value.clone();
    }
    let response = worker.request_with(&request, &bytes(pcm));
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["id"], json!(id));
    response
}

fn text(response: &Value) -> String {
    response["text"].as_str().unwrap().to_lowercase()
}

/// Tokens, timestamps and durations line up, and time only moves forward.
fn check_timing(response: &Value, seconds: f64) {
    let tokens = response["tokens"].as_array().unwrap();
    let timestamps: Vec<f64> = response["timestamps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_f64().unwrap())
        .collect();
    assert!(!tokens.is_empty());
    assert_eq!(tokens.len(), timestamps.len());
    assert_eq!(
        tokens.len(),
        response["durations"].as_array().unwrap().len()
    );
    assert!(
        timestamps.windows(2).all(|pair| pair[0] <= pair[1]),
        "{timestamps:?}"
    );
    assert!(*timestamps.last().unwrap() < seconds);
}

#[test]
fn transcribes_speech_and_leaves_the_model_folder_alone() {
    const TEST: &str = "transcribes_speech_and_leaves_the_model_folder_alone";
    let Some(dir) = model_dir(TEST) else {
        return;
    };
    let Some(speech) = speech_file(TEST) else {
        return;
    };
    let before = listing(&dir);
    let clip = read_wav(&speech);
    let clip_seconds = clip.len() as f64 / 16_000.0;

    let mut worker = Worker::spawn();
    let load = json!({"op": "load", "model": "orukeet", "dir": dir, "threads": 4});
    let response = worker.request(&load);
    assert_eq!(response["ok"], json!(true), "{response}");
    let load_ms = response["loadMs"].as_u64().unwrap();
    assert!(load_ms > 0);
    eprintln!("load: {load_ms} ms");
    assert_eq!(worker.request(&load), json!({"ok": true, "loadMs": 0}));

    let response = transcribe(&mut worker, 1, &clip, json!({}));
    eprintln!(
        "decode {clip_seconds:.2} s: {} ms: {}",
        response["ms"], response["text"]
    );
    assert!(text(&response).contains("quick brown fox"), "{response}");
    assert!(text(&response).contains("thursday"), "{response}");
    check_timing(&response, clip_seconds);

    let response = transcribe(
        &mut worker,
        2,
        &clip,
        json!({"hotwords": ["Bart", "Rukoo"]}),
    );
    eprintln!("decode with hotwords: {} ms", response["ms"]);
    assert!(text(&response).contains("quick brown fox"), "{response}");
    assert!(text(&response).contains("thursday"), "{response}");

    // Over 30 s: cut at pauses and decoded window by window, as Ownkey does past 15 s.
    let long: Vec<i16> = clip.repeat(3);
    let long_seconds = 3.0 * clip_seconds;
    let response = transcribe(&mut worker, 3, &long, json!({}));
    eprintln!(
        "decode {long_seconds:.2} s split: {} ms: {}",
        response["ms"], response["text"]
    );
    assert_eq!(
        text(&response).matches("quick brown fox").count(),
        3,
        "{response}"
    );
    assert_eq!(text(&response).matches("thursday").count(), 3, "{response}");
    check_timing(&response, long_seconds);
    let last = response["timestamps"].as_array().unwrap().last().unwrap();
    assert!(last.as_f64().unwrap() > 2.0 * clip_seconds, "{last}");

    let response = transcribe(&mut worker, 4, &long, json!({"split": false}));
    eprintln!("decode {long_seconds:.2} s whole: {} ms", response["ms"]);
    assert_eq!(
        text(&response).matches("quick brown fox").count(),
        3,
        "{response}"
    );
    check_timing(&response, long_seconds);

    let response = transcribe(&mut worker, 5, &[], json!({}));
    assert_eq!(response["text"], json!(""));

    assert_eq!(
        worker.request(&json!({"op": "unload"})),
        json!({"ok": true})
    );
    let response = worker.request_with(&json!({"op": "transcribe", "bytes": 2}), &[0, 0]);
    assert_eq!(response["kind"], json!("not_loaded"));

    worker.send(&json!({"op": "quit"}));
    assert!(worker.wait().success());
    // Logs written after the load's stderr capture still arrive.
    let stderr = worker.stderr();
    assert!(stderr.contains("loaded Orukeet"), "{stderr}");
    assert!(stderr.contains("unloaded Orukeet"), "{stderr}");
    assert_eq!(listing(&dir), before, "the model folder changed");
}

/// sherpa-onnx ends the process when the encoder has no NeMo metadata, so the worker checks first.
#[test]
fn an_encoder_without_nemo_metadata_is_bad_model() {
    let Some(dir) = model_dir("an_encoder_without_nemo_metadata_is_bad_model") else {
        return;
    };
    let temp = TempDir::new("no-metadata");
    for name in [
        "decoder.int8.onnx",
        "joiner.int8.onnx",
        "tokens.txt",
        "bpe.vocab",
    ] {
        fs::copy(dir.join(name), temp.path().join(name)).unwrap();
    }
    fs::copy(
        dir.join("decoder.int8.onnx"),
        temp.path().join("encoder.int8.onnx"),
    )
    .unwrap();

    let mut worker = Worker::spawn();
    let response = worker.request(&json!({"op": "load", "model": "orukeet", "dir": temp.path()}));
    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(response["kind"], json!("bad_model"), "{response}");
    let error = response["error"].as_str().unwrap();
    assert!(error.contains("encoder.int8.onnx"), "{error}");
    assert!(error.contains("vocab_size"), "{error}");
    assert_eq!(
        worker.request(&json!({"op": "ping"})),
        json!({"ok": true, "version": "0.1.0"})
    );
    worker.send(&json!({"op": "quit"}));
    assert!(worker.wait().success());
}
