//! The Rukoo Voice screen recorder: captures one monitor into the meeting's `screen/` folder as
//! frames with the text on them and, in video mode, a fragmented MP4, on the recorder's meeting
//! clock (`voice/RECORDING.md`, "The screen folder").
//!
//! The binary `rukoo-voice-screen` runs as its own process, so a graphics driver, encoder or OCR
//! crash never costs audio.
//!
//! `record` and `video::thread` are the orchestration and build on every platform; everything an
//! operating system provides for them — capture, the graphics device, thumbnails, pixels, the
//! video writer, JPEG, OCR, the clocks, the monitors and free space — sits behind the traits in
//! `platform`. The Windows backend is `platform::windows` and the macOS one
//! `platform::macos`; elsewhere the commands still refuse to record.

use std::fmt;

pub mod args;
pub mod finalize;
pub mod inspect;
pub mod platform;
pub mod record;
pub mod video;

pub mod frames;

#[cfg(any(windows, target_os = "macos"))]
pub mod capture;
#[cfg(target_os = "macos")]
pub mod clock_mac;
#[cfg(windows)]
pub mod clockwin;
#[cfg(windows)]
mod com;
#[cfg(any(windows, target_os = "macos"))]
pub mod disk;
#[cfg(any(windows, target_os = "macos"))]
pub mod probe;

/// The screen process's own version, written into `session` lines.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exit codes of `rukoo-voice-screen`, as the recorder's.
pub mod exit {
    /// Anything else, such as a disk error at start, or a pinned monitor that isn't there.
    pub const FAILED: i32 = 1;
    /// The arguments are wrong, or the command can't run on this platform.
    pub const USAGE: i32 = 2;
    /// The screen folder's state doesn't allow it: a journal with `stop`, or content without
    /// `--resume`.
    pub const CONFLICT: i32 = 3;
    /// Another screen process is recording into this meeting.
    pub const BUSY: i32 = 4;
    /// The computer doesn't let this process capture the screen: macOS' Screen Recording grant is
    /// off. It starts nothing and writes nothing, so Voice shows that reason and doesn't retry.
    pub const NO_PERMISSION: i32 = 5;
}

#[derive(Debug)]
pub struct ExitError {
    pub code: i32,
    pub message: String,
}

impl ExitError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: exit::USAGE,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            code: exit::CONFLICT,
            message: message.into(),
        }
    }

    pub fn busy() -> Self {
        Self {
            code: exit::BUSY,
            message: "another screen process is recording into this meeting".into(),
        }
    }

    pub fn no_permission(message: impl Into<String>) -> Self {
        Self {
            code: exit::NO_PERMISSION,
            message: message.into(),
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            code: exit::FAILED,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// The message `record`, `finalize` and `probe` give where the crate has no backend.
pub const NEEDS_WINDOWS: &str = "Screen recording needs Windows or macOS.";

/// Whether this is a development copy: `RUKOO_VOICE_DATA_DIR` is set, as for the app's own test
/// switches. Only then does `--test` work.
pub fn dev_mode() -> bool {
    std::env::var_os("RUKOO_VOICE_DATA_DIR").is_some_and(|v| !v.is_empty())
}

/// Runs one command line (without the program name):
///
/// ```text
/// rukoo-voice-screen record   --meeting <dir> --mode frames|video [--resume] [--monitor <id>] [--test <spec>]
/// rukoo-voice-screen finalize --meeting <dir> [--reason interrupted|failed]
/// rukoo-voice-screen probe
/// rukoo-voice-screen inspect <file.mp4>
/// ```
///
/// Tests call this from their own binary to play the screen process.
pub fn run(args: &[String]) -> Result<(), ExitError> {
    match args::parse(args, dev_mode())? {
        args::Cmd::Inspect(path) => {
            let report = inspect::inspect(&path);
            print_line(&report.to_string());
            match report.get("error").and_then(|e| e.as_str()) {
                Some(error) => Err(ExitError::failed(error.to_string())),
                None => Ok(()),
            }
        }
        #[cfg(windows)]
        args::Cmd::Record(opts) => record::record(platform::windows::WindowsPlatform::new(), opts),
        #[cfg(target_os = "macos")]
        args::Cmd::Record(opts) => record::record(platform::macos::MacPlatform::new(), opts),
        #[cfg(windows)]
        args::Cmd::Finalize { meeting, reason } => {
            finalize_with(platform::windows::WindowsPlatform::new(), &meeting, &reason)
        }
        #[cfg(target_os = "macos")]
        args::Cmd::Finalize { meeting, reason } => {
            finalize_with(platform::macos::MacPlatform::new(), &meeting, &reason)
        }
        #[cfg(any(windows, target_os = "macos"))]
        args::Cmd::Probe => {
            let caps = probe::probe();
            print_line(&serde_json::to_string(&caps).unwrap_or_default());
            Ok(())
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        args::Cmd::Record(_) | args::Cmd::Finalize { .. } | args::Cmd::Probe => {
            Err(ExitError::usage(NEEDS_WINDOWS))
        }
    }
}

/// `finalize` on the platform's backend, its report printed.
#[cfg(any(windows, target_os = "macos"))]
fn finalize_with<P: platform::Platform>(
    platform: P,
    meeting: &std::path::Path,
    reason: &str,
) -> Result<(), ExitError> {
    let done = finalize::finalize(&platform, meeting, reason)?;
    print_line(&done.to_json().to_string());
    Ok(())
}

/// Prints a result line. A closed stdout is the caller's business, not a reason to panic.
fn print_line(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{line}");
}

/// Writes a line to stderr, which the app sends to `<data>/logs/screen-<id>.log`. Callers never
/// pass frame text, window titles or monitor names: frames go by id and size, monitors by id.
#[doc(hidden)]
pub fn log(message: impl fmt::Display) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "rukoo-voice-screen: {message}");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    /// Every Media Foundation free function the `windows` crate binds is a load-time import of
    /// mfplat.dll or mfreadwrite.dll, which stops the exe on a Windows N edition. Only
    /// `video/mf.rs` may name them, and it resolves them at run time.
    #[test]
    fn no_direct_media_foundation_calls() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect(&root, &mut files);
        assert!(files.len() > 10, "found only {files:?}");
        let mut problems = Vec::new();
        for file in files {
            let rel = file
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "video/mf.rs" {
                continue;
            }
            let text = fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                if let Some(why) = direct_call(line) {
                    problems.push(format!("src/{rel}:{}: {why}: {}", n + 1, line.trim()));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "call Media Foundation through video::mf::Mf:\n{}",
            problems.join("\n")
        );
    }

    /// The line with the contents of its string literals blanked, so names in messages and in
    /// this test's own examples don't count.
    fn without_strings(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut in_string = false;
        let mut escaped = false;
        for c in line.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                    out.push(c);
                }
                continue;
            }
            if c == '"' {
                in_string = true;
            }
            out.push(c);
        }
        out
    }

    #[test]
    fn the_scan_finds_direct_calls() {
        for bad in [
            "let s = MFCreateSample()?;",
            "unsafe { MFStartup(MF_VERSION, 0) }",
            "windows::Win32::Media::MediaFoundation::MFCreateFile(a, b, c, d)",
            "use windows::Win32::Media::MediaFoundation::{IMFSample, MFCreateMediaType};",
            "use windows::Win32::Media::MediaFoundation::MFTEnumEx;",
            "MFTEnumEx (cat, flags, None, Some(&out), &mut pp, &mut n)",
        ] {
            assert!(direct_call(bad).is_some(), "{bad}");
        }
        for good in [
            "use windows::Win32::Media::MediaFoundation::*;",
            "out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;",
            "let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0);",
            "let sample = mf.create_sample()?;",
            "// MFCreateSample is called through Mf",
            "log(\"MFStartup(MF_VERSION) failed\");",
        ] {
            assert!(direct_call(good).is_none(), "{good}");
        }
    }

    fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// A call of an `MF…` function (a name with a lower-case letter, unlike the all-caps
    /// constructors such as `MFT_ENUM_FLAG(..)`), or a `use` that names one.
    fn direct_call(line: &str) -> Option<&'static str> {
        let code = without_strings(line.split("//").next().unwrap_or(""));
        let code = code.as_str();
        let bytes = code.as_bytes();
        let mut i = 0;
        while let Some(at) = code[i..].find("MF") {
            let start = i + at;
            let boundary = start == 0 || {
                let c = bytes[start - 1];
                !(c.is_ascii_alphanumeric() || c == b'_')
            };
            let mut end = start + 2;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let name = &code[start..end];
            if boundary && name.len() > 2 && name.bytes().any(|c| c.is_ascii_lowercase()) {
                let rest = code[end..].trim_start();
                if rest.starts_with('(') {
                    return Some("calls a Media Foundation function");
                }
                let is_function =
                    name[2..].starts_with(|c: char| c.is_ascii_uppercase()) && !name.contains('_');
                if code.trim_start().starts_with("use ")
                    && code.contains("MediaFoundation")
                    && is_function
                {
                    return Some("imports a Media Foundation function");
                }
            }
            i = end.max(start + 1);
        }
        None
    }
}
