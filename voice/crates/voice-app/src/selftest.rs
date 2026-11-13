//! `--self-test <folder>`, development only and always with `--fake-engine`: drives the real windows
//! through a meeting, records what the platform reports about them (Windows' window styles, macOS'
//! window recipe and front app), and captures the pill and Settings pages in both themes into
//! `<folder>`. It never sends input and never reads the screen; the captures come from the web view
//! rendering the app's own pages. On macOS it gives up the front once at the start, since `open` put
//! Voice there. The report is `<folder>/self-test.json`. The app then quits, with 0 when the report
//! says `ok` and 1 otherwise, unless `RUKOO_VOICE_SELF_TEST_KEEP_RUNNING` is set.

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
            // Done: the app quits, with 0 for a passing report and 1 otherwise. A test that reads
            // the windows from outside afterwards (Windows' `tests/launch.rs`) keeps it running.
            if std::env::var_os(KEEP_RUNNING).is_none() {
                let code = if report["ok"] == true { 0 } else { 1 };
                app.exit(code);
            }
        });
}

/// Set (to anything) to keep the app running after the self-test wrote its report.
pub const KEEP_RUNNING: &str = "RUKOO_VOICE_SELF_TEST_KEEP_RUNNING";

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

/// Waits for the pill page to acknowledge that it painted the snapshot. A window that shows nothing is
/// the fault ownkey-windows #13 was about, so on Windows the wait is final.
#[cfg(windows)]
fn await_paint(what: &str, limit: Duration, pill: &Pill, seq: u64) -> Result<(), String> {
    wait(what, limit, || painted(pill, seq))
}

/// A Mac waits exactly as Windows does when someone is at its console, so a pill that really doesn't
/// paint still fails the self-test. Otherwise (no GUI session, a session that isn't the console's, or a
/// locked screen) WebKit runs no frames: the page's `requestAnimationFrame` acknowledgement never comes
/// and waiting would only fail the run. The capture then shows what the page drew, and `session` in the
/// report says which of those it was.
#[cfg(target_os = "macos")]
fn await_paint(what: &str, limit: Duration, pill: &Pill, seq: u64) -> Result<(), String> {
    if crate::session_mac::at_the_console() {
        return wait(what, limit, || painted(pill, seq));
    }
    thread::sleep(Duration::from_millis(500));
    if !painted(pill, seq) {
        log::warn!(
            "self-test: {what}: no paint acknowledgement while nobody is at the console ({}); \
             capturing what the page drew",
            session()
        );
    }
    Ok(())
}

/// Other platforms capture nothing here, so a missing acknowledgement is only worth a line in the log.
#[cfg(not(any(windows, target_os = "macos")))]
fn await_paint(what: &str, _limit: Duration, pill: &Pill, seq: u64) -> Result<(), String> {
    thread::sleep(Duration::from_millis(500));
    if !painted(pill, seq) {
        log::warn!("self-test: {what}: no paint acknowledgement; capturing what the page drew");
    }
    Ok(())
}

/// What the report says about the session the self-test ran in, so an empty pill capture is explained.
#[cfg(target_os = "macos")]
fn session() -> Value {
    let (on_console, screen_locked) = crate::session_mac::session_flags();
    json!({
        "onConsole": on_console,
        "screenLocked": screen_locked,
        "atTheConsole": crate::session_mac::at_the_console_from(on_console, screen_locked),
    })
}

#[cfg(not(target_os = "macos"))]
fn session() -> Value {
    Value::Null
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

/// The front app's process. A pill that activates Voice shows up here as Voice's pid.
#[cfg(target_os = "macos")]
fn foreground() -> Value {
    match objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication() {
        Some(front) => json!({
            "pid": front.processIdentifier(),
            "bundle": front.bundleIdentifier().map(|id| id.to_string()),
        }),
        None => json!({ "pid": null }),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn foreground() -> Value {
    Value::Null
}

/// `open` makes a freshly started Voice the front app. Give the front back before the first look,
/// so the comparison afterwards is about the pill and Settings, not the launch.
#[cfg(target_os = "macos")]
fn yield_front(app: &AppHandle) -> Result<(), String> {
    on_main(app, || {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            let ns_app = objc2_app_kit::NSApplication::sharedApplication(mtm);
            if ns_app.isActive() {
                ns_app.deactivate();
            }
        }
    })?;
    thread::sleep(Duration::from_millis(500));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn yield_front(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

/// macOS: a button panel made the way the pill makes its own, clicked by handing mouse events
/// straight to it (nothing reaches the window server): a click, a press dragged off before the
/// release, and a click again. Two clicks must arrive, and neither the panel nor Voice may become key or active.
#[cfg(target_os = "macos")]
fn panel_clicks(app: &AppHandle) -> Result<Value, String> {
    on_main(app, || {
        use std::cell::Cell;
        use std::rc::Rc;

        use crate::pill::hit_mac::{HitEvent, HitPanel};
        let clicks = Rc::new(Cell::new(0u32));
        let counter = clicks.clone();
        let panel = HitPanel::create(
            "self-test",
            Box::new(move |event| {
                if event == HitEvent::Click {
                    counter.set(counter.get() + 1);
                }
            }),
        )?;
        // AppKit drops mouse events sent to a window that isn't ordered in, so the panel is ordered
        // in, as the real ones are, far off every display where nobody sees it.
        let far = crate::pill::place::Frame::new(-20_000.0, -20_000.0, 40.0, 40.0);
        panel.show_at(crate::pill::hit_mac::PanelFrame {
            hit: far,
            button: Some(far),
        });
        for inside in [true, false, true] {
            panel.click_for_test(inside)?;
        }
        let recipe = panel.recipe();
        let active = objc2::MainThreadMarker::new()
            .is_some_and(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive());
        Ok::<_, String>(json!({
            "clicks": clicks.get(),
            "recipe": recipe,
            "voiceActive": active,
            "ok": clicks.get() == 2 && !recipe.is_key && !recipe.can_become_key && !active,
        }))
    })?
}

#[cfg(not(target_os = "macos"))]
fn panel_clicks(_app: &AppHandle) -> Result<Value, String> {
    Ok(json!({ "ok": true }))
}

/// One click on the stop button. Windows calls what its stop window calls; macOS clicks the real stop
/// panel and waits for the engine's next snapshot, which its click handler asks for on a thread.
fn stop_click(
    app: &AppHandle,
    engine: &dyn voice_engine::EngineApi,
    pill: &std::sync::Arc<Pill>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let before = engine.pill_snapshot().seq;
        let p = pill.clone();
        on_main(app, move || p.click_stop_for_test())??;
        wait("the stop panel's click", Duration::from_secs(5), || {
            engine.pill_snapshot().seq > before
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, pill);
        engine.pill_stop_clicked();
        Ok(())
    }
}

fn pill_styles(app: &AppHandle, pill: &std::sync::Arc<Pill>) -> Result<Value, String> {
    let pill = pill.clone();
    on_main(app, move || {
        // Only Windows and macOS have the stop and screen windows, so elsewhere nothing changes this
        // value.
        #[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_mut))]
        let mut styles =
            json!({ "pill": pill.window().and_then(|w| crate::pill::window::styles(&w)) });
        #[cfg(any(windows, target_os = "macos"))]
        {
            styles["stopWindow"] = json!(pill.hit_styles());
        }
        // macOS: the recipe as AppKit reports it, `isKey` and `canHide` included.
        #[cfg(target_os = "macos")]
        {
            styles["native"] = pill.native_status();
        }
        styles
    })
}

/// The PNG a capture wrote, as the self-test reports it.
#[cfg(any(windows, target_os = "macos"))]
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

#[cfg(any(windows, target_os = "macos"))]
fn capture(window: Option<tauri::WebviewWindow>, path: &Path) -> Value {
    let Some(window) = window else {
        return json!({ "file": path.display().to_string(), "error": "no window" });
    };
    match crate::capture::capture_png(&window, path) {
        Ok(()) => png_info(path),
        Err(error) => json!({ "file": path.display().to_string(), "error": error }),
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn capture(_window: Option<tauri::WebviewWindow>, path: &Path) -> Value {
    json!({ "file": path.display().to_string(), "error": "captures need WebView2" })
}

/// The pill's native window styles. Windows sets and reports them, and macOS reports its recipe in the
/// same shape (`window_mac::styles_of`): never key or main, at the pill's level, out of the window
/// cycle.
#[cfg(any(windows, target_os = "macos"))]
fn styles_ok(styles: &Value) -> bool {
    let p = &styles["pill"];
    // The stop window is made with the pill; when it's there it never activates either.
    let stop = &styles["stopWindow"];
    let stop_ok = stop.is_null() || (stop["noactivate"] == true && stop["topmost"] == true);
    p["toolwindow"] == true
        && p["noactivate"] == true
        && p["topmost"] == true
        && p["appwindow"] == false
        && stop_ok
        && native_ok(&styles["native"])
}

/// macOS: neither the pill, the stop panel nor the body panel is ever the key window, none hides with
/// the app, and all sit at the pill's level.
#[cfg(target_os = "macos")]
fn native_ok(native: &Value) -> bool {
    let window_ok = |w: &Value| {
        w["isKey"] == false
            && w["canBecomeKey"] == false
            && w["canHide"] == false
            && w["level"] == json!(crate::pill::window::LEVEL)
    };
    window_ok(&native["pill"]) && window_ok(&native["stopPanel"]) && window_ok(&native["bodyPanel"])
}

#[cfg(windows)]
fn native_ok(_native: &Value) -> bool {
    true
}

#[cfg(not(any(windows, target_os = "macos")))]
fn styles_ok(_styles: &Value) -> bool {
    true
}

fn run(app: &AppHandle, out: &Path) -> Result<Value, String> {
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let shared = app.state::<Shared>();
    let (engine, pill) = (shared.engine.clone(), shared.pill.clone());
    let mut steps = Vec::new();

    wait("the pill page to load", Duration::from_secs(60), || {
        pill.status().page_loaded
    })?;
    yield_front(app)?;
    let before = foreground();

    // 1. A meeting: the pill shows with its stop button. `--fake-engine=meeting` starts with one
    // already recording; the self-test then takes that one.
    if engine.tray_state().meeting.is_none() {
        engine.start_meeting().map_err(|e| e.to_string())?;
    }
    let seq = engine.pill_snapshot().seq;
    await_paint(
        "the meeting pill to paint",
        Duration::from_secs(20),
        &pill,
        seq,
    )?;
    // The button windows over the pill's stop and screen buttons: Windows and macOS. They follow the
    // layout the page sends after it painted, so on a Mac whose screen runs no frames (locked, or a
    // session nobody shows) they wait for that paint, as the capture does.
    #[cfg(windows)]
    wait(
        "the stop window over the button",
        Duration::from_secs(5),
        || pill.status().hit_visible,
    )?;
    #[cfg(target_os = "macos")]
    if painted(&pill, seq) {
        wait(
            "the stop window over the button",
            Duration::from_secs(5),
            || pill.status().hit_visible,
        )?;
    }
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

    // 3. Both themes in both languages, captured from the web view.
    let mut captures = Vec::new();
    for (theme, language) in CAPTURES {
        for (key, value) in [("ui.theme", theme), ("ui.language", language)] {
            engine
                .settings_set(key, json!(value))
                .map_err(|e| e.to_string())?;
        }
        let seq = engine.pill_snapshot().seq;
        await_paint(
            "the restyled pill to paint",
            Duration::from_secs(10),
            &pill,
            seq,
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
    let panel = panel_clicks(app)?;
    stop_click(app, &*engine, &pill)?;
    let asked = engine
        .pill_snapshot()
        .meeting
        .is_some_and(|m| m.confirm_stop);
    stop_click(app, &*engine, &pill)?;
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
    await_paint(
        "the pill to show again",
        Duration::from_secs(10),
        &pill,
        seq,
    )?;
    thread::sleep(Duration::from_millis(400));
    let reshown = pill_styles(app, &pill)?;
    let end = foreground();
    steps.push("hidden and shown again");

    // Windows compares the foreground window, macOS the front app (`hwnd` is null there).
    let same = |a: &Value, b: &Value| a["hwnd"] == b["hwnd"] && a["pid"] == b["pid"];
    let unchanged =
        same(&before, &after_pill) && same(&before, &after_settings) && same(&before, &end);
    let ours = [&after_pill, &after_settings, &end]
        .iter()
        .any(|f| f["pid"] == json!(std::process::id()));
    let ok = unchanged
        && !ours
        && styles_ok(&shown)
        && styles_ok(&hidden)
        && styles_ok(&reshown)
        && panel["ok"] == true
        && asked
        && saving.as_deref() == Some("processing")
        && captures.iter().all(|c| c["png"] == true);

    Ok(json!({
        "ok": ok,
        "pid": std::process::id(),
        "steps": steps,
        "session": session(),
        "foreground": {
            "before": before, "afterPill": after_pill, "afterSettings": after_settings, "end": end,
            "unchanged": unchanged, "everOurs": ours
        },
        "styles": { "shown": shown, "hidden": hidden, "shownAgain": reshown, "settings": settings_styles },
        "pill": pill.status(),
        "settingsLoads": shared.settings_loads.load(Ordering::Relaxed),
        "twoStepStop": { "firstClickAsks": asked, "secondClickStops": saving },
        "panelClicks": panel,
        "captures": captures,
    }))
}
