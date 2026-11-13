//! `rukoo-voice-speakers`: turns call audio into speaker frame probabilities with NeMo-Speech.cpp,
//! in a process of its own, so a crash in the native runtime never takes the app down.
//!
//! The app's speaker client (voice-core `speakers::client`) starts one worker per labelled
//! meeting stream, in its kill-on-close job on Windows and watching its parent on Unix, at
//! below-normal priority. The worker only runs NVIDIA Nemotron 3 Diarization; turning the
//! probabilities into speaker turns and stable `call-N` ids happens in the app. It reads the
//! runtime folder and the model file it is given and writes nothing to disk.
//!
//! # Protocol
//!
//! The framing is voice-local's: one JSON request per line on stdin, a request that names `bytes`
//! followed by that many raw bytes, one JSON line per request on stdout in order, except `quit`.
//! Logs and the runtime's own output go to stderr. The worker exits 0 on `quit` and when stdin
//! closes, and 70 on Unix when the app that started it is gone.
//!
//! | Request | Response |
//! |---|---|
//! | `{"op":"ping","id":1}` | `{"id":1,"ok":true,"pid":4242,"version":"0.1.0","cpu":{"avx2":true,"fma":true,"f16c":true,"bmi2":true}}` |
//! | `{"op":"load","id":2,"runtime":"C:\\…\\nemo_speech","model":"C:\\…\\Nemotron-3-Diarization.q8_0.gguf","preset":"v3-streaming","chunkFrames":38,"rightContextFrames":1}` | `{"id":2,"ok":true,"speakers":8,"secondsPerFrame":0.01,"loadMs":812,"runtimeVersion":"…"}` |
//! | `{"op":"open","id":3}` | `{"id":3,"ok":true}` |
//! | `{"op":"push","id":4,"rate":16000,"bytes":32000}` + 32000 bytes of s16le mono | `{"id":4,"ok":true,"first":0,"frames":304,"count":304,"probs":"<base64>"}` |
//! | `{"op":"finish","id":5}` | the shape of `push`, with the frames the tail produced |
//! | `{"op":"close","id":6}` | `{"id":6,"ok":true}` |
//! | `{"op":"quit"}` | exits 0 |
//!
//! - `load` is valid once per process; a failed one may be repeated. `preset`, `chunkFrames` and
//!   `rightContextFrames` are optional and default to the live geometry above (0 keeps the
//!   preset's value). `runtimeVersion` is there when the runtime exports its version.
//! - `open` closes any previous stream. `rate` is optional and must be 16000. A push carries at
//!   most 10 s of audio.
//! - `probs` is base64 of `frames × speakers` little-endian f32 values, frame-major, for frames
//!   `first` to `first + frames`. `first` continues the previous reply's `count`, which is the
//!   stream's frame count after the call.
//! - A failure is `{"ok":false,"kind":…,"error":"<English message>"}` with `kind` one of
//!   `bad_request`, `cpu` (the processor lacks AVX2, FMA, F16C or BMI2; nothing was loaded),
//!   `runtime` (the folder, the DLL or an export is missing, or the DLL didn't load), `model` (the
//!   runtime refused the model), `not_open` (`push` or `finish` without an open stream),
//!   `behind` (the runtime dropped frames the worker hadn't returned) or `failed`. `error`
//!   carries `nemo_speech_asr_last_error()` when the runtime gave one.
//!
//! `--self-test --runtime <dir> --model <gguf> --wav <16 kHz mono wav> [--report <json>]` labels
//! the file in 1 s pushes and prints `{"speakers":n,"frames":f,"segments":s,"loadMs":l,
//! "realtimeFactor":r}`, with `speakers` counted from the runtime's own segments. `--version`
//! prints the version.

mod backend;
mod cpu;
#[cfg(windows)]
mod nemo;
// The watch Voice uses to end this worker with itself lives in voice-core, next to the client that
// spawns it. Included by path rather than as a dependency: a worker that a native runtime takes
// down shouldn't link the app's network and archive stacks for a `kqueue` and `pidfd` watch.
#[path = "../../voice-core/src/local/parent.rs"]
mod parent;
mod protocol;
mod selftest;
mod stdio;
mod worker;

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(windows)]
    quiet_crashes();
    // Before the arguments are read: when the app that started this worker is gone, so is the
    // worker. A worker started by hand has no parent to watch.
    parent::watch(worker::log);
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        None => serve(),
        Some("--version") => {
            println!("rukoo-voice-speakers {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--self-test") => self_test(&args[1..]),
        Some(_) => {
            eprintln!(
                "rukoo-voice-speakers: unknown arguments. Run it without arguments as the worker, or as\n  {}",
                selftest::USAGE
            );
            ExitCode::from(2)
        }
    }
}

/// A crash in the native runtime ends the process at once, without Windows' error dialog, so the
/// app sees it exit instead of waiting on a window nobody can see.
#[cfg(windows)]
fn quiet_crashes() {
    use windows::Win32::System::Diagnostics::Debug::{
        SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
    };
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
    }
}

fn serve() -> ExitCode {
    // Before anything native runs: the runtime must never write into the protocol.
    let mut output = match stdio::claim_stdout() {
        Ok(output) => output,
        Err(error) => {
            worker::log(&format!("could not set up stdout: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let mut worker = worker::Worker::new(backend::Native::default());
    let result = worker::serve(&mut io::stdin().lock(), &mut output, &mut worker);
    worker.close();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            worker::log(&format!("stopped: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn self_test(args: &[OsString]) -> ExitCode {
    let options = match selftest::parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("rukoo-voice-speakers: {error}\nusage: {}", selftest::USAGE);
            return ExitCode::from(2);
        }
    };
    let mut output = match stdio::claim_stdout() {
        Ok(output) => output,
        Err(error) => {
            worker::log(&format!("could not set up stdout: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let report = match selftest::run(&options) {
        Ok(report) => report,
        Err(error) => {
            worker::log(&format!("self-test failed: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let mut line = serde_json::to_vec(&report).expect("a report always serializes");
    line.push(b'\n');
    if let Some(path) = &options.report {
        if let Err(error) = std::fs::write(path, &line) {
            worker::log(&format!("could not write {}: {error}", path.display()));
            return ExitCode::FAILURE;
        }
    }
    if output.write_all(&line).is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
