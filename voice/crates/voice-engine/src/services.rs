//! What the engine needs from the computer, behind traits so tests never touch the real
//! keyboard, clipboard, microphone, speakers, registry or network.
//!
//! [`Services::system`] wires the real ones; tests build a [`Services`] of fakes and start the
//! engine with [`crate::start_with`].

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use voice_core::local::{
    LocalEngine, LocalOptions, LocalStatus, Located, ModelError, ModelRegistry, ModelSpec, Progress,
};
use voice_core::providers::stream::{self, StreamingSpeech};
use voice_core::providers::{Activity, ProviderConfig};
use voice_core::speakers::{Diarizer, RuntimeInfo, SpeakerLabels, SpeakerOptions, Unavailable};
use voice_core::{ProviderError, Rewriter, SpeechToText};
use voice_input::{BeforeKeys, InputError, InsertMode, Key, TextTarget};
use voice_meetings::PipelineOptions;
use voice_sync::protect::Protector;
use voice_sync::{ScreenFiles, SyncTiming};

use crate::api::{EngineOptions, Locale, Theme};
use crate::media::{RecordedScreen, ScreenMedia};
use crate::meetings::controller::ControllerTiming;
use crate::meetings::screen::{ScreenTiming, SCREEN_EXE};

/// Whether a key is physically down.
pub type KeyState = Arc<dyn Fn(Key) -> bool + Send + Sync>;

/// Text into the focused app, and the selection out of it.
///
/// Both call `before_keys` right before every batch of keys they send, after any wait for the
/// clipboard (voice-input's `insert_text_with`). Dictation waits there while a hotkey is held.
pub trait TextInput: Send + Sync {
    fn insert(
        &self,
        text: &str,
        mode: InsertMode,
        before_keys: BeforeKeys,
    ) -> Result<(), InputError>;
    fn capture_selection(
        &self,
        timeout: Duration,
        before_keys: BeforeKeys,
    ) -> Result<Option<String>, InputError>;
    fn text_target(&self) -> TextTarget;
    /// Before quitting, so the user's clipboard comes back.
    fn wait_for_clipboard_restore(&self) {}
    /// Puts text on the clipboard hidden from clipboard history and cloud clipboard
    /// (voice-input's `copy_hidden`), for the recovery code.
    fn copy_hidden(&self, text: &str) -> Result<(), InputError> {
        let _ = text;
        Err(InputError::Unsupported)
    }
    /// Empties the clipboard while it still holds `text` (voice-input's `clear_if`).
    fn clear_if(&self, text: &str) -> Result<bool, InputError> {
        let _ = text;
        Err(InputError::Unsupported)
    }
}

/// Blocks of 16 kHz mono samples, as they are captured.
pub type AudioSink = Box<dyn FnMut(&[i16]) + Send>;

/// An open microphone. Dropping it stops the capture.
pub trait MicStream: Send {}

pub trait Microphone: Send + Sync {
    /// Opens the default microphone; blocks until it is open or failed. The error is an English
    /// sentence for the notification.
    fn open(&self, on_audio: AudioSink) -> Result<Box<dyn MicStream>, String>;
}

/// Speech and rewrite providers, and the local model.
pub trait Speech: Send + Sync {
    fn recognizer(&self, cfg: &ProviderConfig) -> Result<Arc<dyn SpeechToText>, ProviderError>;
    /// How a meeting with this provider is transcribed. Orukeet also times its words, which
    /// lets passages split at pauses; any other recognizer gives one passage per window. A
    /// provider that streams transcribes through [`Speech::streaming`] instead.
    fn meeting_recognizer(&self, cfg: &ProviderConfig) -> Result<PipelineOptions, ProviderError> {
        self.recognizer(cfg).map(PipelineOptions::new)
    }
    fn rewriter(&self, cfg: &ProviderConfig) -> Result<Arc<dyn Rewriter>, ProviderError>;
    fn list_models(
        &self,
        activity: Activity,
        cfg: &ProviderConfig,
    ) -> Result<Vec<String>, ProviderError>;
    /// Starts loading Orukeet in the background, at a press.
    fn local_preload(&self);
    fn local_status(&self) -> LocalStatus;
    /// Switches Orukeet to `registry` and loads it now. On failure the old location stays.
    fn local_load(&self, registry: ModelRegistry) -> Result<(), ProviderError>;
    /// Where Orukeet is looked for and how long it stays loaded, after a Settings change.
    fn local_configure(&self, registry: ModelRegistry, idle: Duration);
    fn local_unload(&self);
    fn download(
        &self,
        registry: &ModelRegistry,
        spec: &ModelSpec,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<Located, ModelError>;
    fn shutdown(&self);
    /// Nemotron on this computer, for one meeting. Never a cloud provider.
    fn diarizer(&self) -> Result<Arc<dyn Diarizer>, Unavailable> {
        Err(Unavailable::Platform)
    }
    /// Whether Nemotron can run now, for Settings. Starts no process.
    fn speaker_availability(&self) -> Result<RuntimeInfo, Unavailable> {
        Err(Unavailable::Platform)
    }
    /// The realtime stream for a provider that streams.
    fn streaming(&self, _cfg: &ProviderConfig) -> Result<Arc<dyn StreamingSpeech>, ProviderError> {
        Err(ProviderError::provider(
            "Streaming transcription isn't available.",
        ))
    }
}

pub trait Chime: Send + Sync {
    /// The ready chime: Windows' "Asterisk" sound, asynchronous.
    fn play(&self);
}

/// The HKCU Run value.
pub trait Autostart: Send + Sync {
    fn is_on(&self) -> bool;
    fn set(&self, on: bool) -> Result<(), String>;
}

pub trait Updates: Send + Sync {
    /// The newer Voice version on GitHub, `None` when this one is the newest.
    fn check(&self, current: &str) -> Result<Option<String>, String>;
}

/// What the engine reads from the system: the theme, the language, Ownkey's process, disk space.
pub trait System: Send + Sync {
    fn theme(&self) -> Theme;
    fn locale(&self) -> Locale;
    fn ownkey_running(&self) -> bool;
    fn free_bytes(&self, path: &Path) -> Option<u64>;
}

/// Everything [`crate::start_with`] runs on.
pub struct Services {
    /// `None` reads the real keyboard through voice-input's poller; tests inject the key state.
    pub key_state: Option<KeyState>,
    pub hotkey_interval: Duration,
    pub input: Arc<dyn TextInput>,
    pub microphone: Arc<dyn Microphone>,
    pub speech: Arc<dyn Speech>,
    pub chime: Arc<dyn Chime>,
    pub autostart: Arc<dyn Autostart>,
    pub updates: Arc<dyn Updates>,
    pub system: Arc<dyn System>,
    /// Ownkey's settings, read only by the import.
    pub ownkey_config: Option<PathBuf>,
    /// Ownkey's model folder, read only.
    pub ownkey_models: Option<PathBuf>,
    pub recorder_exe: PathBuf,
    /// `rukoo-voice-screen`. A path that doesn't exist means this Voice records no screen.
    pub screen_exe: PathBuf,
    /// `None`: PROTOCOL.md's path for this data folder (`RUKOO_VOICE_BRIDGE` still wins).
    pub discovery_path: Option<PathBuf>,
    pub controller: ControllerTiming,
    pub screen_timing: ScreenTiming,
    /// What the screen recording left in a meeting folder, for bucket sync.
    pub screen_files: Arc<dyn ScreenFiles>,
    /// The same, for media links: frames and video clips.
    pub screen_media: Arc<dyn ScreenMedia>,
    /// Keeps the bucket secret and the encryption key readable only by this Windows user.
    pub protector: Arc<dyn Protector>,
    /// Bucket sync's clocks and sizes; tests shorten them.
    pub sync_timing: SyncTiming,
}

impl Services {
    /// The real computer.
    pub fn system(options: &EngineOptions) -> Services {
        let ownkey_models = voice_core::paths::ownkey_models();
        let registry =
            ModelRegistry::new(&options.data_dir).with_ownkey_models(ownkey_models.clone());
        let mut local = LocalOptions::new(registry.clone());
        let worker = options.exe_dir.join(voice_core::paths::LOCAL_EXE_NAME);
        if worker.is_file() {
            local.worker_exe = Some(worker);
        }
        // The speaker label worker sits next to the app too; voice-core finds it, and its
        // runtime in `nemo_speech\` next to the exe.
        let mut speakers = SpeakerOptions::new(&options.exe_dir, registry);
        speakers.dev_mode = options.dev_mode;
        let screen = Arc::new(RecordedScreen::default());
        Services {
            key_state: None,
            hotkey_interval: voice_input::POLL_INTERVAL,
            input: Arc::new(SystemInput),
            microphone: Arc::new(crate::dictation::capture::DefaultMicrophone),
            speech: Arc::new(SystemSpeech {
                local: LocalEngine::new(local),
                speakers: SpeakerLabels::new(speakers),
            }),
            chime: Arc::new(crate::platform::SystemChime),
            autostart: crate::autostart::system(),
            updates: Arc::new(GitHubUpdates),
            system: Arc::new(crate::platform::SystemInfo),
            ownkey_config: voice_core::paths::ownkey_config(),
            ownkey_models,
            recorder_exe: options.exe_dir.join(RECORDER_EXE),
            screen_exe: options.exe_dir.join(SCREEN_EXE),
            discovery_path: None,
            controller: ControllerTiming::default(),
            screen_timing: ScreenTiming::default(),
            // The screen folder for both, sharing one frame index cache.
            screen_files: Arc::clone(&screen) as Arc<dyn ScreenFiles>,
            screen_media: screen,
            protector: voice_sync::protect::system(),
            sync_timing: SyncTiming::default(),
        }
    }
}

#[cfg(windows)]
pub const RECORDER_EXE: &str = "rukoo-voice-recorder.exe";
#[cfg(not(windows))]
pub const RECORDER_EXE: &str = "rukoo-voice-recorder";

struct SystemInput;

impl TextInput for SystemInput {
    fn insert(
        &self,
        text: &str,
        mode: InsertMode,
        before_keys: BeforeKeys,
    ) -> Result<(), InputError> {
        voice_input::insert_text_with(text, mode, before_keys)
    }

    fn capture_selection(
        &self,
        timeout: Duration,
        before_keys: BeforeKeys,
    ) -> Result<Option<String>, InputError> {
        voice_input::capture_selection_with(timeout, before_keys)
    }

    fn text_target(&self) -> TextTarget {
        voice_input::text_target()
    }

    fn wait_for_clipboard_restore(&self) {
        voice_input::wait_for_clipboard_restore();
    }

    fn copy_hidden(&self, text: &str) -> Result<(), InputError> {
        voice_input::copy_hidden(text)
    }

    fn clear_if(&self, text: &str) -> Result<bool, InputError> {
        voice_input::clear_if(text)
    }
}

struct SystemSpeech {
    local: LocalEngine,
    /// Nemotron on this computer: whether it can run, and a diarizer per meeting.
    speakers: SpeakerLabels,
}

impl Speech for SystemSpeech {
    fn recognizer(&self, cfg: &ProviderConfig) -> Result<Arc<dyn SpeechToText>, ProviderError> {
        voice_core::speech_to_text(cfg, &self.local).map(Arc::from)
    }

    fn meeting_recognizer(&self, cfg: &ProviderConfig) -> Result<PipelineOptions, ProviderError> {
        if stream::streams(cfg) {
            // A realtime model has no endpoint for recorded windows; the meeting's stream
            // transcribes it, so no window ever goes out this way. A window only asks for this
            // when a meeting that started with another provider keeps its way of transcribing
            // (voice-meetings stores it at the first start) after Settings switched to a
            // realtime model mid-meeting.
            return Err(ProviderError::config(
                "A realtime model only transcribes meetings that start with it. Choose the meeting provider this meeting started with to transcribe the rest of it.",
            ));
        }
        if cfg.provider == "orukeet" {
            // The same worker as dictation: one model in memory, one request at a time.
            return Ok(PipelineOptions::local(self.local.clone()));
        }
        self.recognizer(cfg).map(PipelineOptions::new)
    }

    fn rewriter(&self, cfg: &ProviderConfig) -> Result<Arc<dyn Rewriter>, ProviderError> {
        voice_core::rewriter(cfg).map(Arc::from)
    }

    fn list_models(
        &self,
        activity: Activity,
        cfg: &ProviderConfig,
    ) -> Result<Vec<String>, ProviderError> {
        voice_core::list_models(activity, cfg)
    }

    fn local_preload(&self) {
        self.local.preload();
    }

    fn local_status(&self) -> LocalStatus {
        self.local.status()
    }

    fn local_load(&self, registry: ModelRegistry) -> Result<(), ProviderError> {
        let before = self.local.registry();
        self.local.set_registry(registry);
        let result = self.local.load();
        if result.is_err() {
            self.local.set_registry(before);
        }
        result
    }

    fn local_configure(&self, registry: ModelRegistry, idle: Duration) {
        // Nemotron is looked for in the same places as Orukeet.
        self.speakers.set_registry(registry.clone());
        self.local.set_registry(registry);
        self.local.set_idle_timeout(idle);
    }

    fn local_unload(&self) {
        self.local.unload();
    }

    fn download(
        &self,
        registry: &ModelRegistry,
        spec: &ModelSpec,
        progress: &mut dyn FnMut(Progress),
        cancel: &AtomicBool,
    ) -> Result<Located, ModelError> {
        registry.download(spec, progress, cancel)
    }

    fn shutdown(&self) {
        self.local.shutdown();
    }

    fn diarizer(&self) -> Result<Arc<dyn Diarizer>, Unavailable> {
        self.speakers.diarizer()
    }

    fn speaker_availability(&self) -> Result<RuntimeInfo, Unavailable> {
        self.speakers.availability()
    }

    fn streaming(&self, cfg: &ProviderConfig) -> Result<Arc<dyn StreamingSpeech>, ProviderError> {
        voice_core::providers::streaming_speech(cfg).map(Arc::from)
    }
}

struct GitHubUpdates;

impl Updates for GitHubUpdates {
    fn check(&self, current: &str) -> Result<Option<String>, String> {
        voice_core::update::check(current)
            .map(|release| release.map(|r| r.version))
            .map_err(|e| e.to_string())
    }
}
