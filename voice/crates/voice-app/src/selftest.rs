//! `--self-test <folder>`, development only and always with `--fake-engine`: drives the real windows
//! through a meeting, records what Windows reports about them, and captures the pill and Settings pages in
//! both themes into `<folder>`. It never sends input and never reads the screen; the captures come from
//! WebView2 rendering the app's own pages. The report is `<folder>/self-test.json`. The app keeps running
//! afterwards, so whoever started it stops it by its process id.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::commands::Shared;
use crate::pill::Pill;
use crate::settings_window;

/// Theme and language of each capture; the files are `pill-<theme>-<language>.png` and
/// `settings-<theme>-<language>.png`.
pub const CAPTURES: [(&str, &str); 4] = [
    ("dark", "en"),
    ("light", "en"),
    ("dark", "nl"),
    ("light", "nl"),
];

pub fn spawn(app: AppHandle, out: PathBuf) {
    let _ = thread::Builder::new()
        .name("self-test".into())
        .spawn(move || {
            let report = match run(&app, &out) {
                Ok(report) => report,
                Err(error) => {
                    log::error!("self-test: {error}");
                    json!({ "ok": false, "error": error })
                }
            };
            let path = out.join("self-test.json");
            let tmp = out.join("self-test.json.tmp");
            let text = serde_json::to_string_pretty(&report).unwrap_or_default();
            if fs::write(&tmp, text)
                .and_then(|_| fs::rename(&tmp, &path))
                .is_err()
            {
                log::error!("self-test: could not write {}", path.display());
            }
            log::info!("self-test: report in {}", path.display());
        });
}

fn on_main<T: Send + 'static>(
    app: &AppHandle,
    task: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(task());
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| "the event loop didn't answer".to_string())
}

fn wait(what: &str, limit: Duration, mut done: impl FnMut() -> bool) -> Result<(), String> {
    let start = Instant::now();
    while !done() {
        if start.elapsed() > limit {
            return Err(format!(
                "timed out after {} s waiting for {what}",
                limit.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn painted(pill: &Pill, seq: u64) -> bool {
    let status = pill.status();
    status.shown && status.acked >= seq
}

#[cfg(windows)]
fn foreground() -> Value {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        json!({ "hwnd": hwnd.0 as isize, "pid": pid })
    }
}

#[cfg(not(windows))]
fn foreground() -> Value {
    Value::Null
}

fn pill_styles(app: &AppHandle, pill: &std::sync::Arc<Pill>) -> Result<Value, String> {
    let pill = pill.clone();
    on_main(app, move || {
        // Only Windows has the stop and screen windows, so elsewhere nothing changes this value.
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut styles =
            json!({ "pill": pill.window().and_then(|w| crate::pill::window::styles(&w)) });
        #[cfg(windows)]
        {
            styles["stopWindow"] = json!(pill.hit_styles());
        }
        styles
    })
}

/// The PNG a capture wrote, as the self-test reports it. Only Windows can capture a window here.
#[cfg(windows)]
fn png_info(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_default();
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() > 24;
    let (width, height) = if png {
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap_or_default()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap_or_default()),
        )
    } else {
        (0, 0)
    };
    json!({ "file": path.file_name().map(|n| n.to_string_lossy().into_owned()), "png": png, "bytes": bytes.len(), "width": width, "height": height })
}

#[cfg(windows)]
fn capture(window: Option<tauri::WebviewWindow>, path: &Path) -> Value {
    let Some(window) = window else {
        return json!({ "file": path.display().to_string(), "error": "no window" });
    };
    match crate::capture::capture_png(&window, path) {
        Ok(()) => png_info(path),
        Err(error) => json!({ "file": path.display().to_string(), "error": error }),
    }
}

#[cfg(not(windows))]
fn capture(_window: Option<tauri::WebviewWindow>, path: &Path) -> Value {
    json!({ "file": path.display().to_string(), "error": "captures need WebView2" })
}

fn run(app: &AppHandle, out: &Path) -> Result<Value, String> {
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let shared = app.state::<Shared>();
    let (engine, pill) = (shared.engine.clone(), shared.pill.clone());
    let mut steps = Vec::new();

    wait("the pill page to load", Duration::from_secs(60), || {
        pill.status().page_loaded
    })?;
    let before = foreground();

    // 1. A meeting: the pill shows with its stop button.
    engine.start_meeting().map_err(|e| e.to_string())?;
    let seq = engine.pill_snapshot().seq;
    wait("the meeting pill to paint", Duration::from_secs(20), || {
        painted(&pill, seq)
    })?;
    wait(
        "the stop window over the button",
        Duration::from_secs(5),
        || pill.status().hit_visible,
    )?;
    thread::sleep(Duration::from_millis(400));
    let after_pill = foreground();
    let shown = pill_styles(app, &pill)?;
    steps.push("meeting pill shown");

    // 2. Settings, opened without focus so the user's window stays in front.
    let catalog = shared.catalog.clone();
    let dir = shared.webview_dir.clone();
    settings_window::open(app, &*engine, &catalog, dir, false, None).map_err(|e| e.to_string())?;
    wait(
        "the Settings page to call settings_state",
        Duration::from_secs(30),
        || shared.settings_loads.load(Ordering::Relaxed) >= 1,
    )?;
    thread::sleep(Duration::from_millis(1500));
    let after_settings = foreground();
    let settings_styles = {
        let app2 = app.clone();
        on_main(app, move || {
            app2.get_webview_window(settings_window::LABEL)
                .and_then(|w| crate::pill::window::styles(&w))
        })?
    };
    steps.push("Settings loaded");

    // 3. Both themes in both languages, captured from WebView2.
    let mut captures = Vec::new();
    for (theme, language) in CAPTURES {
        for (key, value) in [("ui.theme", theme), ("ui.language", language)] {
            engine
                .settings_set(key, json!(value))
                .map_err(|e| e.to_string())?;
        }
        let seq = engine.pill_snapshot().seq;
        wait(
            "the restyled pill to paint",
            Duration::from_secs(10),
            || painted(&pill, seq),
        )?;
        thread::sleep(Duration::from_millis(1200));
        captures.push(capture(
            pill.window(),
            &out.join(format!("pill-{theme}-{language}.png")),
        ));
        captures.push(capture(
            app.get_webview_window(settings_window::LABEL),
            &out.join(format!("settings-{theme}-{language}.png")),
        ));
    }
    steps.push("captured");

    // 4. The two-step stop as the stop window reports it, then the pill hides and shows again.
    engine.pill_stop_clicked();
    let asked = engine
        .pill_snapshot()
        .meeting
        .is_some_and(|m| m.confirm_stop);
    engine.pill_stop_clicked();
    let saving = engine.pill_snapshot().meeting.map(|m| m.state);
    wait(
        "the pill to hide after saving",
        Duration::from_secs(10),
        || !pill.status().shown,
    )?;
    thread::sleep(Duration::from_millis(300));
    let hidden = pill_styles(app, &pill)?;
    engine.start_meeting().map_err(|e| e.to_string())?;
    let seq = engine.pill_snapshot().seq;
    wait("the pill to show again", Duration::from_secs(10), || {
        painted(&pill, seq)
    })?;
    thread::sleep(Duration::from_millis(400));
    let reshown = pill_styles(app, &pill)?;
    let end = foreground();
    steps.push("hidden and shown again");

    let same = |a: &Value, b: &Value| a["hwnd"] == b["hwnd"];
    let unchanged =
        same(&before, &after_pill) && same(&before, &after_settings) && same(&before, &end);
    let ours = [&after_pill, &after_settings, &end]
        .iter()
        .any(|f| f["pid"] == json!(std::process::id()));
    let styles_ok = |v: &Value| {
        let p = &v["pill"];
        p["toolwindow"] == true
            && p["noactivate"] == true
            && p["topmost"] == true
            && p["appwindow"] == false
    };
    let ok = unchanged
        && !ours
        && styles_ok(&shown)
        && styles_ok(&hidden)
        && styles_ok(&reshown)
        && asked
        && saving.as_deref() == Some("processing")
        && captures.iter().all(|c| c["png"] == true);

    Ok(json!({
        "ok": ok,
        "pid": std::process::id(),
        "steps": steps,
        "foreground": {
            "before": before, "afterPill": after_pill, "afterSettings": after_settings, "end": end,
            "unchanged": unchanged, "everOurs": ours
        },
        "styles": { "shown": shown, "hidden": hidden, "shownAgain": reshown, "settings": settings_styles },
        "pill": pill.status(),
        "settingsLoads": shared.settings_loads.load(Ordering::Relaxed),
        "twoStepStop": { "firstClickAsks": asked, "secondClickStops": saving },
        "captures": captures,
    }))
}
