//! NeMo-Speech.cpp's diarization C ABI, loaded at run time from the runtime folder.
//!
//! The declarations follow `include/nemo_speech/diar.h` and `asr.h` at the pinned commit
//! (`voice/nemo-runtime/manifest.json`): opaque handles, size-prefixed structs and status codes,
//! with `nemo_speech_asr_last_error()` holding the reason for the last failure on this thread.
//! The model must outlive its streams, and a stream is used from one thread.
//!
//! On Windows `nemo_speech_asr_c.dll` is loaded by absolute path with
//! `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR`, so the ggml DLLs and the Visual C++ runtime next to it are
//! found there before anywhere else. The system DLLs it imports are loaded from System32 before
//! it, so a file of the same name in the runtime folder or a `dll_dirs` folder never loads. The
//! application folder isn't searched.
//!
//! On macOS `libnemo_speech_asr_c.dylib` is loaded by absolute path with `dlopen(RTLD_NOW |
//! RTLD_LOCAL)`; its `@loader_path` install names make dyld find the five ggml and C ABI dylibs in
//! the same folder. A hardened process refuses a relative path before it looks at any signature,
//! which [`std::path::absolute`] prevents, and it refuses dylibs signed by another team, which is
//! why the speaker worker carries `com.apple.security.cs.disable-library-validation`.
//!
//! None of these libraries is ever unloaded.

use std::ffi::{c_char, c_void, CStr, CString};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

#[cfg(windows)]
use windows::core::{PCSTR, PCWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::HMODULE;
#[cfg(windows)]
use windows::Win32::System::Diagnostics::Debug::{
    SetThreadErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX, THREAD_ERROR_MODE,
};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::{
    AddDllDirectory, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_FLAGS,
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LOAD_LIBRARY_SEARCH_USER_DIRS,
};

use crate::backend::{check_cpu, Frames, SAMPLE_RATE};
use crate::cpu;
use crate::protocol::{ErrorKind, Failure};

/// The loaded library's handle: a Windows module or a `dlopen` handle.
#[cfg(windows)]
type Module = HMODULE;
#[cfg(not(windows))]
type Module = *mut c_void;

/// The library with the C ABI: it imports the other four DLLs on Windows, the other five dylibs on
/// macOS.
#[cfg(windows)]
pub const LIBRARY: &str = "nemo_speech_asr_c.dll";
#[cfg(not(windows))]
pub const LIBRARY: &str = "libnemo_speech_asr_c.dylib";

/// The system DLLs the runtime imports, as `build.ps1` allows them. Its `api-ms-win-crt-*` API sets
/// resolve to `ucrtbase.dll`. kernel32 and advapi32 are KnownDLLs, which the loader never searches
/// for; dbghelp and ucrtbase it finds by search, starting in the runtime folder.
/// `tests/runtime.rs` plants a copy of every system DLL a real runtime imports in its folder and
/// checks that the System32 copy loads, so a new import missing here fails there.
#[cfg(windows)]
const SYSTEM_IMPORTS: [&str; 4] = [
    "kernel32.dll",
    "advapi32.dll",
    "dbghelp.dll",
    "ucrtbase.dll",
];

/// `nemo_speech_asr_status`: 0 is success.
type Status = i32;
const OK: Status = 0;

/// `nemo_speech_diar_model_config` (`diar.h:30-49`).
#[repr(C)]
#[derive(Debug)]
struct DiarModelConfig {
    size: usize,
    model_path: *const c_char,
    gpu: i32,
    preset: *const c_char,
    chunk_frames: i32,
    right_context_frames: i32,
    left_context_frames: i32,
    fifo_frames: i32,
    spkcache_frames: i32,
    update_period_frames: i32,
}
const _: () = assert!(std::mem::size_of::<DiarModelConfig>() == 56);

impl DiarModelConfig {
    /// The CPU, the given preset and chunk geometry, and the preset's values for everything else
    /// (`left_context_frames < 0` keeps the preset; `0` would be an explicit zero).
    fn live(
        model_path: &CStr,
        preset: &CStr,
        chunk_frames: i32,
        right_context_frames: i32,
    ) -> Self {
        DiarModelConfig {
            size: std::mem::size_of::<DiarModelConfig>(),
            model_path: model_path.as_ptr(),
            gpu: -1,
            preset: preset.as_ptr(),
            chunk_frames,
            right_context_frames,
            left_context_frames: -1,
            fifo_frames: 0,
            spkcache_frames: 0,
            update_period_frames: 0,
        }
    }
}

/// `nemo_speech_diar_segment` (`diar.h:66-70`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Segment {
    pub start_time: f64,
    pub end_time: f64,
    /// 1-based.
    pub speaker: i32,
}

type Create = unsafe extern "C" fn(*const DiarModelConfig, *mut *mut c_void) -> Status;
type Destroy = unsafe extern "C" fn(*mut c_void);
type NumSpeakers = unsafe extern "C" fn(*const c_void) -> i32;
type SecondsPerFrame = unsafe extern "C" fn(*const c_void) -> f64;
type StreamOpen = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> Status;
type PushF32 = unsafe extern "C" fn(*mut c_void, *const f32, usize, i32) -> Status;
type StreamFinish = unsafe extern "C" fn(*mut c_void) -> Status;
type StreamClose = unsafe extern "C" fn(*mut c_void);
type OfflineF32 =
    unsafe extern "C" fn(*mut c_void, *const f32, usize, i32, *mut *mut c_void) -> Status;
type FrameCount = unsafe extern "C" fn(*const c_void) -> i64;
type FrameProbs = unsafe extern "C" fn(*const c_void, *mut f32, usize) -> Status;
type Segments =
    unsafe extern "C" fn(*const c_void, *const c_void, *mut Segment, usize, *mut usize) -> Status;
type Text = unsafe extern "C" fn() -> *const c_char;

/// The exports the worker calls. `offline_f32` is resolved only to check that the DLL has all 13
/// diarization exports of the pinned header.
struct Functions {
    create: Create,
    destroy: Destroy,
    num_speakers: NumSpeakers,
    seconds_per_frame: SecondsPerFrame,
    stream_open: StreamOpen,
    stream_push_f32: PushF32,
    stream_finish: StreamFinish,
    stream_close: StreamClose,
    frame_count: FrameCount,
    frame_probs_start: FrameCount,
    frame_probs: FrameProbs,
    segments: Segments,
    last_error: Text,
    version: Option<Text>,
}

pub struct Runtime {
    f: Functions,
}

/// A path as the NUL-terminated UTF-16 Windows takes.
#[cfg(windows)]
fn wide(path: &std::ffi::OsStr) -> Vec<u16> {
    path.encode_wide().chain(std::iter::once(0)).collect()
}

/// Lets the loader search `dir` for the runtime's imports, as `LOAD_LIBRARY_SEARCH_USER_DIRS`
/// includes the folders added this way. For the rest of the process, which loads one runtime.
#[cfg(windows)]
fn add_dll_directory(dir: &Path) -> Result<(), Failure> {
    // AddDllDirectory takes only an absolute path.
    let absolute = std::path::absolute(dir).map_err(|error| {
        runtime_failure(format!(
            "The folder {} for the speaker label runtime is not usable: {error}",
            dir.display()
        ))
    })?;
    let path = wide(absolute.as_os_str());
    let cookie = unsafe { AddDllDirectory(PCWSTR(path.as_ptr())) };
    if cookie.is_null() {
        return Err(runtime_failure(format!(
            "The speaker label runtime can't use the folder {}: {}",
            absolute.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// Loads each of [`SYSTEM_IMPORTS`] from System32. An import whose name matches a loaded module
/// binds to that module without a search, so the runtime gets these copies even when its folder
/// has a DLL of the same name. The handles are never freed, so the modules stay for the process.
#[cfg(windows)]
fn pin_system_imports() -> Result<(), Failure> {
    for name in SYSTEM_IMPORTS {
        let wide = wide(name.as_ref());
        unsafe { LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|error| {
                runtime_failure(format!(
                    "The speaker label runtime needs {name} from System32, which did not load: {}",
                    error.message()
                ))
            })?;
    }
    Ok(())
}

/// Where the loader looks for the runtime's imports: the runtime folder, the `dll_dirs` folders
/// when there are any, then System32. Not the application folder, which holds none of them.
#[cfg(windows)]
fn search_flags(has_dll_dirs: bool) -> LOAD_LIBRARY_FLAGS {
    let flags = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32;
    if has_dll_dirs {
        flags | LOAD_LIBRARY_SEARCH_USER_DIRS
    } else {
        flags
    }
}

fn runtime_failure(message: impl Into<String>) -> Failure {
    Failure::new(ErrorKind::Runtime, message)
}

/// Looks an export up and casts it to its declared type.
///
/// # Safety
/// `T` must be the function pointer type the export has in the header.
#[cfg(windows)]
unsafe fn symbol<T: Copy>(module: Module, name: &'static str) -> Option<T> {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<usize>());
    let c_name = CString::new(name).expect("export names have no NUL");
    let address = unsafe { GetProcAddress(module, PCSTR(c_name.as_ptr().cast())) }?;
    Some(unsafe { std::mem::transmute_copy(&address) })
}

/// Looks an export up and casts it to its declared type.
///
/// # Safety
/// `T` must be the function pointer type the export has in the header.
#[cfg(not(windows))]
unsafe fn symbol<T: Copy>(module: Module, name: &'static str) -> Option<T> {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<usize>());
    let c_name = CString::new(name).expect("export names have no NUL");
    // dlsym answers null both for a missing export and for one whose value is null; the C ABI
    // exports here are functions, so a null answer is the missing case.
    let address = unsafe { libc::dlsym(module, c_name.as_ptr()) };
    (!address.is_null()).then(|| unsafe { std::mem::transmute_copy(&address) })
}

unsafe fn require<T: Copy>(module: Module, name: &'static str) -> Result<T, Failure> {
    unsafe { symbol(module, name) }.ok_or_else(|| {
        runtime_failure(format!(
            "The speaker label runtime lacks the export {name}; it isn't the pinned NeMo-Speech.cpp build."
        ))
    })
}

/// Loads the C ABI DLL from `library` with the search rules described on [`Runtime::load`].
#[cfg(windows)]
fn load_windows(dir: &Path, library: &Path, dll_dirs: &[PathBuf]) -> Result<Module, Failure> {
    for extra in dll_dirs {
        add_dll_directory(extra)?;
    }
    pin_system_imports()?;
    let wide = wide(library.as_os_str());
    // A damaged or foreign DLL must fail the load instead of opening an error box in a process
    // nobody sees.
    let mut previous = THREAD_ERROR_MODE(0);
    let quiet = unsafe {
        SetThreadErrorMode(
            SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX,
            Some(&mut previous),
        )
    }
    .is_ok();
    let module = unsafe {
        LoadLibraryExW(
            PCWSTR(wide.as_ptr()),
            None,
            search_flags(!dll_dirs.is_empty()),
        )
    };
    if quiet {
        let _ = unsafe { SetThreadErrorMode(previous, None) };
    }
    module.map_err(|error| {
        runtime_failure(format!(
            "The speaker label runtime could not load from {}: {}",
            dir.display(),
            error.message()
        ))
    })
}

/// Loads the C ABI dylib by absolute path. Its `@loader_path` install names make dyld find the
/// five dylibs it imports in the same folder, so no `DYLD_LIBRARY_PATH` or rpath is involved.
#[cfg(not(windows))]
fn load_unix(dir: &Path, library: &Path) -> Result<Module, Failure> {
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(library.as_os_str().as_bytes()).map_err(|_| {
        runtime_failure(format!(
            "The speaker label runtime path {} contains a NUL.",
            library.display()
        ))
    })?;
    let module = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if module.is_null() {
        let reason = unsafe {
            let error = libc::dlerror();
            if error.is_null() {
                "no reason given".to_owned()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            }
        };
        return Err(runtime_failure(format!(
            "The speaker label runtime could not load from {}: {reason}",
            dir.display()
        )));
    }
    Ok(module)
}

impl Runtime {
    /// Checks the processor, then loads the C ABI library from `dir` and resolves its diarization
    /// exports.
    ///
    /// On Windows the system DLLs it imports come from System32 and the others from `dir`, then
    /// from `dll_dirs` (such as Ownkey's C runtime one folder up), then from System32. On macOS
    /// the `dir` holds every dylib the C ABI imports, which come with the folder itself; there is
    /// no C runtime to find. The library stays loaded until the process ends.
    pub fn load(dir: &Path, dll_dirs: &[PathBuf]) -> Result<&'static Runtime, Failure> {
        check_cpu(cpu::detect())?;
        // Both loaders need an absolute path: LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR on Windows, and a
        // hardened process refuses a relative dlopen before any signature check. A verbatim \\?\
        // path would keep the Windows loader from searching the DLL's folder, so no canonicalize.
        let dir = std::path::absolute(dir).map_err(|error| {
            runtime_failure(format!(
                "The speaker label runtime folder {} is not usable: {error}",
                dir.display()
            ))
        })?;
        let library = dir.join(LIBRARY);
        if !library.is_file() {
            return Err(runtime_failure(format!(
                "The speaker label runtime is missing: there is no {}.",
                library.display()
            )));
        }
        #[cfg(windows)]
        let module = load_windows(&dir, &library, dll_dirs)?;
        #[cfg(not(windows))]
        let module = {
            // Only Ownkey's Windows runtime has imports from a folder of its own.
            let _ = dll_dirs;
            load_unix(&dir, &library)?
        };

        let f = unsafe {
            // Present in the pinned build but unused: checked so a different build fails here.
            require::<OfflineF32>(module, "nemo_speech_diar_offline_f32")?;
            Functions {
                create: require(module, "nemo_speech_diar_create")?,
                destroy: require(module, "nemo_speech_diar_destroy")?,
                num_speakers: require(module, "nemo_speech_diar_num_speakers")?,
                seconds_per_frame: require(module, "nemo_speech_diar_seconds_per_frame")?,
                stream_open: require(module, "nemo_speech_diar_stream_open")?,
                stream_push_f32: require(module, "nemo_speech_diar_stream_push_f32")?,
                stream_finish: require(module, "nemo_speech_diar_stream_finish")?,
                stream_close: require(module, "nemo_speech_diar_stream_close")?,
                frame_count: require(module, "nemo_speech_diar_frame_count")?,
                frame_probs_start: require(module, "nemo_speech_diar_frame_probs_start")?,
                frame_probs: require(module, "nemo_speech_diar_frame_probs")?,
                segments: require(module, "nemo_speech_diar_segments")?,
                last_error: require(module, "nemo_speech_asr_last_error")?,
                version: symbol(module, "nemo_speech_asr_version"),
            }
        };
        Ok(Box::leak(Box::new(Runtime { f })))
    }

    /// `nemo_speech_asr_version()`, when the build exports it.
    pub fn version(&self) -> Option<String> {
        let version = self.f.version?;
        text(unsafe { version() })
    }

    /// A failure with the runtime's reason. Call it on the thread of the failing call, right
    /// after it, before anything else touches the runtime.
    fn failure(&self, kind: ErrorKind, what: &str, status: Status) -> Failure {
        let detail = text(unsafe { (self.f.last_error)() })
            .unwrap_or_else(|| format!("The runtime returned status {status}."));
        Failure::new(
            kind,
            format!("Speaker labels failed while {what}. {detail}"),
        )
    }
}

fn text(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .trim()
        .to_owned();
    (!text.is_empty()).then_some(text)
}

pub struct Model {
    rt: &'static Runtime,
    handle: *mut c_void,
    speakers: usize,
    seconds_per_frame: f64,
}

impl Model {
    pub fn create(
        rt: &'static Runtime,
        gguf: &Path,
        preset: &str,
        chunk_frames: i32,
        right_context_frames: i32,
    ) -> Result<Model, Failure> {
        let model_failure = |message: String| Failure::new(ErrorKind::Model, message);
        if !gguf.is_file() {
            return Err(model_failure(format!(
                "The speaker model is missing: there is no {}.",
                gguf.display()
            )));
        }
        // The header takes a UTF-8 path.
        let path = gguf.to_str().ok_or_else(|| {
            model_failure(format!(
                "The speaker model path {} isn't valid Unicode.",
                gguf.display()
            ))
        })?;
        let path = CString::new(path)
            .map_err(|_| model_failure("The speaker model path contains a NUL.".into()))?;
        let preset = CString::new(preset)
            .map_err(|_| Failure::bad_request("preset must not contain a NUL."))?;
        let config = DiarModelConfig::live(&path, &preset, chunk_frames, right_context_frames);
        let mut handle = ptr::null_mut();
        let status = unsafe { (rt.f.create)(&config, &mut handle) };
        if status != OK || handle.is_null() {
            return Err(rt.failure(ErrorKind::Model, "loading the model", status));
        }
        let speakers = unsafe { (rt.f.num_speakers)(handle) };
        let seconds_per_frame = frame_seconds(unsafe { (rt.f.seconds_per_frame)(handle) });
        if speakers <= 0 || seconds_per_frame.is_nan() || seconds_per_frame <= 0.0 {
            unsafe { (rt.f.destroy)(handle) };
            return Err(model_failure(format!(
                "The speaker model reports {speakers} speakers and {seconds_per_frame} s per frame."
            )));
        }
        Ok(Model {
            rt,
            handle,
            speakers: speakers as usize,
            seconds_per_frame,
        })
    }

    pub fn speakers(&self) -> usize {
        self.speakers
    }

    pub fn seconds_per_frame(&self) -> f64 {
        self.seconds_per_frame
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { (self.rt.f.destroy)(self.handle) };
    }
}

/// One diarizer stream. `read` is the next frame to return.
pub struct Stream {
    model: &'static Model,
    handle: *mut c_void,
    read: u64,
    retained: Vec<f32>,
}

impl Stream {
    pub fn open(model: &'static Model) -> Result<Stream, Failure> {
        let rt = model.rt;
        let mut handle = ptr::null_mut();
        let status = unsafe { (rt.f.stream_open)(model.handle, &mut handle) };
        if status != OK || handle.is_null() {
            return Err(rt.failure(ErrorKind::Failed, "opening a stream", status));
        }
        Ok(Stream {
            model,
            handle,
            read: 0,
            retained: Vec::new(),
        })
    }

    /// Pushes 16 kHz mono samples and returns the frames committed since the last call.
    pub fn push(&mut self, pcm: &[i16]) -> Result<Frames, Failure> {
        let rt = self.model.rt;
        if !pcm.is_empty() {
            let samples = to_f32(pcm);
            let status = unsafe {
                (rt.f.stream_push_f32)(
                    self.handle,
                    samples.as_ptr(),
                    samples.len(),
                    SAMPLE_RATE as i32,
                )
            };
            if status != OK {
                return Err(rt.failure(ErrorKind::Failed, "pushing audio", status));
            }
        }
        self.collect()
    }

    /// Labels the rest of the audio and returns the last frames. Pushes after it are ignored by
    /// the runtime.
    pub fn finish(&mut self) -> Result<Frames, Failure> {
        let rt = self.model.rt;
        let status = unsafe { (rt.f.stream_finish)(self.handle) };
        if status != OK {
            return Err(rt.failure(ErrorKind::Failed, "finishing the stream", status));
        }
        self.collect()
    }

    /// Reads the retained probabilities and returns the ones from `read` on. The runtime keeps
    /// about the last 10 minutes once a stream passes 20; reading after every push stays far
    /// inside that.
    fn collect(&mut self) -> Result<Frames, Failure> {
        let rt = self.model.rt;
        let speakers = self.model.speakers;
        let count = unsafe { (rt.f.frame_count)(self.handle) };
        let start = unsafe { (rt.f.frame_probs_start)(self.handle) };
        let (count, start) = match (u64::try_from(count), u64::try_from(start)) {
            (Ok(count), Ok(start)) if start <= count => (count, start),
            _ => {
                return Err(Failure::new(
                    ErrorKind::Failed,
                    format!("The runtime reports frames {start} to {count}."),
                ))
            }
        };
        check_window(self.read, start, count)?;
        if count == self.read {
            return Ok(Frames {
                first: self.read,
                values: Vec::new(),
                count,
            });
        }
        let floats = (count - start) as usize * speakers;
        self.retained.resize(floats, 0.0);
        let status = unsafe { (rt.f.frame_probs)(self.handle, self.retained.as_mut_ptr(), floats) };
        if status != OK {
            return Err(rt.failure(ErrorKind::Failed, "reading frame probabilities", status));
        }
        let from = (self.read - start) as usize * speakers;
        let frames = Frames {
            first: self.read,
            values: self.retained[from..].to_vec(),
            count,
        };
        self.read = count;
        Ok(frames)
    }

    /// The runtime's own segments under its default postprocessing, for `--self-test`.
    pub fn segments(&self) -> Result<Vec<Segment>, Failure> {
        let rt = self.model.rt;
        let mut count = 0usize;
        let status =
            unsafe { (rt.f.segments)(self.handle, ptr::null(), ptr::null_mut(), 0, &mut count) };
        if status != OK {
            return Err(rt.failure(ErrorKind::Failed, "counting segments", status));
        }
        let mut segments = vec![Segment::default(); count];
        let status = unsafe {
            (rt.f.segments)(
                self.handle,
                ptr::null(),
                segments.as_mut_ptr(),
                segments.len(),
                &mut count,
            )
        };
        if status != OK {
            return Err(rt.failure(ErrorKind::Failed, "reading segments", status));
        }
        segments.truncate(count);
        Ok(segments)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { (self.model.rt.f.stream_close)(self.handle) };
    }
}

/// The frame length to the microsecond. The runtime keeps it as a float32, so 10 ms arrives as
/// 0.009999999776; a frame is a whole number of samples, and the app counts in exact frames.
fn frame_seconds(reported: f64) -> f64 {
    (reported * 1e6).round() / 1e6
}

/// 16-bit samples as the runtime's floats in [-1, 1).
pub fn to_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter()
        .map(|&sample| f32::from(sample) / 32768.0)
        .collect()
}

/// Fails when the runtime already dropped frame `read` (it keeps frames `start..count`).
fn check_window(read: u64, start: u64, count: u64) -> Result<(), Failure> {
    if start > read {
        return Err(Failure::new(
            ErrorKind::Behind,
            format!(
                "Speaker labels fell behind the runtime's retained frames: frame {read} is gone, the runtime keeps {start} to {count}."
            ),
        ));
    }
    if count < read {
        return Err(Failure::new(
            ErrorKind::Failed,
            format!("The runtime's frame count went back from {read} to {count}."),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_matches_the_header_layout() {
        assert_eq!(std::mem::size_of::<DiarModelConfig>(), 56);
        assert_eq!(std::mem::offset_of!(DiarModelConfig, model_path), 8);
        assert_eq!(std::mem::offset_of!(DiarModelConfig, gpu), 16);
        assert_eq!(std::mem::offset_of!(DiarModelConfig, preset), 24);
        assert_eq!(std::mem::offset_of!(DiarModelConfig, chunk_frames), 32);
        assert_eq!(
            std::mem::offset_of!(DiarModelConfig, update_period_frames),
            52
        );
        assert_eq!(std::mem::size_of::<Segment>(), 24);
    }

    #[test]
    fn the_live_config_runs_on_the_cpu_and_keeps_the_preset_elsewhere() {
        let path = CString::new("C:\\models\\n.gguf").unwrap();
        let preset = CString::new("v3-streaming").unwrap();
        let config = DiarModelConfig::live(&path, &preset, 38, 1);
        assert_eq!(config.size, 56);
        assert_eq!(config.model_path, path.as_ptr());
        assert_eq!(config.preset, preset.as_ptr());
        assert_eq!(config.gpu, -1);
        assert_eq!((config.chunk_frames, config.right_context_frames), (38, 1));
        assert_eq!(config.left_context_frames, -1);
        assert_eq!(
            (
                config.fifo_frames,
                config.spkcache_frames,
                config.update_period_frames
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn the_frame_length_loses_the_float32_noise() {
        assert_eq!(frame_seconds(f64::from(0.01f32)), 0.01);
        assert_eq!(frame_seconds(f64::from(0.08f32)), 0.08);
        assert!(frame_seconds(f64::NAN).is_nan());
    }

    #[test]
    fn samples_become_floats_in_the_unit_range() {
        assert_eq!(
            to_f32(&[0, 16384, -16384, i16::MIN, i16::MAX]),
            [0.0, 0.5, -0.5, -1.0, 32767.0 / 32768.0]
        );
    }

    #[test]
    fn a_reader_behind_the_retained_window_is_behind() {
        assert!(check_window(0, 0, 0).is_ok());
        assert!(check_window(100, 0, 400).is_ok());
        assert!(check_window(100, 100, 400).is_ok());
        let behind = check_window(100, 101, 400).unwrap_err();
        assert_eq!(behind.kind, ErrorKind::Behind);
        assert!(behind.message.contains("frame 100"), "{}", behind.message);
        assert_eq!(
            check_window(500, 0, 400).unwrap_err().kind,
            ErrorKind::Failed
        );
    }

    #[test]
    fn a_processor_without_the_baseline_is_refused_before_loading() {
        let failure = check_cpu(cpu::CpuFeatures::default()).unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Cpu);
        assert!(
            failure.message.starts_with("This processor lacks "),
            "{}",
            failure.message
        );
    }

    #[test]
    fn a_folder_without_the_dll_is_a_missing_runtime() {
        let dir =
            std::env::temp_dir().join(format!("rukoo-voice-speakers-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let failure = Runtime::load(&dir, &[]).err().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        if cpu::detect().refusal().is_some() {
            assert_eq!(failure.kind, ErrorKind::Cpu);
            return;
        }
        assert_eq!(failure.kind, ErrorKind::Runtime);
        assert!(failure.message.contains(LIBRARY), "{}", failure.message);
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_for_imports_is_added_and_a_missing_one_refused() {
        let dir =
            std::env::temp_dir().join(format!("rukoo-voice-speakers-crt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let added = add_dll_directory(&dir);
        let missing = add_dll_directory(&dir.join("missing"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(added, Ok(()));
        let failure = missing.unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Runtime);
        assert!(failure.message.contains("missing"), "{}", failure.message);
    }

    #[cfg(windows)]
    #[test]
    fn the_system_imports_load_from_system32() {
        use std::os::windows::ffi::OsStringExt;
        use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
        use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

        let path = |buffer: &[u16], len: u32| {
            assert!(len > 0 && (len as usize) < buffer.len());
            PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len as usize]))
        };
        let mut buffer = [0u16; 260];
        let len = unsafe { GetSystemDirectoryW(Some(&mut buffer)) };
        let system32 = std::fs::canonicalize(path(&buffer, len)).unwrap();

        assert_eq!(pin_system_imports(), Ok(()));
        for name in SYSTEM_IMPORTS {
            let wide = wide(name.as_ref());
            let module = unsafe { GetModuleHandleW(PCWSTR(wide.as_ptr())) }.unwrap();
            let mut buffer = [0u16; 260];
            let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) };
            let file = path(&buffer, len);
            let folder = std::fs::canonicalize(file.parent().unwrap()).unwrap();
            assert!(
                folder
                    .as_os_str()
                    .eq_ignore_ascii_case(system32.as_os_str()),
                "{name} is {}",
                file.display()
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn imports_are_searched_for_outside_the_application_folder() {
        let own = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32;
        assert_eq!(search_flags(false), own);
        assert_eq!(search_flags(true), own | LOAD_LIBRARY_SEARCH_USER_DIRS);
    }

    #[test]
    fn a_file_that_is_not_a_dll_fails_to_load_without_a_dialog() {
        if cpu::detect().refusal().is_some() {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("rukoo-voice-speakers-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LIBRARY), b"MZ but not a DLL").unwrap();
        let failure = Runtime::load(&dir, &[]).err().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(failure.kind, ErrorKind::Runtime);
        assert!(
            failure.message.contains("could not load"),
            "{}",
            failure.message
        );
    }
}
