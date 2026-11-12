//! `rukoo-voice-local`: Rukoo Voice's local speech worker.
//!
//! The worker runs Orukeet (sherpa-onnx) in its own process, so a crashing or memory-hungry model
//! can't take dictation down, and unloading gives the memory back to the system. It only reads
//! the model folder it is given; it never downloads, hashes or writes model files.
//!
//! # Protocol
//!
//! Requests arrive on stdin, one JSON object per line. A request that names `bytes` is followed by
//! exactly that many raw bytes. Every request gets one JSON line on stdout, in order; `quit` gets
//! none. Logs and sherpa-onnx's own output go to stderr. The worker exits 0 on `quit` and when
//! stdin closes, and 70 on Unix when the app that started it is gone.
//!
//! | Request | Response |
//! |---|---|
//! | `{"op":"ping"}` | `{"ok":true,"version":"0.1.0"}` |
//! | `{"op":"load","model":"orukeet","dir":"…","threads":4}` | `{"ok":true,"loadMs":1700}` |
//! | `{"op":"transcribe","id":7,"rate":16000,"bytes":n,"hotwords":["Bart"]}` + n bytes | `{"ok":true,"id":7,"text":"…","tokens":[…],"timestamps":[…],"durations":[…],"ms":450}` |
//! | `{"op":"unload"}` | `{"ok":true}` |
//! | `{"op":"quit"}` | exits 0 |
//!
//! - `threads` is optional (4). Loading the model that is already loaded answers `loadMs: 0`;
//!   loading another one unloads the first.
//! - `transcribe` takes 16 kHz mono s16le audio; `rate` is optional and must be 16000. Audio over
//!   15 s is cut at pauses and decoded window by window, as Ownkey does for dictation; send
//!   `"split": false` for audio the caller has already cut, such as meeting windows. Times are in
//!   seconds from the start of the audio. `hotwords` is optional.
//! - Any response may carry the request's `id`.
//! - A failure is `{"ok":false,"kind":…,"error":"<English message>"}` with `kind` one of
//!   `bad_request` (framing, unknown op or a bad field), `not_loaded` (transcribe before load),
//!   `missing_model` (folder or file absent), `bad_model` (files present but refused) or `failed`.
//! - A malformed line gets `bad_request` and the worker reads on. A request whose `bytes` is
//!   unusable can't be skipped exactly, so what follows is read as lines until one parses.
//! - sherpa-onnx ends the process on some damaged models instead of returning an error. The worker
//!   checks for the ones known to do that (`tokens.txt`, `bpe.vocab` and the encoder's metadata)
//!   before loading. If it still dies during a load, stderr holds sherpa-onnx's reason.

mod engine;
mod long_audio;
mod onnx;
// The watch Voice uses to end this worker with itself lives in voice-core, next to the client that
// spawns it. Included by path rather than as a dependency: a worker that a model takes down
// shouldn't link the app's network and archive stacks for a `kqueue` and `pidfd` watch.
#[path = "../../voice-core/src/local/parent.rs"]
mod parent;
mod protocol;
mod stdio;
mod worker;

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    // Before anything else: when the app that started this worker is gone, so is the worker. A
    // worker started by hand has no parent to watch and stops when its stdin closes.
    parent::watch(engine::log);
    // Before anything native runs: sherpa-onnx and onnxruntime must never write into the protocol.
    let mut output = match stdio::claim_stdout() {
        Ok(output) => output,
        Err(error) => {
            engine::log(&format!("could not set up stdout: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let mut engine = engine::Engine::new();
    let result = worker::serve(&mut io::stdin().lock(), &mut output, &mut engine);
    engine.unload();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            engine::log(&format!("stopped: {error}"));
            ExitCode::FAILURE
        }
    }
}
