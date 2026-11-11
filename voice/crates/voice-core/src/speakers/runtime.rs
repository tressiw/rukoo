//! Where the NeMo-Speech.cpp runtime is: Voice's own copy next to the exe, checked against the
//! committed manifest, or Ownkey's copy when its files match Ownkey's known hashes.
//!
//! The order is fixed: a development copy's `RUKOO_VOICE_NEMO_DIR`, then Voice's own `nemo_speech/`
//! (in a macOS bundle under `Contents/Resources/`), then the installed Ownkey's folder, read only.
//! Voice never writes in any of them.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::api::RUNTIME_FOLDER;
use crate::local::sha256_file;
use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeLocation {
    /// Next to Voice, or in its bundle's `Contents/Resources` on macOS.
    Voice,
    Ownkey,
    Dev,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub dir: PathBuf,
    pub location: RuntimeLocation,
    /// The NeMo-Speech.cpp commit the runtime was built from, as its 7-character short hash.
    /// Empty for a development folder that doesn't say.
    pub commit: String,
}

impl RuntimeInfo {
    /// Folders besides `dir` that the worker searches for the runtime's own imports: for
    /// Ownkey's copy, its C runtime one folder up, which the worker's DLL search wouldn't find
    /// on a PC without a system copy.
    pub fn dll_dirs(&self) -> Vec<PathBuf> {
        match self.location {
            RuntimeLocation::Ownkey => self
                .dir
                .parent()
                .map(Path::to_path_buf)
                .into_iter()
                .collect(),
            RuntimeLocation::Voice | RuntimeLocation::Dev => Vec::new(),
        }
    }
}

/// voice/nemo-runtime/manifest.json: the files of Voice's own runtime build and their hashes.
/// build.rs copies it, or the manifest a test package names (`package.ps1 -RuntimeManifest`).
pub(crate) const MANIFEST: &str = include_str!(concat!(env!("OUT_DIR"), "/nemo-runtime.json"));

/// The manifest entry for the platform this build runs on. Every platform has its own runtime:
/// Windows on x86-64 the DLLs, macOS on Apple Silicon the dylibs. A build for another platform
/// has no runtime at all, so it looks for an entry no manifest has.
#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) const TARGET: &str = "windows-x86_64";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(crate) const TARGET: &str = "macos-aarch64";
#[cfg(not(any(
    all(windows, target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
)))]
pub(crate) const TARGET: &str = "unsupported";

/// The five DLLs of every Windows runtime folder. The worker loads the first; it loads the others.
#[cfg(windows)]
pub(crate) const RUNTIME_DLLS: [&str; 5] = [
    "nemo_speech_asr_c.dll",
    "nemo_speech_asr.dll",
    "ggml.dll",
    "ggml-base.dll",
    "ggml-cpu.dll",
];

/// The six dylibs of every macOS runtime folder, the C ABI first. There is no C runtime: libc++
/// and libSystem come with macOS, and Accelerate is imported by the two ggml backends that use it.
#[cfg(not(windows))]
pub(crate) const RUNTIME_DLLS: [&str; 6] = [
    "libnemo_speech_asr_c.dylib",
    "libnemo_speech_asr.dylib",
    "libggml.dylib",
    "libggml-base.dylib",
    "libggml-cpu.dylib",
    "libggml-blas.dylib",
];

/// The Visual C++ runtime the five DLLs import. Voice's own folder holds it, pinned by the
/// manifest; Ownkey's sits one folder up, in Ownkey's `_internal\`, pinned by [`OWNKEY_CRT`].
/// Ownkey's fallback is a Windows one, so this is Windows-only, and only the tests name it.
#[cfg(all(windows, test))]
const CRT_DLLS: [&str; 3] = ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"];

/// The NeMo-Speech.cpp commit of Ownkey's runtime: the same one Voice builds.
const OWNKEY_COMMIT: &str = "97a15afa5caa9bce5baaa86c1184103877af4101";

/// The installed Ownkey's build of that commit (2026-10-02): size and SHA-256 per DLL, from its
/// `runtime.json`. Its MSVC runtime sits one folder up and is pinned too ([`OWNKEY_CRT`]).
const OWNKEY_RUNTIME: [(&str, u64, &str); 5] = [
    (
        "nemo_speech_asr_c.dll",
        56_320,
        "c45eeb7c158d40f1b523b6a306376ad06a339b7a0ec53722fb0a5790dbffd51d",
    ),
    (
        "nemo_speech_asr.dll",
        4_195_840,
        "68da916f37fe8b1bcc051c6e9d161ce7bcfb822fe5ecdbe8a559d59146e6eaf0",
    ),
    (
        "ggml.dll",
        67_072,
        "e7fbe5d6eb20ae2a5056af4050016ad9a487b7fa8f2590657a0f3942a98210c5",
    ),
    (
        "ggml-base.dll",
        623_616,
        "86ce9be60062561491c4febbec02b5664d84eb7a100c5f55f75f374fa99eb2dc",
    ),
    (
        "ggml-cpu.dll",
        786_432,
        "aaecc8a7984d92ca5ec1d3a15a54fcb371e15fb912626d2d31e8bb86cc9e415c",
    ),
];

/// The Visual C++ runtime in that Ownkey install's `_internal\`, one folder above its runtime:
/// size and SHA-256 per DLL (msvcp140 14.51.36247.0, vcruntime140 and vcruntime140_1
/// 14.44.35211.0). The worker searches that folder for the five DLLs' imports, so a copy that
/// isn't this one would run in the worker while the five still match. An Ownkey update that
/// brings another C runtime makes the fallback stop counting, as other DLL hashes do.
const OWNKEY_CRT: [(&str, u64, &str); 3] = [
    (
        "msvcp140.dll",
        643_512,
        "7c26614e1d733892c2deac7e245ce115504b1d80592dd0a01b08e3e5a55f89ca",
    ),
    (
        "vcruntime140.dll",
        124_544,
        "d5e4d9a3e835fa679450145d6a7d94e36573a509317111904d9b3712c30d9066",
    ),
    (
        "vcruntime140_1.dll",
        49_792,
        "1f2d41c4aa5db0bc33ebf7b66d72943a817d7ce6cbe880502a9403823633093f",
    ),
];

/// The parts of `voice/nemo-runtime/manifest.json` the lookup reads.
///
/// Format 2 holds one entry per target under `targets`, so a Windows build and a macOS build pin
/// their own files and archive while the commit stays shared. Format 1, which the Windows build
/// used before macOS existed, is a single Windows entry; it is still read so that an older
/// `pack.ps1` artifact keeps verifying on Windows.
///
/// Until a target's runtime is published its `archive` is null and its `files` empty, and then
/// Voice's own copy never verifies for that target.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct Manifest {
    #[serde(default)]
    pub format: u64,
    #[serde(default)]
    pub commit: String,
    /// Format 2: the entry per target.
    #[serde(default)]
    pub targets: BTreeMap<String, Target>,
    /// Format 1: the Windows files.
    #[serde(default)]
    pub files: Option<Vec<ManifestFile>>,
}

/// One target's files in a format 2 manifest.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct Target {
    #[serde(default)]
    pub files: Option<Vec<ManifestFile>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct ManifestFile {
    /// Relative to the runtime folder, with `/` between parts.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

impl Manifest {
    pub(crate) fn parse(text: &str) -> Option<Manifest> {
        serde_json::from_str(text).ok()
    }

    /// The files this platform's runtime must hold. Empty when the manifest pins no runtime for
    /// it, which verifies nothing.
    fn files(&self) -> &[ManifestFile] {
        let files = match self.format {
            1 if TARGET == "windows-x86_64" => self.files.as_deref(),
            2 => self
                .targets
                .get(TARGET)
                .and_then(|target| target.files.as_deref()),
            _ => None,
        };
        files.unwrap_or_default()
    }
}

/// The compiled-in manifest. A manifest Voice can't read verifies nothing.
pub(crate) fn manifest() -> &'static Manifest {
    static PARSED: OnceLock<Manifest> = OnceLock::new();
    PARSED.get_or_init(|| Manifest::parse(MANIFEST).unwrap_or_default())
}

/// Finds a runtime folder that verifies: the development override, then Voice's own folder (next to
/// the exe, or in its bundle's Resources on macOS), then Ownkey's. Hashes are cached by path, size
/// and modification time.
pub fn locate_runtime(exe_dir: &Path, dev_mode: bool) -> Option<RuntimeInfo> {
    let dev = dev_dir_in(dev_mode, &paths::env_var);
    let ownkey = paths::ownkey_nemo_runtime();
    locate_in(exe_dir, dev.as_deref(), ownkey.as_deref(), manifest())
}

/// `RUKOO_VOICE_NEMO_DIR`, in development mode only.
pub(crate) fn dev_dir_in(dev_mode: bool, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if !dev_mode {
        return None;
    }
    env(paths::NEMO_DIR_ENV)
        .filter(|dir| !dir.trim().is_empty())
        .map(PathBuf::from)
}

/// The lookup with every input explicit: `dev` is a development folder (already gated on
/// development mode), `ownkey` Ownkey's runtime folder.
pub(crate) fn locate_in(
    exe_dir: &Path,
    dev: Option<&Path>,
    ownkey: Option<&Path>,
    manifest: &Manifest,
) -> Option<RuntimeInfo> {
    if let Some(dir) = dev.filter(|dir| dev_runtime_present(dir)) {
        return Some(RuntimeInfo {
            dir: dir.to_path_buf(),
            location: RuntimeLocation::Dev,
            commit: dev_commit(dir),
        });
    }
    for dir in runtime_candidates(exe_dir) {
        if verify_voice(&dir, manifest) {
            return Some(RuntimeInfo {
                dir,
                location: RuntimeLocation::Voice,
                commit: short_commit(&manifest.commit),
            });
        }
    }
    if let Some(dir) = ownkey.filter(|dir| verify_ownkey(dir)) {
        return Some(RuntimeInfo {
            dir: dir.to_path_buf(),
            location: RuntimeLocation::Ownkey,
            commit: short_commit(OWNKEY_COMMIT),
        });
    }
    None
}

fn short_commit(commit: &str) -> String {
    commit.trim().chars().take(7).collect()
}

fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Where Voice keeps its own runtime, in the order it is tried: a bundle keeps it in
/// `Contents/Resources`, where codesign reads the licences and `runtime.json` as plain files
/// instead of unsigned nested code; an unbundled build keeps it next to the exe.
#[cfg(target_os = "macos")]
fn runtime_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(contents) = exe_dir.parent() {
        candidates.push(contents.join("Resources").join(RUNTIME_FOLDER));
    }
    candidates.push(exe_dir.join(RUNTIME_FOLDER));
    candidates
}

/// Where Voice keeps its own runtime: `nemo_speech/` next to the exe.
#[cfg(not(target_os = "macos"))]
fn runtime_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    vec![exe_dir.join(RUNTIME_FOLDER)]
}

/// A development folder needs this platform's runtime files; its hashes aren't checked.
fn dev_runtime_present(dir: &Path) -> bool {
    !RUNTIME_DLLS.is_empty()
        && RUNTIME_DLLS
            .iter()
            .all(|name| fs::metadata(dir.join(name)).is_ok_and(|meta| meta.is_file()))
}

/// The commit a development folder's `runtime.json` names, if any.
fn dev_commit(dir: &Path) -> String {
    fs::read(dir.join("runtime.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("commit")?.as_str().map(short_commit))
        .unwrap_or_default()
}

/// Voice's own copy: every file of this platform's manifest entry, at its size and hash. An empty
/// entry, a manifest of another format, or a folder that is a link never verifies.
pub(crate) fn verify_voice(dir: &Path, manifest: &Manifest) -> bool {
    let files = manifest.files();
    matches!(manifest.format, 1 | 2)
        && !files.is_empty()
        && !is_link(dir)
        && files.iter().all(|file| {
            relative(&file.path)
                .is_some_and(|path| file_matches(&dir.join(path), file.size, &file.sha256))
        })
}

/// Ownkey's copy: its five DLLs and its C runtime one folder up, all at the known sizes and
/// hashes.
pub(crate) fn verify_ownkey(dir: &Path) -> bool {
    verify_ownkey_with(dir, &OWNKEY_RUNTIME, &OWNKEY_CRT)
}

/// The folder above `dir` has to hold the C runtime the DLLs import, matching `crt`: without it
/// they load only on a PC that has a system copy, and the worker searches that folder, so an
/// unknown copy there would run in the worker.
fn verify_ownkey_with(dir: &Path, known: &[(&str, u64, &str)], crt: &[(&str, u64, &str)]) -> bool {
    verify_known(dir, known) && dir.parent().is_some_and(|up| verify_known(up, crt))
}

fn verify_known(dir: &Path, known: &[(&str, u64, &str)]) -> bool {
    !is_link(dir)
        && known
            .iter()
            .all(|(name, size, sha256)| file_matches(&dir.join(name), *size, sha256))
}

/// A manifest path as a relative path, refusing anything that could leave the folder.
fn relative(path: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = path.split('/').collect();
    let unsafe_part = |part: &&str| {
        part.is_empty() || *part == "." || *part == ".." || part.contains(['\\', ':'])
    };
    if parts.iter().any(unsafe_part) {
        return None;
    }
    Some(parts.iter().collect())
}

struct Hashed {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
    sha256: String,
}

/// Hashes already computed in this process. Settings asks on every refresh, and Ownkey's runtime
/// alone is 5.7 MB.
static HASHES: Mutex<Vec<Hashed>> = Mutex::new(Vec::new());

/// A regular file (not a link) with this size and SHA-256.
fn file_matches(path: &Path, size: u64, sha256: &str) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_file() || meta.len() != size {
        return false;
    }
    let modified = meta.modified().ok();
    let cached = HASHES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|hashed| hashed.path == path && hashed.size == size && hashed.modified == modified)
        .map(|hashed| hashed.sha256.clone());
    let actual = match cached {
        Some(actual) => actual,
        None => {
            let Ok(actual) = sha256_file(path, &AtomicBool::new(false)) else {
                return false;
            };
            let mut hashes = HASHES.lock().unwrap_or_else(PoisonError::into_inner);
            hashes.retain(|hashed| hashed.path != path);
            hashes.push(Hashed {
                path: path.to_path_buf(),
                size,
                modified,
                sha256: actual.clone(),
            });
            actual
        }
    };
    actual.eq_ignore_ascii_case(sha256.trim())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::speakers::tests::TempDir;

    fn sha(path: &Path) -> String {
        sha256_file(path, &AtomicBool::new(false)).unwrap()
    }

    /// A runtime folder with this platform's runtime files and a licence, plus the format 2
    /// manifest that pins it.
    fn runtime_folder(dir: &Path) -> Manifest {
        fs::create_dir_all(dir.join("licenses")).unwrap();
        let mut files = Vec::new();
        for (i, name) in RUNTIME_DLLS.iter().chain(&["licenses/LICENSE"]).enumerate() {
            let path = dir.join(relative(name).unwrap());
            fs::write(&path, format!("fake file {i}: {name}")).unwrap();
            files.push(ManifestFile {
                path: name.to_string(),
                size: fs::metadata(&path).unwrap().len(),
                sha256: sha(&path).to_uppercase(),
            });
        }
        manifest_with(files)
    }

    /// A format 2 manifest whose entry for this build's target pins `files`.
    fn manifest_with(files: Vec<ManifestFile>) -> Manifest {
        Manifest {
            format: 2,
            commit: OWNKEY_COMMIT.into(),
            targets: BTreeMap::from([(TARGET.to_string(), Target { files: Some(files) })]),
            files: None,
        }
    }

    #[test]
    fn the_committed_manifest_pins_every_target() {
        let committed = Manifest::parse(MANIFEST).expect("voice/nemo-runtime/manifest.json");
        assert_eq!(committed.format, 2);
        assert_eq!(committed.commit, OWNKEY_COMMIT);
        assert_eq!(manifest(), &committed);
        assert!(committed.targets.contains_key("windows-x86_64"));
        assert!(committed.targets.contains_key("macos-aarch64"));
        if TARGET == "windows-x86_64" || TARGET == "macos-aarch64" {
            let pinned = committed.targets[TARGET].files.clone().unwrap_or_default();
            assert_eq!(manifest().files(), pinned.as_slice());
        }
        // A target whose runtime isn't published pins no files, or null: nothing verifies.
        let unpublished = Manifest::parse(
            r#"{"format": 2, "commit": "97a15afa", "targets": {"macos-aarch64": {"archive": null, "files": []}}}"#,
        )
        .unwrap();
        assert!(unpublished.files().is_empty());
        let null_files = Manifest::parse(
            r#"{"format": 2, "commit": "97a15afa", "targets": {"macos-aarch64": {"files": null}}}"#,
        )
        .unwrap();
        assert!(null_files.files().is_empty());
        // Format 1, what the Windows build wrote before the manifest had targets, is still read
        // there, and only there.
        let legacy = Manifest::parse(
            r#"{"format": 1, "commit": "97a15afa", "files": [{"path": "msvcp140.dll", "size": 3, "sha256": "cd", "version": "14.44"}]}"#,
        )
        .unwrap();
        if TARGET == "windows-x86_64" {
            assert_eq!(legacy.files()[0].path, "msvcp140.dll");
        } else {
            assert!(
                legacy.files().is_empty(),
                "a format 1 manifest is the Windows entry"
            );
        }
    }

    #[test]
    fn a_target_without_pinned_files_never_verifies() {
        let temp = TempDir::new("runtime-unpublished");
        let dir = temp.join(RUNTIME_FOLDER);
        let pinned = runtime_folder(&dir);
        assert!(verify_voice(&dir, &pinned));
        let mut unpublished = pinned;
        unpublished.targets.get_mut(TARGET).unwrap().files = Some(Vec::new());
        assert!(!verify_voice(&dir, &unpublished));
        assert!(!verify_voice(&dir, &Manifest::default()));
    }

    #[test]
    fn voices_copy_verifies_against_its_manifest() {
        let temp = TempDir::new("runtime-voice");
        let exe_dir = temp.path();
        let dir = exe_dir.join(RUNTIME_FOLDER);
        let pinned = runtime_folder(&dir);
        assert!(verify_voice(&dir, &pinned));
        let found = locate_in(exe_dir, None, None, &pinned).unwrap();
        assert_eq!(
            found,
            RuntimeInfo {
                dir: dir.clone(),
                location: RuntimeLocation::Voice,
                commit: "97a15af".into(),
            }
        );
        // Another format isn't read as this one.
        let other = Manifest {
            format: 3,
            ..pinned.clone()
        };
        assert!(!verify_voice(&dir, &other));
        // A path that leaves the folder never verifies.
        let mut escaping = pinned.clone();
        escaping
            .targets
            .get_mut(TARGET)
            .unwrap()
            .files
            .as_mut()
            .unwrap()[5]
            .path = "../LICENSE".into();
        assert!(!verify_voice(&dir, &escaping));
    }

    /// A macOS bundle keeps the runtime in Contents/Resources; the exe lives in Contents/MacOS.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_finds_the_runtime_in_its_resources() {
        let temp = TempDir::new("runtime-bundle");
        let contents = temp.join("Rukoo Voice.app").join("Contents");
        let exe_dir = contents.join("MacOS");
        fs::create_dir_all(&exe_dir).unwrap();
        let dir = contents.join("Resources").join(RUNTIME_FOLDER);
        let pinned = runtime_folder(&dir);
        assert_eq!(
            locate_in(&exe_dir, None, None, &pinned).unwrap(),
            RuntimeInfo {
                dir,
                location: RuntimeLocation::Voice,
                commit: "97a15af".into(),
            }
        );
        // An unbundled build still finds the folder next to its exe.
        let unbundled = temp.join("unbundled");
        fs::create_dir_all(&unbundled).unwrap();
        let pinned = runtime_folder(&unbundled.join(RUNTIME_FOLDER));
        assert_eq!(
            locate_in(&unbundled, None, None, &pinned).unwrap().location,
            RuntimeLocation::Voice
        );
    }

    #[test]
    fn one_changed_byte_fails_even_with_a_warm_cache() {
        let temp = TempDir::new("runtime-changed");
        let dir = temp.join(RUNTIME_FOLDER);
        let pinned = runtime_folder(&dir);
        assert!(verify_voice(&dir, &pinned));
        let dll = dir.join(
            *RUNTIME_DLLS
                .iter()
                .find(|name| name.contains("ggml-cpu"))
                .unwrap(),
        );
        let mut bytes = fs::read(&dll).unwrap();
        bytes[0] ^= 0x01;
        fs::write(&dll, &bytes).unwrap();
        // Same size; a new modification time makes the cache hash again.
        let file = fs::OpenOptions::new().write(true).open(&dll).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(30))
            .unwrap();
        drop(file);
        assert!(!verify_voice(&dir, &pinned));
        assert_eq!(locate_in(temp.path(), None, None, &pinned), None);
        // A missing file fails too.
        bytes[0] ^= 0x01;
        fs::write(&dll, &bytes).unwrap();
        fs::remove_file(dir.join("licenses").join("LICENSE")).unwrap();
        assert!(!verify_voice(&dir, &pinned));
    }

    /// Ownkey's runtime is a Windows install: these pins and the fallback only exist there.
    #[cfg(windows)]
    #[test]
    fn ownkeys_folder_with_other_hashes_is_skipped() {
        let temp = TempDir::new("runtime-ownkey");
        let ownkey = temp.join("ownkey").join("nemo_speech");
        fs::create_dir_all(&ownkey).unwrap();
        // The right names and sizes, other bytes.
        for (name, size, _) in OWNKEY_RUNTIME {
            fs::write(ownkey.join(name), vec![0u8; size as usize]).unwrap();
        }
        assert!(!verify_ownkey(&ownkey));
        assert_eq!(
            locate_in(&temp.join("exe"), None, Some(&ownkey), &Manifest::default()),
            None
        );
        // The same rule with hashes this test knows does find it.
        let known: Vec<(String, u64, String)> = OWNKEY_RUNTIME
            .iter()
            .map(|(name, size, _)| (name.to_string(), *size, sha(&ownkey.join(name))))
            .collect();
        let known: Vec<(&str, u64, &str)> = known
            .iter()
            .map(|(name, size, sha)| (name.as_str(), *size, sha.as_str()))
            .collect();
        assert!(verify_known(&ownkey, &known));
    }

    /// Ownkey's runtime is a Windows install: these pins and the fallback only exist there.
    #[cfg(windows)]
    #[test]
    fn ownkeys_folder_counts_only_with_its_c_runtime_one_folder_up() {
        let temp = TempDir::new("runtime-ownkey-crt");
        let internal = temp.join("_internal");
        let ownkey = internal.join("nemo_speech");
        fs::create_dir_all(&ownkey).unwrap();
        for (i, name) in RUNTIME_DLLS.iter().enumerate() {
            fs::write(ownkey.join(name), format!("fake dll {i}")).unwrap();
        }
        let known: Vec<(String, u64, String)> = RUNTIME_DLLS
            .iter()
            .map(|name| {
                let path = ownkey.join(name);
                (
                    name.to_string(),
                    fs::metadata(&path).unwrap().len(),
                    sha(&path),
                )
            })
            .collect();
        let known: Vec<(&str, u64, &str)> = known
            .iter()
            .map(|(name, size, sha)| (name.as_str(), *size, sha.as_str()))
            .collect();
        assert!(verify_known(&ownkey, &known));
        for (i, name) in CRT_DLLS.iter().enumerate() {
            fs::write(internal.join(name), format!("crt {i}")).unwrap();
        }
        let crt: Vec<(String, u64, String)> = CRT_DLLS
            .iter()
            .map(|name| {
                let path = internal.join(name);
                (
                    name.to_string(),
                    fs::metadata(&path).unwrap().len(),
                    sha(&path),
                )
            })
            .collect();
        let crt: Vec<(&str, u64, &str)> = crt
            .iter()
            .map(|(name, size, sha)| (name.as_str(), *size, sha.as_str()))
            .collect();
        assert!(verify_ownkey_with(&ownkey, &known, &crt));
        // A C runtime DLL that isn't the known one, at the same size: the worker would load it
        // while the five still match, so the folder doesn't count. Each rewrite gets its own
        // modification time, as a real one would: the hash cache keys on size and time, and a
        // coarse file system clock can give two quick writes the same time.
        let rewrite = |bytes: &str, later: u64| {
            let path = internal.join(CRT_DLLS[1]);
            fs::write(&path, bytes).unwrap();
            let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::now() + Duration::from_secs(later))
                .unwrap();
        };
        rewrite("crt 9", 30);
        assert!(
            !verify_ownkey_with(&ownkey, &known, &crt),
            "a changed C runtime DLL"
        );
        rewrite("crt 1", 60);
        assert!(verify_ownkey_with(&ownkey, &known, &crt));
        // Nothing the DLLs import is there.
        fs::remove_file(internal.join(CRT_DLLS[2])).unwrap();
        assert!(
            !verify_ownkey_with(&ownkey, &known, &crt),
            "one of three is missing"
        );
        fs::write(internal.join(CRT_DLLS[2]), "crt 2").unwrap();
        assert!(verify_ownkey_with(&ownkey, &known, &crt));
        // The real pins never match these stand-ins.
        assert!(!verify_ownkey(&ownkey));
        // The worker searches that folder too.
        let info = RuntimeInfo {
            dir: ownkey.clone(),
            location: RuntimeLocation::Ownkey,
            commit: "97a15af".into(),
        };
        assert_eq!(info.dll_dirs(), vec![internal.clone()]);
        for location in [RuntimeLocation::Voice, RuntimeLocation::Dev] {
            assert!(RuntimeInfo {
                location,
                ..info.clone()
            }
            .dll_dirs()
            .is_empty());
        }
    }

    #[test]
    fn voices_copy_comes_before_ownkeys_and_dev_before_both() {
        let temp = TempDir::new("runtime-order");
        let exe_dir = temp.join("exe");
        let pinned = runtime_folder(&exe_dir.join(RUNTIME_FOLDER));
        let dev = temp.join("dev");
        runtime_folder(&dev);
        fs::write(
            dev.join("runtime.json"),
            r#"{"commit": "0123456789abcdef"}"#,
        )
        .unwrap();
        let found = locate_in(&exe_dir, Some(&dev), None, &pinned).unwrap();
        assert_eq!(found.location, RuntimeLocation::Dev);
        assert_eq!(found.dir, dev);
        assert_eq!(found.commit, "0123456");
        // A development folder without the DLLs falls through to Voice's copy.
        let empty = temp.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let found = locate_in(&exe_dir, Some(&empty), None, &pinned).unwrap();
        assert_eq!(found.location, RuntimeLocation::Voice);
    }

    #[test]
    fn the_development_folder_is_ignored_outside_development_mode() {
        let env: HashMap<String, String> =
            HashMap::from([(paths::NEMO_DIR_ENV.to_string(), "E:\\nemo".to_string())]);
        let lookup = |name: &str| env.get(name).cloned();
        assert_eq!(dev_dir_in(false, &lookup), None);
        assert_eq!(dev_dir_in(true, &lookup), Some(PathBuf::from("E:\\nemo")));
        let blank = |_: &str| Some("  ".to_string());
        assert_eq!(dev_dir_in(true, &blank), None);
    }

    /// Ownkey's runtime is a Windows install: these pins and the fallback only exist there.
    #[cfg(windows)]
    #[test]
    fn ownkeys_pins_are_lower_case_sha256() {
        for (name, size, sha) in OWNKEY_RUNTIME.into_iter().chain(OWNKEY_CRT) {
            assert!(RUNTIME_DLLS.contains(&name) || CRT_DLLS.contains(&name));
            assert!(size > 0);
            assert_eq!(sha.len(), 64);
            assert!(sha
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }
    }
}
