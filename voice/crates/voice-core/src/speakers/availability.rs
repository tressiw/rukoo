//! Whether speaker labels can run on this computer, answered without starting a process, and what
//! Voice remembers about failed workers until it restarts.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use super::api::{DiarError, DiarErrorKind, Diarizer};
use super::client::WorkerDiarizer;
use super::cpu::{cpu_features, CpuFeatures};
use super::runtime::{self, RuntimeInfo};
#[cfg(windows)]
use crate::local::Job;
use crate::local::{Located, ModelRegistry, ModelSpec};
use crate::paths;

/// The runtime is built for Windows on x86-64 and, with its own build, for macOS on Apple Silicon.
const PLATFORM_SUPPORTED: bool = cfg!(all(windows, target_arch = "x86_64"))
    || cfg!(all(target_os = "macos", target_arch = "aarch64"));

/// Why speaker labels can't run. Serialized with the Settings reason code in `code`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum Unavailable {
    /// Not Windows on x86-64 or macOS on Apple Silicon.
    Platform,
    /// The processor lacks features the runtime needs.
    Cpu { missing: Vec<String> },
    /// No runtime folder verifies, or the worker program is missing.
    Runtime,
    /// A worker refused to load the runtime or the model, or crashed while loading, since Voice
    /// started.
    RuntimeFailed { detail: String },
    /// The Nemotron model isn't installed.
    Model,
    /// The model failed its check since Voice started.
    ModelDamaged { detail: String },
}

impl Unavailable {
    /// The Settings reason code: the same string as the serialized `code`.
    pub fn code(&self) -> &'static str {
        match self {
            Unavailable::Platform => "platform",
            Unavailable::Cpu { .. } => "cpu",
            Unavailable::Runtime => "runtime",
            Unavailable::RuntimeFailed { .. } => "runtimeFailed",
            Unavailable::Model => "model",
            Unavailable::ModelDamaged { .. } => "modelDamaged",
        }
    }
}

impl fmt::Display for Unavailable {
    /// English, for logs. Settings shows its own text for each code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::Platform => f.write_str(
                "Speaker labels need Windows on an Intel or AMD PC, or an Apple Silicon Mac.",
            ),
            Unavailable::Cpu { missing } => write!(
                f,
                "This processor lacks {}, which speaker labels need.",
                missing.join(", ")
            ),
            Unavailable::Runtime => f.write_str("The speaker label runtime is missing."),
            Unavailable::RuntimeFailed { detail } | Unavailable::ModelDamaged { detail } => {
                f.write_str(detail)
            }
            Unavailable::Model => f.write_str("The speaker model isn't downloaded."),
        }
    }
}

/// How [`SpeakerLabels`] finds and runs the worker.
#[derive(Clone, Debug)]
pub struct SpeakerOptions {
    /// None: `RUKOO_VOICE_SPEAKERS_EXE`, then `rukoo-voice-speakers` next to the running exe.
    pub worker_exe: Option<PathBuf>,
    /// The folder the runtime is looked for in, as `nemo_speech\`.
    pub exe_dir: PathBuf,
    pub registry: ModelRegistry,
    pub dev_mode: bool,
    pub start_timeout: Duration,
    pub load_timeout: Duration,
    pub push_timeout: Duration,
    pub finish_timeout: Duration,
    /// The model the worker loads: Nemotron 3 Diarization. Tests use a small stand-in.
    pub model: ModelSpec,
    /// This processor's features. Tests simulate an older processor.
    pub cpu: CpuFeatures,
    /// `RUKOO_VOICE_NEMO_DIR` when the options were made; read in development mode only.
    pub nemo_dir: Option<PathBuf>,
    /// The installed Ownkey's runtime folder, read only; `None` never looks there.
    pub ownkey_runtime: Option<PathBuf>,
    /// `RUKOO_VOICE_TEST_NO_SPEAKERS` when the options were made: `cpu`, `runtime` or `model`
    /// forces that answer. Read in development mode only.
    pub no_speakers: Option<String>,
}

impl SpeakerOptions {
    pub fn new(exe_dir: impl Into<PathBuf>, registry: ModelRegistry) -> SpeakerOptions {
        SpeakerOptions {
            worker_exe: None,
            exe_dir: exe_dir.into(),
            registry,
            dev_mode: false,
            start_timeout: Duration::from_secs(10),
            load_timeout: Duration::from_secs(60),
            push_timeout: Duration::from_secs(30),
            finish_timeout: Duration::from_secs(60),
            model: ModelSpec::nemotron(),
            cpu: cpu_features(),
            nemo_dir: paths::env_var(paths::NEMO_DIR_ENV).map(PathBuf::from),
            ownkey_runtime: paths::ownkey_nemo_runtime(),
            no_speakers: paths::env_var(paths::NO_SPEAKERS_ENV),
        }
    }

    /// The worker program this would start.
    pub fn worker(&self) -> Option<PathBuf> {
        self.worker_exe.clone().or_else(paths::speakers_worker_exe)
    }

    /// The reason a development copy forces, if any.
    fn forced(&self) -> Option<Unavailable> {
        if !self.dev_mode {
            return None;
        }
        match self.no_speakers.as_deref().map(str::trim) {
            Some("cpu") => Some(Unavailable::Cpu {
                missing: CpuFeatures::default()
                    .missing()
                    .into_iter()
                    .map(String::from)
                    .collect(),
            }),
            Some("runtime") => Some(Unavailable::Runtime),
            Some("model") => Some(Unavailable::Model),
            _ => None,
        }
    }
}

/// One per process: answers whether labels can run and opens diarizers for meetings.
#[derive(Clone)]
pub struct SpeakerLabels {
    inner: Arc<Inner>,
}

struct Inner {
    opts: Mutex<SpeakerOptions>,
    memory: Mutex<Memory>,
    /// Whether this build can run the runtime at all.
    supported: bool,
    /// Every worker goes in here, so none outlives Voice.
    #[cfg(windows)]
    job: Option<Job>,
}

/// What failed since Voice started.
#[derive(Default)]
struct Memory {
    runtime_failed: Option<String>,
    damaged: Option<Damaged>,
}

/// A model that failed its check, as long as its files stay as they were.
struct Damaged {
    dir: PathBuf,
    files: Vec<Option<(u64, Option<SystemTime>)>>,
    detail: String,
}

/// Each pinned file's size and modification time, so a new download clears the memory.
fn file_stamps(spec: &ModelSpec, dir: &Path) -> Vec<Option<(u64, Option<SystemTime>)>> {
    spec.files
        .iter()
        .map(|file| {
            fs::symlink_metadata(dir.join(&file.name))
                .ok()
                .map(|meta| (meta.len(), meta.modified().ok()))
        })
        .collect()
}

impl fmt::Debug for SpeakerLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpeakerLabels")
            .field("failure", &self.failure())
            .finish()
    }
}

impl SpeakerLabels {
    pub fn new(opts: SpeakerOptions) -> SpeakerLabels {
        SpeakerLabels::with_platform(opts, PLATFORM_SUPPORTED)
    }

    fn with_platform(opts: SpeakerOptions, supported: bool) -> SpeakerLabels {
        SpeakerLabels {
            inner: Arc::new(Inner {
                opts: Mutex::new(opts),
                memory: Mutex::new(Memory::default()),
                supported,
                #[cfg(windows)]
                job: Job::new(),
            }),
        }
    }

    /// Whether labels can run now: platform, processor, remembered failures, runtime and model.
    /// Cheap enough for every Settings refresh; starts no process. The model is only looked
    /// for here; its hash is checked when a meeting opens a stream.
    pub fn availability(&self) -> Result<RuntimeInfo, Unavailable> {
        let opts = self.options();
        if let Some(forced) = opts.forced() {
            return Err(forced);
        }
        if !self.inner.supported {
            return Err(Unavailable::Platform);
        }
        let missing = opts.cpu.missing();
        if !missing.is_empty() {
            return Err(Unavailable::Cpu {
                missing: missing.into_iter().map(String::from).collect(),
            });
        }
        if let Some(detail) = self.memory().runtime_failed.clone() {
            return Err(Unavailable::RuntimeFailed { detail });
        }
        let runtime = find_runtime(&opts).ok_or(Unavailable::Runtime)?;
        let model = opts
            .registry
            .locate(&opts.model)
            .ok_or(Unavailable::Model)?;
        if let Some(detail) = self.damaged(&opts.model, &model.dir) {
            return Err(Unavailable::ModelDamaged { detail });
        }
        Ok(runtime)
    }

    /// A diarizer for one meeting, or why there is none.
    pub fn diarizer(&self) -> Result<Arc<dyn Diarizer>, Unavailable> {
        let runtime = self.availability()?;
        let opts = self.options();
        let model = opts
            .registry
            .locate(&opts.model)
            .ok_or(Unavailable::Model)?;
        Ok(self.worker_diarizer(runtime, model))
    }

    /// A diarizer for a runtime and model the caller found itself, skipping the platform and
    /// runtime checks of [`SpeakerLabels::availability`]. For tests and tools; the app uses
    /// [`SpeakerLabels::diarizer`]. Remembered failures and the model check still apply when it
    /// opens a stream.
    pub fn worker_diarizer(&self, runtime: RuntimeInfo, model: Located) -> Arc<dyn Diarizer> {
        Arc::new(WorkerDiarizer {
            labels: self.clone(),
            runtime,
            model,
        })
    }

    /// What Voice remembers since it started: a runtime that failed to load, or a model that
    /// failed its check and hasn't changed since.
    pub fn failure(&self) -> Option<Unavailable> {
        if let Some(detail) = self.memory().runtime_failed.clone() {
            return Some(Unavailable::RuntimeFailed { detail });
        }
        let dir = self.memory().damaged.as_ref().map(|d| d.dir.clone())?;
        self.damaged(&self.options().model, &dir)
            .map(|detail| Unavailable::ModelDamaged { detail })
    }

    /// Where the model is looked for, after a Settings change.
    pub fn set_registry(&self, registry: ModelRegistry) {
        lock(&self.inner.opts).registry = registry;
    }

    pub(super) fn options(&self) -> SpeakerOptions {
        lock(&self.inner.opts).clone()
    }

    fn memory(&self) -> MutexGuard<'_, Memory> {
        lock(&self.inner.memory)
    }

    /// Remembers that the runtime can't run here, until Voice restarts. The first reason stays.
    pub(super) fn remember_runtime_failure(&self, detail: &str) {
        let mut memory = self.memory();
        if memory.runtime_failed.is_none() {
            memory.runtime_failed = Some(detail.to_string());
        }
    }

    /// Remembers a model that failed its check, while its files stay as they are.
    pub(super) fn remember_damaged_model(&self, spec: &ModelSpec, model: &Located, detail: &str) {
        self.memory().damaged = Some(Damaged {
            dir: model.dir.clone(),
            files: file_stamps(spec, &model.dir),
            detail: detail.to_string(),
        });
    }

    /// The remembered check failure for this model folder, if its files haven't changed since.
    fn damaged(&self, spec: &ModelSpec, dir: &Path) -> Option<String> {
        let mut memory = self.memory();
        let damaged = memory.damaged.as_ref()?;
        if damaged.dir != dir {
            return None;
        }
        if damaged.files != file_stamps(spec, dir) {
            // Replaced since, for example downloaded again: check it afresh.
            memory.damaged = None;
            return None;
        }
        Some(damaged.detail.clone())
    }

    /// Why a stream must not start, from what Voice remembers.
    pub(super) fn refusal(&self, spec: &ModelSpec, model: &Located) -> Option<DiarError> {
        if let Some(detail) = self.memory().runtime_failed.clone() {
            return Some(DiarError::new(DiarErrorKind::Runtime, detail));
        }
        self.damaged(spec, &model.dir)
            .map(|detail| DiarError::new(DiarErrorKind::Model, detail))
    }

    /// Puts a worker in the kill-on-close job.
    #[cfg(windows)]
    pub(super) fn adopt(&self, child: &std::process::Child) {
        if let Some(job) = &self.inner.job {
            job.assign(child);
        }
    }
}

/// The runtime folder, and only when the worker program exists too.
fn find_runtime(opts: &SpeakerOptions) -> Option<RuntimeInfo> {
    if !opts.worker().is_some_and(|exe| exe.is_file()) {
        return None;
    }
    let dev = opts.nemo_dir.as_deref().filter(|_| opts.dev_mode);
    runtime::locate_in(
        &opts.exe_dir,
        dev,
        opts.ownkey_runtime.as_deref(),
        runtime::manifest(),
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::local::{Location, ModelFile};
    use crate::speakers::runtime::{RuntimeLocation, RUNTIME_DLLS};
    use crate::speakers::tests::TempDir;

    const MODEL_BYTES: &[u8] = b"a tiny stand-in for the speaker model";
    const ALL: CpuFeatures = CpuFeatures {
        avx2: true,
        fma: true,
        f16c: true,
        bmi2: true,
        neon: true,
        dotprod: true,
        fp16: true,
    };

    fn spec() -> ModelSpec {
        use sha2::{Digest, Sha256};
        let sha256 = Sha256::digest(MODEL_BYTES)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        ModelSpec {
            id: "speakers-test".into(),
            label: "Speakers test".into(),
            revision: "rev1".into(),
            folder: "speakers-test".into(),
            settings_page: "Meetings > Speaker labels".into(),
            custom_folder: false,
            archive: None,
            files: vec![ModelFile {
                name: "model.gguf".into(),
                size: MODEL_BYTES.len() as u64,
                sha256,
                url: None,
            }],
        }
    }

    /// Options where everything is in place: a development runtime folder, a worker program and
    /// the model in Voice's own folder.
    fn ready(temp: &TempDir) -> SpeakerOptions {
        let nemo = temp.join("nemo");
        fs::create_dir_all(&nemo).unwrap();
        for name in RUNTIME_DLLS {
            fs::write(nemo.join(name), name).unwrap();
        }
        let worker = temp.join("rukoo-voice-speakers.exe");
        fs::write(&worker, b"not run here").unwrap();
        let registry = ModelRegistry::new(temp.join("data")).with_ownkey_models(None);
        let mut opts = SpeakerOptions::new(temp.join("exe"), registry);
        opts.worker_exe = Some(worker);
        opts.dev_mode = true;
        opts.nemo_dir = Some(nemo);
        opts.ownkey_runtime = None;
        opts.no_speakers = None;
        opts.cpu = ALL;
        opts.model = spec();
        let dir = opts.registry.own_dir(&opts.model);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.gguf"), MODEL_BYTES).unwrap();
        opts
    }

    fn labels(opts: SpeakerOptions) -> SpeakerLabels {
        SpeakerLabels::with_platform(opts, true)
    }

    #[test]
    fn codes_match_the_serialized_reason() {
        let cases = [
            Unavailable::Platform,
            Unavailable::Cpu {
                missing: vec!["avx2".into()],
            },
            Unavailable::Runtime,
            Unavailable::RuntimeFailed { detail: "x".into() },
            Unavailable::Model,
            Unavailable::ModelDamaged { detail: "y".into() },
        ];
        for case in cases {
            let value = serde_json::to_value(&case).unwrap();
            assert_eq!(value["code"], case.code());
        }
        let cpu = serde_json::to_value(Unavailable::Cpu {
            missing: vec!["bmi2".into()],
        })
        .unwrap();
        assert_eq!(cpu, serde_json::json!({"code": "cpu", "missing": ["bmi2"]}));
    }

    #[test]
    fn everything_in_place_is_ready() {
        let temp = TempDir::new("avail-ready");
        let opts = ready(&temp);
        let nemo = opts.nemo_dir.clone().unwrap();
        let runtime = labels(opts).availability().unwrap();
        assert_eq!(runtime.location, RuntimeLocation::Dev);
        assert_eq!(runtime.dir, nemo);
    }

    #[test]
    fn another_platform_is_unavailable_first() {
        let temp = TempDir::new("avail-platform");
        let mut opts = ready(&temp);
        opts.cpu = CpuFeatures::default();
        let labels = SpeakerLabels::with_platform(opts, false);
        assert_eq!(labels.availability(), Err(Unavailable::Platform));
        assert_eq!(labels.diarizer().err(), Some(Unavailable::Platform));
    }

    #[test]
    fn missing_cpu_features_give_cpu() {
        let temp = TempDir::new("avail-cpu");
        let mut opts = ready(&temp);
        opts.cpu = CpuFeatures::default();
        let expected = Unavailable::Cpu {
            missing: CpuFeatures::default()
                .missing()
                .into_iter()
                .map(String::from)
                .collect(),
        };
        let labels = labels(opts);
        assert_eq!(labels.availability(), Err(expected.clone()));
        assert_eq!(labels.diarizer().err(), Some(expected));
    }

    #[test]
    fn a_remembered_failure_stays_until_a_new_speaker_labels() {
        let temp = TempDir::new("avail-memory");
        let opts = ready(&temp);
        let first = labels(opts.clone());
        first.remember_runtime_failure("ggml-cpu.dll failed to load");
        first.remember_runtime_failure("a later failure doesn't replace the first");
        let failed = Unavailable::RuntimeFailed {
            detail: "ggml-cpu.dll failed to load".into(),
        };
        assert_eq!(first.availability(), Err(failed.clone()));
        assert_eq!(first.failure(), Some(failed.clone()));
        // Clones share the memory: it's one per process.
        assert_eq!(first.clone().diarizer().err(), Some(failed));
        // The CPU check still comes first.
        let mut old_cpu = opts.clone();
        old_cpu.cpu = CpuFeatures::default();
        let second = labels(old_cpu);
        second.remember_runtime_failure("x");
        assert_eq!(second.availability().unwrap_err().code(), "cpu");
        // A restart forgets.
        assert!(labels(opts).availability().is_ok());
    }

    #[test]
    fn a_damaged_model_is_remembered_until_its_files_change() {
        let temp = TempDir::new("avail-damaged");
        let opts = ready(&temp);
        let spec = opts.model.clone();
        let dir = opts.registry.own_dir(&spec);
        let located = Located {
            dir: dir.clone(),
            location: Location::Voice,
        };
        let labels = labels(opts);
        labels.remember_damaged_model(&spec, &located, "Model file is damaged: model.gguf.");
        let damaged = Unavailable::ModelDamaged {
            detail: "Model file is damaged: model.gguf.".into(),
        };
        assert_eq!(labels.availability(), Err(damaged.clone()));
        assert_eq!(labels.failure(), Some(damaged));
        // A new download replaces the file: checked afresh.
        let file = fs::OpenOptions::new()
            .write(true)
            .open(dir.join("model.gguf"))
            .unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(60))
            .unwrap();
        drop(file);
        assert!(labels.availability().is_ok());
        assert_eq!(labels.failure(), None);
    }

    #[test]
    fn runtime_model_and_worker_presence() {
        let temp = TempDir::new("avail-presence");
        let opts = ready(&temp);

        let mut no_runtime = opts.clone();
        no_runtime.nemo_dir = Some(temp.join("nowhere"));
        assert_eq!(labels(no_runtime).availability(), Err(Unavailable::Runtime));

        let mut no_worker = opts.clone();
        no_worker.worker_exe = Some(temp.join("missing.exe"));
        assert_eq!(labels(no_worker).availability(), Err(Unavailable::Runtime));

        let mut no_model = opts.clone();
        no_model.registry = ModelRegistry::new(temp.join("other-data")).with_ownkey_models(None);
        let unmodelled = labels(no_model);
        assert_eq!(unmodelled.availability(), Err(Unavailable::Model));
        assert_eq!(unmodelled.diarizer().err(), Some(Unavailable::Model));
        // A Settings change to the model folder is seen at once.
        unmodelled.set_registry(opts.registry.clone());
        assert!(unmodelled.availability().is_ok());
    }

    #[test]
    fn the_development_runtime_needs_development_mode() {
        let temp = TempDir::new("avail-devdir");
        let mut opts = ready(&temp);
        opts.dev_mode = false;
        // Only the development folder holds a runtime, and it's ignored now.
        assert_eq!(labels(opts).availability(), Err(Unavailable::Runtime));
    }

    #[test]
    fn the_test_override_works_only_in_development_mode() {
        let temp = TempDir::new("avail-override");
        let opts = ready(&temp);
        let forced = |code: &str, dev_mode: bool| {
            let mut opts = opts.clone();
            opts.no_speakers = Some(code.into());
            opts.dev_mode = dev_mode;
            labels(opts).availability()
        };
        assert_eq!(forced("cpu", true).unwrap_err().code(), "cpu");
        assert_eq!(
            forced("cpu", true),
            Err(Unavailable::Cpu {
                missing: CpuFeatures::default()
                    .missing()
                    .into_iter()
                    .map(String::from)
                    .collect()
            })
        );
        assert_eq!(forced("runtime", true), Err(Unavailable::Runtime));
        assert_eq!(forced("model", true), Err(Unavailable::Model));
        assert!(forced("nonsense", true).is_ok());
        assert!(forced("", true).is_ok());
        // Outside development mode the variable means nothing: the answer is the real one.
        for code in ["cpu", "model"] {
            assert_ne!(forced(code, false).unwrap_err().code(), code);
        }
        // Even before the platform check.
        let mut other = opts.clone();
        other.no_speakers = Some("model".into());
        let elsewhere = SpeakerLabels::with_platform(other, false);
        assert_eq!(elsewhere.availability(), Err(Unavailable::Model));
    }

    #[test]
    fn options_read_the_environment_once() {
        let opts = SpeakerOptions::new("exe", ModelRegistry::new("data"));
        assert_eq!(opts.model, ModelSpec::nemotron());
        assert_eq!(opts.start_timeout, Duration::from_secs(10));
        assert_eq!(opts.load_timeout, Duration::from_secs(60));
        assert_eq!(opts.push_timeout, Duration::from_secs(30));
        assert_eq!(opts.finish_timeout, Duration::from_secs(60));
        assert!(!opts.dev_mode);
        let mut explicit = opts.clone();
        explicit.worker_exe = Some(PathBuf::from("w.exe"));
        assert_eq!(explicit.worker(), Some(PathBuf::from("w.exe")));
    }
}
