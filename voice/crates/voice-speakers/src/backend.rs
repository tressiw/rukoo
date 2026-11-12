//! What the worker asks of the diarizer, so its request handling can be tested without the native
//! runtime. [`Native`] is NeMo-Speech.cpp on Windows and macOS on Apple Silicon, and a refusal
//! elsewhere.

use std::path::PathBuf;

use crate::cpu::CpuFeatures;
use crate::protocol::{ErrorKind, Failure};

/// The `v3-streaming` preset with Ownkey's live geometry: 38 frames of 80 ms (3.04 s) per chunk
/// and 1 frame of right context. It ran 8.2 times faster than real time on the maintainer's PC
/// with batch accuracy and 26 times on an Apple M4; the preset's own 13-frame chunk ran 3.3 times
/// faster than real time on the PC.
pub const PRESET: &str = "v3-streaming";
pub const CHUNK_FRAMES: i32 = 38;
pub const RIGHT_CONTEXT_FRAMES: i32 = 1;

/// The audio rate the worker accepts.
pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadRequest {
    /// The folder holding the runtime's C ABI library and the libraries it imports.
    pub runtime: PathBuf,
    /// More folders to search for those libraries' imports, such as Ownkey's C runtime one folder
    /// up. Windows only.
    pub dll_dirs: Vec<PathBuf>,
    /// The Nemotron GGUF.
    pub model: PathBuf,
    pub preset: String,
    pub chunk_frames: i32,
    pub right_context_frames: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub speakers: usize,
    pub seconds_per_frame: f64,
    pub runtime_version: Option<String>,
}

/// Probabilities a call produced: `values` holds whole frames of `speakers` values each, starting
/// at frame `first`. `count` is the stream's frame count after the call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frames {
    pub first: u64,
    pub values: Vec<f32>,
    pub count: u64,
}

/// Refuses a processor the portable build can't run on, before anything native is loaded.
pub fn check_cpu(features: CpuFeatures) -> Result<(), Failure> {
    match features.refusal() {
        Some(message) => Err(Failure::new(ErrorKind::Cpu, message)),
        None => Ok(()),
    }
}

pub trait Backend {
    /// Loads the runtime and the model. Called at most once successfully.
    fn load(&mut self, request: &LoadRequest) -> Result<Loaded, Failure>;
    /// Opens a new stream, closing the previous one. Only after a successful load.
    fn open(&mut self) -> Result<(), Failure>;
    /// Pushes 16 kHz mono audio into the open stream and returns the frames it committed.
    fn push(&mut self, pcm: &[i16]) -> Result<Frames, Failure>;
    /// Labels the rest of the stream and returns the last frames.
    fn finish(&mut self) -> Result<Frames, Failure>;
    /// Closes the stream, if one is open.
    fn close(&mut self);
}

/// The real diarizer: NeMo-Speech.cpp, loaded from the folder `load` names.
#[derive(Default)]
pub struct Native {
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    model: Option<&'static crate::nemo::Model>,
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    stream: Option<crate::nemo::Stream>,
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
impl Backend for Native {
    fn load(&mut self, request: &LoadRequest) -> Result<Loaded, Failure> {
        let runtime = crate::nemo::Runtime::load(&request.runtime, &request.dll_dirs)?;
        let model = crate::nemo::Model::create(
            runtime,
            &request.model,
            &request.preset,
            request.chunk_frames,
            request.right_context_frames,
        )?;
        // One model per process, alive until it exits, so streams can borrow it for good.
        let model: &'static crate::nemo::Model = Box::leak(Box::new(model));
        self.model = Some(model);
        Ok(Loaded {
            speakers: model.speakers(),
            seconds_per_frame: model.seconds_per_frame(),
            runtime_version: runtime.version(),
        })
    }

    fn open(&mut self) -> Result<(), Failure> {
        let model = self.model.expect("the worker opens only after a load");
        self.stream = None;
        self.stream = Some(crate::nemo::Stream::open(model)?);
        Ok(())
    }

    fn push(&mut self, pcm: &[i16]) -> Result<Frames, Failure> {
        self.stream
            .as_mut()
            .expect("the worker pushes only into an open stream")
            .push(pcm)
    }

    fn finish(&mut self) -> Result<Frames, Failure> {
        self.stream
            .as_mut()
            .expect("the worker finishes only an open stream")
            .finish()
    }

    fn close(&mut self) {
        self.stream = None;
    }
}

#[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
impl Backend for Native {
    fn load(&mut self, _request: &LoadRequest) -> Result<Loaded, Failure> {
        check_cpu(crate::cpu::detect())?;
        Err(Failure::new(
            ErrorKind::Runtime,
            "Speaker labels run on Windows on x86-64 and macOS on Apple Silicon only.",
        ))
    }

    fn open(&mut self) -> Result<(), Failure> {
        unreachable!("the worker opens only after a load")
    }

    fn push(&mut self, _pcm: &[i16]) -> Result<Frames, Failure> {
        unreachable!("the worker pushes only into an open stream")
    }

    fn finish(&mut self) -> Result<Frames, Failure> {
        unreachable!("the worker finishes only an open stream")
    }

    fn close(&mut self) {}
}
