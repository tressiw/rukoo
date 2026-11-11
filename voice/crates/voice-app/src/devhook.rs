//! `--dev-hooks <folder>`, development only: lets a script drive a running Voice without the tray or
//! the keyboard, which a test may not touch. The script writes `<folder>/request.json` (to another
//! name first, then renames it), Voice removes it, does what it asks and answers in
//! `<folder>/reply-<id>.json` the same way. `voice/scripts/live-meeting.mjs` uses it.
//!
//! Requests: `{"id": 1, "cmd": "status"}` (the pill snapshot, the pill window, whether Settings'
//! window is open and the whole Settings state), `start_meeting` and `stop_meeting` (what the tray's
//! items call), `pill_screen_click` (what a click on the pill's screen button does: opens its menu),
//! `pill_screen_set` with meeting, mode and optional monitor (the selected menu action), and
//! `{"cmd": "capture", "dir": "...", "page": "rukoo"}`, which opens Settings at a page without focus
//! and writes PNGs of the pill (`voice-pill.png`) and Settings (`voice-settings-<page>.png`) from
//! WebView2. Nothing here sends input or reads the screen.
//!
//! Settings and bucket sync, as the page would change them: `{"cmd": "settings", "key":
//! "storage.sync.bucket", "value": "test"}` calls `settings_set`, and `{"cmd": "sync", "action":
//! ...}` calls the sync commands: `secret` with `secret`, `test`, `key` (the recovery code, which
//! the caller keeps to itself), `import` with `code`, `restore_list`, `restore_start` and `status`
//! (Settings' `sync`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use voice_engine::StopReason;

use crate::commands::Shared;
use crate::settings_window;

const POLL: Duration = Duration::from_millis(100);
pub const REQUEST: &str = "request.json";

pub fn spawn(app: AppHandle, dir: PathBuf) {
    let _ = thread::Builder::new()
        .name("dev-hooks".into())
        .spawn(move || {
            if let Err(error) = fs::create_dir_all(&dir) {
                log::error!("dev hooks: {}: {error}", dir.display());
                return;
            }
            log::warn!("dev hooks: listening in {}", dir.display());
            loop {
                thread::sleep(POLL);
                let path = dir.join(REQUEST);
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let _ = fs::remove_file(&path);
                let request: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let id = request["id"].as_u64().unwrap_or(0);
                let reply = match handle(&app, &request) {
                    Ok(result) => json!({ "id": id, "ok": true, "result": result }),
                    Err(error) => json!({ "id": id, "ok": false, "error": error }),
                };
                if let Err(error) = write_atomic(&dir.join(format!("reply-{id}.json")), &reply) {
                    log::error!("dev hooks: {error}");
                }
            }
        });
}

fn write_atomic(path: &Path, value: &Value) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn handle(app: &AppHandle, request: &Value) -> Result<Value, String> {
    let shared = app.state::<Shared>();
    let engine = shared.engine.clone();
    match request["cmd"].as_str().unwrap_or_default() {
        "status" => {
            let tray = engine.tray_state();
            Ok(json!({
                "pid": std::process::id(),
                "pill": engine.pill_snapshot(),
                "pillWindow": shared.pill.status(),
                // Whether Settings is open, which a second launch has to ask for.
                "settingsWindow": app.get_webview_window(settings_window::LABEL).is_some(),
                "tray": { "meeting": tray.meeting, "meetingAtMs": tray.meeting_at_ms },
                "settings": engine.settings_state(),
            }))
        }
        "start_meeting" => engine
            .start_meeting()
            .map(|id| json!({ "meeting": id }))
            .map_err(|e| e.to_string()),
        "stop_meeting" => engine
            .stop_meeting(StopReason::User)
            .map(|()| json!({}))
            .map_err(|e| e.to_string()),
        "pill_screen_click" => {
            engine.pill_screen_clicked();
            Ok(json!({}))
        }
        "pill_screen_set" => engine
            .pill_screen_set(
                request["meeting"]
                    .as_str()
                    .ok_or("pill_screen_set needs a meeting")?,
                request["mode"]
                    .as_str()
                    .ok_or("pill_screen_set needs a mode")?,
                request["monitor"].as_str(),
            )
            .map(|()| json!({}))
            .map_err(|e| e.to_string()),
        "capture" => {
            let dir = request["dir"]
                .as_str()
                .map(PathBuf::from)
                .ok_or("capture needs a dir")?;
            let page = request["page"].as_str().map(str::to_string);
            capture(app, &shared, &dir, page)
        }
        "settings" => {
            let key = request["key"].as_str().ok_or("settings needs a key")?;
            engine
                .settings_set(key, request["value"].clone())
                .map(|()| json!({}))
                .map_err(|e| e.to_string())
        }
        "sync" => sync(&*engine, request),
        other => Err(format!("unknown dev hook {other:?}")),
    }
}

/// The bucket sync commands of Settings > Storage.
fn sync(engine: &dyn voice_engine::EngineApi, request: &Value) -> Result<Value, String> {
    let text = |name: &str| request[name].as_str().map(str::to_string);
    let result = match request["action"].as_str().unwrap_or_default() {
        "secret" => {
            let secret = text("secret").ok_or("secret needs a secret")?;
            engine.settings_sync_save_secret(secret).map(|()| json!({}))
        }
        "test" => engine.settings_sync_test(),
        "key" => engine.settings_sync_key("show", None),
        "import" => engine.settings_sync_key("import", text("code")),
        "restore_list" => engine.settings_sync_restore("list"),
        "restore_start" => engine.settings_sync_restore("start"),
        "status" => Ok(engine.settings_state()["sync"].clone()),
        other => return Err(format!("unknown sync action {other:?}")),
    };
    result.map_err(|e| e.to_string())
}

fn capture(
    app: &AppHandle,
    shared: &Shared,
    dir: &Path,
    page: Option<String>,
) -> Result<Value, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let page_name = page.clone();
    let loads = shared.settings_loads.load(Ordering::Relaxed);
    let fresh = app.get_webview_window(settings_window::LABEL).is_none();
    settings_window::open(
        app,
        &*shared.engine,
        &shared.catalog,
        shared.webview_dir.clone(),
        false,
        page,
    )
    .map_err(|e| e.to_string())?;
    if fresh {
        let start = Instant::now();
        while shared.settings_loads.load(Ordering::Relaxed) <= loads {
            if start.elapsed() > Duration::from_secs(30) {
                return Err("the Settings page didn't load within 30 s".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    // The page draws its state after the first settings://state.
    thread::sleep(Duration::from_millis(1500));
    let name = format!(
        "voice-settings-{}.png",
        page_name.as_deref().unwrap_or("main")
    );
    Ok(json!({
        "pill": png(shared.pill.window(), &dir.join("voice-pill.png")),
        "settings": png(
            app.get_webview_window(settings_window::LABEL),
            &dir.join(name),
        ),
    }))
}

#[cfg(windows)]
fn png(window: Option<tauri::WebviewWindow>, path: &Path) -> Value {
    let Some(window) = window else {
        return json!({ "file": path.display().to_string(), "error": "no window" });
    };
    match crate::capture::capture_png(&window, path) {
        Ok(()) => json!({ "file": path.display().to_string() }),
        Err(error) => json!({ "file": path.display().to_string(), "error": error }),
    }
}

#[cfg(not(windows))]
fn png(_window: Option<tauri::WebviewWindow>, path: &Path) -> Value {
    json!({ "file": path.display().to_string(), "error": "captures need WebView2" })
}
