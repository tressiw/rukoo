//! The worker with the real NeMo-Speech.cpp runtime and Nemotron 3 Diarization, on the AMI
//! meeting excerpt. Skips unless all three variables are set, as `voice/scripts/speaker-assets.ps1`
//! on Windows and `voice/scripts/speaker-assets.mjs` on macOS print them:
//!
//! - `RUKOO_VOICE_TEST_NEMO_DIR`: the runtime folder, holding the C ABI library
//!   (`nemo_speech_asr_c.dll` on Windows, `libnemo_speech_asr_c.dylib` on macOS).
//! - `RUKOO_VOICE_TEST_NEMOTRON`: `Nemotron-3-Diarization.q8_0.gguf`, or the folder holding it.
//! - `RUKOO_VOICE_TEST_AMI`: the folder holding `ami_en2002d_2132.wav`, or the WAV itself.

mod common;

#[cfg(windows)]
use std::collections::BTreeSet;
use std::fs;
#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, PoisonError};

use common::{assert_failure, probs, TempDir, Worker, EXE};
use serde_json::{json, Value};

const MODEL_FILE: &str = "Nemotron-3-Diarization.q8_0.gguf";
const AMI_WAV: &str = "ami_en2002d_2132.wav";
/// The excerpt is 60 s, so 6,000 frames of 10 ms.
const AMI_FRAMES: i64 = 6_000;
/// One chunk of the live geometry: 38 encoder frames of 80 ms.
const CHUNK: i64 = 38 * 8;
const SPEAKERS: usize = 8;

/// One test with the runtime at a time. NeMo-Speech.cpp runs four compute threads that wait
/// for each other at every step, so two workers on a four-core CI runner starve each other: the
/// self-test ran at 0.16 times real time next to another worker, against 6.8 alone.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static RUNTIME: Mutex<()> = Mutex::new(());
    RUNTIME.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Assets {
    runtime: PathBuf,
    model: PathBuf,
    wav: PathBuf,
}

fn file_in(variable: &str, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os(variable)?);
    Some(if path.is_dir() { path.join(name) } else { path })
}

fn assets() -> Option<Assets> {
    let found = (|| {
        Some(Assets {
            runtime: PathBuf::from(std::env::var_os("RUKOO_VOICE_TEST_NEMO_DIR")?),
            model: file_in("RUKOO_VOICE_TEST_NEMOTRON", MODEL_FILE)?,
            wav: file_in("RUKOO_VOICE_TEST_AMI", AMI_WAV)?,
        })
    })();
    if found.is_none() {
        eprintln!(
            "skipped: set RUKOO_VOICE_TEST_NEMO_DIR, RUKOO_VOICE_TEST_NEMOTRON and RUKOO_VOICE_TEST_AMI (voice/scripts/speaker-assets.ps1, or speaker-assets.mjs on macOS)"
        );
    }
    found
}

fn load(assets: &Assets) -> Value {
    json!({"op": "load", "id": 2, "runtime": assets.runtime, "model": assets.model,
           "preset": "v3-streaming", "chunkFrames": 38, "rightContextFrames": 1})
}

/// The excerpt's samples as raw s16le bytes, after the canonical 44-byte header.
fn ami_pcm(assets: &Assets) -> Vec<u8> {
    let bytes = fs::read(&assets.wav).expect("read the AMI excerpt");
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[36..40], b"data");
    bytes[44..].to_vec()
}

fn check_frames(response: &Value, expected_first: i64) -> i64 {
    assert_eq!(response["ok"], json!(true), "{response}");
    let first = response["first"].as_i64().unwrap();
    let frames = response["frames"].as_i64().unwrap();
    let count = response["count"].as_i64().unwrap();
    assert_eq!(first, expected_first, "{response}");
    assert_eq!(first + frames, count, "{response}");
    let values = probs(response);
    assert_eq!(values.len(), frames as usize * SPEAKERS);
    assert!(
        values.iter().all(|p| (0.0..=1.0).contains(p)),
        "probabilities outside [0, 1]"
    );
    count
}

#[test]
fn labels_the_ami_excerpt_in_one_second_pushes() {
    let Some(assets) = assets() else { return };
    let _one = one_at_a_time();
    let mut worker = Worker::spawn();
    let loaded = worker.request(&load(&assets));
    assert_eq!(loaded["ok"], json!(true), "{loaded}\n{}", worker.stderr());
    assert_eq!(loaded["speakers"], json!(SPEAKERS));
    assert_eq!(loaded["secondsPerFrame"], json!(0.01));
    assert!(loaded["loadMs"].is_u64());
    assert_eq!(
        worker.request(&json!({"op": "open", "id": 3})),
        json!({"id": 3, "ok": true})
    );

    let pcm = ami_pcm(&assets);
    let mut count = 0;
    let mut speech = [0f32; SPEAKERS];
    for (n, second) in pcm.chunks(32_000).enumerate() {
        let response = worker.request_with(
            &json!({"op": "push", "id": 10 + n, "rate": 16000, "bytes": second.len()}),
            second,
        );
        let values = probs(&response);
        for frame in values.chunks_exact(SPEAKERS) {
            for (speaker, p) in frame.iter().enumerate() {
                if *p > 0.5 {
                    speech[speaker] += 0.01;
                }
            }
        }
        count = check_frames(&response, count);
    }
    // The newest audio waits for its chunk: frames trail the audio pushed so far.
    assert!(count < AMI_FRAMES, "{count}");
    let tail = worker.request(&json!({"op": "finish", "id": 99}));
    let total = check_frames(&tail, count);
    assert!(
        (total - AMI_FRAMES).abs() <= CHUNK,
        "{total} frames for 60 s of audio"
    );
    for frame in probs(&tail).chunks_exact(SPEAKERS) {
        for (speaker, p) in frame.iter().enumerate() {
            if *p > 0.5 {
                speech[speaker] += 0.01;
            }
        }
    }
    // Three people take turns in the excerpt; the model should hear at least three voices for
    // a few seconds each.
    let voices = speech.iter().filter(|seconds| **seconds >= 2.0).count();
    assert!(voices >= 3, "speech per speaker slot: {speech:?}");

    // The stream is finished; a new one starts from frame 0.
    assert_failure(
        &worker.request_with(&json!({"op": "push", "bytes": 2}), &[0, 0]),
        "not_open",
    );
    assert_eq!(worker.request(&json!({"op": "open"}))["ok"], json!(true));
    let again = worker.request_with(
        &json!({"op": "push", "id": 100, "bytes": 320_000}),
        &pcm[..320_000],
    );
    check_frames(&again, 0);
    worker.send(&json!({"op": "quit"}));
    assert_eq!(worker.wait().code(), Some(0));
}

#[test]
fn a_second_load_is_refused() {
    let Some(assets) = assets() else { return };
    let _one = one_at_a_time();
    let mut worker = Worker::spawn();
    assert_eq!(worker.request(&load(&assets))["ok"], json!(true));
    assert_failure(&worker.request(&load(&assets)), "bad_request");
}

#[test]
fn a_bad_model_is_refused_without_crashing() {
    let Some(assets) = assets() else { return };
    let _one = one_at_a_time();
    let temp = TempDir::new("model");
    let mut worker = Worker::spawn();

    let missing = worker.request(&json!({"op": "load", "runtime": assets.runtime,
                                         "model": temp.path().join(MODEL_FILE)}));
    assert_failure(&missing, "model");

    let garbage = temp.path().join("garbage.gguf");
    fs::write(&garbage, vec![0x47u8; 4096]).unwrap();
    let refused =
        worker.request(&json!({"op": "load", "runtime": assets.runtime, "model": garbage}));
    assert_failure(&refused, "model");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .starts_with("Speaker labels failed while loading the model."),
        "{refused}"
    );

    // The worker is still there, and the real model loads after the refusals.
    assert_eq!(worker.request(&json!({"op": "ping"}))["ok"], json!(true));
    assert_eq!(worker.request(&load(&assets))["ok"], json!(true));
}

#[test]
fn the_self_test_hears_at_least_three_speakers() {
    let Some(assets) = assets() else { return };
    let _one = one_at_a_time();
    let temp = TempDir::new("report");
    let report_path = temp.path().join("report.json");
    let output = Command::new(EXE)
        .arg("--self-test")
        .arg("--runtime")
        .arg(&assets.runtime)
        .arg("--model")
        .arg(&assets.model)
        .arg("--wav")
        .arg(&assets.wav)
        .arg("--report")
        .arg(&report_path)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        report,
        serde_json::from_slice::<Value>(&fs::read(&report_path).unwrap()).unwrap()
    );
    assert!(report["speakers"].as_u64().unwrap() >= 3, "{report}");
    assert!(
        report["frames"].as_i64().unwrap() >= AMI_FRAMES - 100,
        "{report}"
    );
    assert!(report["segments"].as_u64().unwrap() >= 3, "{report}");
    assert!(report["realtimeFactor"].as_f64().unwrap() > 1.0, "{report}");
    eprintln!("self-test: {report}");
}

/// A copy of every system DLL the runtime imports, planted in the runtime folder where the loader
/// looks first for the runtime's imports, must not be the copy that loads.
#[cfg(windows)]
#[test]
fn the_runtime_takes_its_system_dlls_from_system32_even_with_copies_beside_it() {
    let Some(assets) = assets() else { return };
    let _one = one_at_a_time();
    let temp = TempDir::new("planted");
    let runtime = temp.path().join("nemo_speech");
    fs::create_dir(&runtime).unwrap();
    let mut shipped = BTreeSet::new();
    for entry in fs::read_dir(&assets.runtime).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            fs::copy(&path, runtime.join(path.file_name().unwrap())).unwrap();
            let name = lower_file_name(&path);
            if name.ends_with(".dll") {
                shipped.insert(name);
            }
        }
    }
    assert!(shipped.contains("nemo_speech_asr.dll"), "{shipped:?}");

    // What the runtime imports and doesn't ship. The api-ms-win-crt API sets resolve to ucrtbase.dll.
    let mut system_dlls = BTreeSet::new();
    for name in &shipped {
        for import in pe_imports(&runtime.join(name)) {
            if import.starts_with("api-ms-win-crt-") {
                system_dlls.insert("ucrtbase.dll".to_owned());
            } else if !shipped.contains(&import) {
                system_dlls.insert(import);
            }
        }
    }
    // nemo_speech_asr.dll imports dbghelp.dll, which is no KnownDLL, so only the worker keeps the
    // loader from taking a dbghelp.dll from the runtime folder.
    assert!(system_dlls.contains("dbghelp.dll"), "{system_dlls:?}");
    let system32 = system_dir();
    for name in &system_dlls {
        fs::copy(system32.join(name), runtime.join(name))
            .unwrap_or_else(|error| panic!("copy {name} from {}: {error}", system32.display()));
    }

    let mut worker = Worker::spawn();
    let mut request = load(&assets);
    request["runtime"] = json!(runtime);
    let loaded = worker.request(&request);
    assert_eq!(loaded["ok"], json!(true), "{loaded}\n{}", worker.stderr());
    let modules = loaded_modules(worker.pid());
    let folder_of = |name: &str| {
        modules
            .iter()
            .find(|path| lower_file_name(path) == name)
            .map(|path| canonical_text(path.parent().unwrap()))
    };
    // The worker loaded the copied runtime, not the original.
    assert_eq!(
        folder_of("nemo_speech_asr.dll"),
        Some(canonical_text(&runtime)),
        "{modules:#?}"
    );
    for name in &system_dlls {
        assert_eq!(
            folder_of(name),
            Some(canonical_text(&system32)),
            "{name} loaded from the wrong folder: {modules:#?}"
        );
    }
    worker.send(&json!({"op": "quit"}));
    assert_eq!(worker.wait().code(), Some(0));
}

#[cfg(windows)]
fn lower_file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().to_lowercase()
}

/// A folder as one comparable string: the long, real path in lowercase.
#[cfg(windows)]
fn canonical_text(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|error| panic!("canonicalize {}: {error}", path.display()))
        .to_string_lossy()
        .to_lowercase()
}

#[cfg(windows)]
fn system_dir() -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = [0u16; 260];
    let len = unsafe {
        windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut buffer))
    } as usize;
    assert!(len > 0 && len < buffer.len(), "GetSystemDirectoryW");
    PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len]))
}

/// The full path of every module loaded in process `pid`.
#[cfg(windows)]
fn loaded_modules(pid: u32) -> Vec<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W, TH32CS_SNAPMODULE,
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid) }
        .expect("list the worker's modules");
    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut paths = Vec::new();
    let mut found = unsafe { Module32FirstW(snapshot, &mut entry) };
    while found.is_ok() {
        let path = &entry.szExePath;
        let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        paths.push(PathBuf::from(std::ffi::OsString::from_wide(&path[..len])));
        found = unsafe { Module32NextW(snapshot, &mut entry) };
    }
    unsafe { CloseHandle(snapshot) }.unwrap();
    assert!(!paths.is_empty(), "no modules in process {pid}");
    paths
}

/// The DLLs a 64-bit PE file imports, directly and delay-loaded, in lowercase.
#[cfg(windows)]
fn pe_imports(file: &Path) -> Vec<String> {
    let pe = fs::read(file).unwrap();
    let u16_at = |at: usize| u16::from_le_bytes([pe[at], pe[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(pe[at..at + 4].try_into().unwrap());
    assert_eq!(&pe[..2], b"MZ", "{}", file.display());
    let nt = u32_at(0x3c) as usize;
    assert_eq!(&pe[nt..nt + 4], b"PE\0\0", "{}", file.display());
    let sections = u16_at(nt + 6) as usize;
    let optional = nt + 24;
    assert_eq!(u16_at(optional), 0x20b, "{} is not PE32+", file.display());
    let section_table = optional + u16_at(nt + 20) as usize;
    let offset = |rva: u32| {
        (0..sections)
            .map(|n| section_table + n * 40)
            .find_map(|section| {
                let start = u32_at(section + 12);
                let size = u32_at(section + 8).max(u32_at(section + 16));
                (start..start + size)
                    .contains(&rva)
                    .then(|| (rva - start + u32_at(section + 20)) as usize)
            })
            .unwrap_or_else(|| panic!("RVA {rva:#x} outside the sections of {}", file.display()))
    };
    let name_at = |at: usize| {
        let end = pe[at..].iter().position(|&b| b == 0).unwrap();
        String::from_utf8_lossy(&pe[at..at + end]).to_lowercase()
    };
    let mut names = Vec::new();
    // Data directory 1 holds 20-byte import descriptors with the name at 12, directory 13
    // 32-byte delay-load descriptors with the name at 4. A zero name ends each list.
    for (directory, descriptor, name) in [(1, 20, 12), (13, 32, 4)] {
        let rva = u32_at(optional + 112 + directory * 8);
        if rva == 0 {
            continue;
        }
        let mut at = offset(rva);
        while u32_at(at + name) != 0 {
            names.push(name_at(offset(u32_at(at + name))));
            at += descriptor;
        }
    }
    names
}
