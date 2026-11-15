//! Starts the built `rukoo-voice` and checks its windows from outside the process. Ignored by default:
//! they open the app's windows on this desktop for a few seconds. Run them on an interactive desktop with
//!
//! ```text
//! cargo test -p voice-app --test launch -- --ignored --test-threads=1
//! ```
//!
//! The app runs with the fake engine in a scratch data folder (`VOICE_APP_LAUNCH_DIR`, default under
//! the target folder) and its own discovery file, so it never meets a real profile. These tests send no
//! input and read no pixels from the screen: the captures come from the app's own WebView2 pages
//! (`--self-test`). Each app they start is stopped through its process handle.

#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowDisplayAffinity, GetWindowLongPtrW,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};

const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;

fn root() -> PathBuf {
    std::env::var_os("VOICE_APP_LAUNCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("voice-app-launch"))
}

/// Stops the app it holds through its process handle, also when a check fails.
struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(data: &Path, args: &[&std::ffi::OsStr]) -> App {
    fs::create_dir_all(data).unwrap();
    let stderr = fs::File::create(data.join("stderr.txt")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_rukoo-voice"))
        .args(args)
        .env("RUKOO_VOICE_DATA_DIR", data)
        .env("RUKOO_VOICE_BRIDGE", data.join("bridge.json"))
        // The self-test's windows are read from outside after its report, so it stays up.
        .env("RUKOO_VOICE_SELF_TEST_KEEP_RUNNING", "1")
        .env_remove("RUKOO_VOICE_TEST_SOURCES")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .expect("start rukoo-voice");
    App(child)
}

#[derive(Debug, Clone)]
struct Window {
    hwnd: isize,
    title: String,
    class: String,
    ex: u32,
    visible: bool,
}

fn windows_of(pid: u32) -> Vec<Window> {
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL {
        let found = unsafe { &mut *(data.0 as *mut (u32, Vec<Window>)) };
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == found.0 {
            let mut title = [0u16; 256];
            let mut class = [0u16; 256];
            let title_len = unsafe { GetWindowTextW(hwnd, &mut title) }.max(0) as usize;
            let class_len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
            found.1.push(Window {
                hwnd: hwnd.0 as isize,
                title: String::from_utf16_lossy(&title[..title_len]),
                class: String::from_utf16_lossy(&class[..class_len]),
                ex: unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32,
                visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
            });
        }
        BOOL(1)
    }
    let mut found = (pid, Vec::new());
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found.1
}

fn foreground() -> isize {
    unsafe { GetForegroundWindow() }.0 as isize
}

fn is_png(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() > 1000)
}

#[test]
#[ignore = "opens the app's windows on this desktop; run with --ignored"]
fn the_windows_show_without_taking_focus_and_settings_loads() {
    let root = root();
    let data = root.join("fake-data");
    let shots = std::env::var_os("VOICE_APP_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("shots"));
    let _ = fs::remove_dir_all(&data);
    fs::create_dir_all(&shots).unwrap();
    let report_path = shots.join("self-test.json");
    let _ = fs::remove_file(&report_path);
    for old in fs::read_dir(&shots).unwrap().flatten() {
        if old.path().extension().is_some_and(|e| e == "png") {
            let _ = fs::remove_file(old.path());
        }
    }

    let before = foreground();
    let mut app = start(
        &data,
        &[
            "--fake-engine".as_ref(),
            "--self-test".as_ref(),
            shots.as_os_str(),
        ],
    );
    let pid = app.0.id();

    // The self-test writes its report when it is done; a cold WebView2 profile takes a while.
    let deadline = Instant::now() + Duration::from_secs(120);
    while !report_path.exists() {
        if let Some(status) = app.0.try_wait().unwrap() {
            let stderr = fs::read_to_string(data.join("stderr.txt")).unwrap_or_default();
            panic!("rukoo-voice exited with {status} before the report:\n{stderr}");
        }
        assert!(Instant::now() < deadline, "no self-test report after 120 s");
        thread::sleep(Duration::from_millis(200));
    }
    let report: Value = serde_json::from_str(&fs::read_to_string(&report_path).unwrap()).unwrap();
    let windows = windows_of(pid);
    let after = foreground();

    // From outside the process: the pill is a visible, non-activating topmost tool window that lets
    // clicks through and isn't in the taskbar, and it is kept out of screen capture.
    let pill = windows
        .iter()
        .find(|w| w.title == "Rukoo Voice" && w.ex & WS_EX_NOACTIVATE.0 != 0)
        .unwrap_or_else(|| panic!("no pill window among {windows:#?}"));
    assert!(pill.visible, "{pill:?}");
    for (flag, name) in [
        (WS_EX_TOOLWINDOW.0, "TOOLWINDOW"),
        (WS_EX_NOACTIVATE.0, "NOACTIVATE"),
        (WS_EX_TOPMOST.0, "TOPMOST"),
        (WS_EX_TRANSPARENT.0, "TRANSPARENT"),
        (WS_EX_LAYERED.0, "LAYERED"),
    ] {
        assert!(pill.ex & flag != 0, "pill lacks {name}: {:#010x}", pill.ex);
    }
    assert_eq!(
        pill.ex & WS_EX_APPWINDOW.0,
        0,
        "pill has APPWINDOW: {:#010x}",
        pill.ex
    );
    let mut affinity = 0u32;
    unsafe {
        GetWindowDisplayAffinity(HWND(pill.hwnd as *mut _), &mut affinity).unwrap();
    }
    assert_eq!(
        affinity, WDA_EXCLUDEFROMCAPTURE,
        "the pill shows in screen capture"
    );

    let stop = windows
        .iter()
        .find(|w| w.class == "RukooVoiceStopButton")
        .unwrap_or_else(|| panic!("no stop window among {windows:#?}"));
    assert!(
        stop.visible,
        "the stop window isn't over the button: {stop:?}"
    );
    // pill.stopButton in the language the self-test left Voice in.
    assert!(
        stop.title == "Stop recording" || stop.title == "Opname stoppen",
        "{stop:?}"
    );
    assert_eq!(
        stop.ex & (WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0),
        WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0
    );

    let settings = windows
        .iter()
        .find(|w| w.title == "Rukoo Voice Settings" || w.title == "Rukoo Voice-instellingen")
        .unwrap_or_else(|| panic!("no Settings window among {windows:#?}"));
    assert!(settings.visible);

    // Nothing the app did moved the foreground window.
    assert_eq!(
        after, before,
        "the foreground window changed while the app ran"
    );
    assert_eq!(report["foreground"]["unchanged"], true, "{report:#}");
    assert_eq!(report["foreground"]["everOurs"], false, "{report:#}");

    // Inside the process: the page invoked settings_state, the pill painted, styles survived a hide and
    // a show, and the two-step stop asked first.
    assert!(
        report["settingsLoads"].as_u64().unwrap_or(0) >= 1,
        "{report:#}"
    );
    assert_eq!(report["twoStepStop"]["firstClickAsks"], true, "{report:#}");
    assert_eq!(
        report["twoStepStop"]["secondClickStops"], "processing",
        "{report:#}"
    );
    for when in ["shown", "hidden", "shownAgain"] {
        let pill = &report["styles"][when]["pill"];
        assert_eq!(pill["toolwindow"], true, "{when}: {report:#}");
        assert_eq!(pill["appwindow"], false, "{when}: {report:#}");
    }
    for page in ["pill", "settings"] {
        for theme in ["dark", "light"] {
            for language in ["en", "nl"] {
                let name = format!("{page}-{theme}-{language}.png");
                assert!(is_png(&shots.join(&name)), "{name} is missing or not a PNG");
            }
        }
    }
    assert_eq!(report["ok"], true, "{report:#}");

    drop(app);
    assert!(
        windows_of(pid).is_empty(),
        "windows left after the app stopped"
    );
}

#[test]
#[ignore = "starts the app; run with --ignored"]
fn without_an_engine_it_says_so_and_offers_the_fake() {
    let data = root().join("no-engine-data");
    let _ = fs::remove_dir_all(&data);
    let mut app = start(&data, &[]);
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = app.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "rukoo-voice didn't exit without an engine"
        );
        thread::sleep(Duration::from_millis(100));
    };
    let stderr = fs::read_to_string(data.join("stderr.txt")).unwrap_or_default();
    // Until wave 3 lands the engine, voice_engine::start answers not_ready.
    assert_eq!(status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("--fake-engine"), "{stderr}");
    // It stopped before building any window or the WebView2 profile.
    assert!(!data.join("WebView2").exists());
}

#[test]
#[ignore = "starts the app; run with --ignored"]
fn an_unknown_fake_scenario_is_refused() {
    let data = root().join("flags-data");
    let _ = fs::remove_dir_all(&data);
    let mut app = start(&data, &["--fake-engine=party".as_ref()]);
    let status = app.0.wait().unwrap();
    assert_eq!(status.code(), Some(2));
    let stderr = fs::read_to_string(data.join("stderr.txt")).unwrap_or_default();
    assert!(
        stderr.contains("Unknown --fake-engine scenario party"),
        "{stderr}"
    );
}
