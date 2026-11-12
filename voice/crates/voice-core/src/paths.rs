//! Where Voice keeps its files, and where Ownkey keeps the files Voice reads.
//!
//! The `*_in` functions are pure: they take the target platform and an environment lookup and
//! build the path with that platform's separators, so every platform's rules can be tested on
//! any host. The functions without the suffix apply them to the running system.

use std::path::{Path, PathBuf};

/// Set and non-empty: Voice keeps everything in this folder and runs in development mode.
pub const DATA_DIR_ENV: &str = "RUKOO_VOICE_DATA_DIR";
/// Set and non-empty: the full path of the local speech worker, instead of the one next to the exe.
pub const LOCAL_EXE_ENV: &str = "RUKOO_VOICE_LOCAL_EXE";

/// Set and non-empty: the full path of the speaker label worker, instead of the one next to the exe.
pub const SPEAKERS_EXE_ENV: &str = "RUKOO_VOICE_SPEAKERS_EXE";
/// A development copy only: the NeMo-Speech.cpp runtime folder to use, unchecked.
pub const NEMO_DIR_ENV: &str = "RUKOO_VOICE_NEMO_DIR";
/// A development copy only: `cpu`, `runtime` or `model` makes speaker labels unavailable for
/// that reason, for the live run and screenshots.
pub const NO_SPEAKERS_ENV: &str = "RUKOO_VOICE_TEST_NO_SPEAKERS";

#[cfg(windows)]
pub const LOCAL_EXE_NAME: &str = "rukoo-voice-local.exe";
#[cfg(not(windows))]
pub const LOCAL_EXE_NAME: &str = "rukoo-voice-local";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }

    fn separator(self) -> char {
        match self {
            Platform::Windows => '\\',
            Platform::MacOs | Platform::Linux => '/',
        }
    }
}

/// An environment lookup in which an empty variable counts as unset, as PROTOCOL.md requires.
fn var(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    env(name).filter(|value| !value.is_empty())
}

fn system_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn join(platform: Platform, base: &str, parts: &[&str]) -> String {
    let sep = platform.separator();
    let mut path = base.trim_end_matches(['/', '\\']).to_string();
    if path.is_empty() {
        // A root such as "/" would otherwise lose its only character.
        path.push(sep);
    }
    for part in parts {
        if !path.ends_with(sep) {
            path.push(sep);
        }
        path.push_str(part);
    }
    path
}

fn home(platform: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    match platform {
        Platform::Windows => var(env, "USERPROFILE").or_else(|| var(env, "HOME")),
        Platform::MacOs | Platform::Linux => var(env, "HOME"),
    }
}

/// Development mode: `RUKOO_VOICE_DATA_DIR` is set. Default hotkeys become F13 and F14, the
/// Ownkey import only runs on request and autostart is never registered.
pub fn dev_mode_in(env: &dyn Fn(&str) -> Option<String>) -> bool {
    var(env, DATA_DIR_ENV).is_some()
}

pub fn dev_mode() -> bool {
    dev_mode_in(&system_env)
}

/// Voice's data folder: config, bridge files, meetings, Voice's own model downloads and logs.
pub fn data_dir_in(platform: Platform, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(dir) = var(env, DATA_DIR_ENV) {
        return Some(dir);
    }
    match platform {
        Platform::Windows => {
            let base = var(env, "LOCALAPPDATA").or_else(|| {
                home(platform, env).map(|h| join(platform, &h, &["AppData", "Local"]))
            })?;
            Some(join(platform, &base, &["Rukoo Voice"]))
        }
        Platform::MacOs => {
            let home = home(platform, env)?;
            Some(join(
                platform,
                &home,
                &["Library", "Application Support", "Rukoo Voice"],
            ))
        }
        Platform::Linux => {
            let base = var(env, "XDG_STATE_HOME").or_else(|| {
                home(platform, env).map(|h| join(platform, &h, &[".local", "state"]))
            })?;
            Some(join(platform, &base, &["rukoo-voice"]))
        }
    }
}

pub fn data_dir() -> Option<PathBuf> {
    data_dir_in(Platform::current(), &system_env).map(PathBuf::from)
}

pub fn config_file(data_dir: &Path) -> PathBuf {
    data_dir.join("config.json")
}

/// Models Voice downloaded itself. "Remove download" only ever works in here.
pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

pub fn meetings_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("meetings")
}

pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Ownkey's settings file. Voice only ever reads it.
pub fn ownkey_config_in(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    match platform {
        Platform::Windows => {
            let base = var(env, "APPDATA").or_else(|| home(platform, env))?;
            Some(join(platform, &base, &["Ownkey", "config.json"]))
        }
        Platform::Linux => {
            let base = var(env, "XDG_CONFIG_HOME")
                .or_else(|| home(platform, env).map(|h| join(platform, &h, &[".config"])))?;
            Some(join(platform, &base, &["ownkey", "config.json"]))
        }
        // Ownkey never ran on macOS.
        Platform::MacOs => None,
    }
}

pub fn ownkey_config() -> Option<PathBuf> {
    ownkey_config_in(Platform::current(), &system_env).map(PathBuf::from)
}

/// Ownkey's model folder. Voice uses the models in it and never writes, moves or deletes anything there.
pub fn ownkey_models_in(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    match platform {
        Platform::Windows => {
            let base = var(env, "LOCALAPPDATA").or_else(|| {
                home(platform, env).map(|h| join(platform, &h, &["AppData", "Local"]))
            })?;
            Some(join(platform, &base, &["Ownkey", "models"]))
        }
        Platform::Linux => {
            let base = var(env, "XDG_CACHE_HOME")
                .or_else(|| home(platform, env).map(|h| join(platform, &h, &[".cache"])))?;
            Some(join(platform, &base, &["ownkey", "models"]))
        }
        Platform::MacOs => None,
    }
}

pub fn ownkey_models() -> Option<PathBuf> {
    ownkey_models_in(Platform::current(), &system_env).map(PathBuf::from)
}

/// The NeMo-Speech.cpp runtime inside the installed Ownkey. Voice loads it read only, and only
/// when its DLLs match Ownkey's known build.
pub fn ownkey_nemo_runtime_in(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    match platform {
        Platform::Windows => {
            let base = var(env, "LOCALAPPDATA").or_else(|| {
                home(platform, env).map(|h| join(platform, &h, &["AppData", "Local"]))
            })?;
            Some(join(
                platform,
                &base,
                &["Programs", "Ownkey", "backend", "_internal", "nemo_speech"],
            ))
        }
        // Ownkey's speaker labels only ever shipped for Windows.
        Platform::MacOs | Platform::Linux => None,
    }
}

pub fn ownkey_nemo_runtime() -> Option<PathBuf> {
    ownkey_nemo_runtime_in(Platform::current(), &system_env).map(PathBuf::from)
}

/// A non-empty environment variable of the running process.
pub(crate) fn env_var(name: &str) -> Option<String> {
    var(&system_env, name)
}

/// The local speech worker: `RUKOO_VOICE_LOCAL_EXE`, otherwise `rukoo-voice-local` next to the running exe.
pub fn local_worker_exe() -> Option<PathBuf> {
    if let Some(path) = var(&system_env, LOCAL_EXE_ENV) {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(LOCAL_EXE_NAME))
}

/// The speaker label worker: `RUKOO_VOICE_SPEAKERS_EXE`, otherwise `rukoo-voice-speakers` next to
/// the running exe.
pub fn speakers_worker_exe() -> Option<PathBuf> {
    if let Some(path) = var(&system_env, SPEAKERS_EXE_ENV) {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join(crate::speakers::SPEAKERS_EXE_NAME))
}
