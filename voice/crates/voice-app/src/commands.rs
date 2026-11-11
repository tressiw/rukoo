//! Every Tauri command in voice/ui/API.md, as thin wrappers over [`EngineApi`]. The few that need the
//! app's own help (folders, links, the folder picker, closing Settings) do that in [`ops`], which the tests
//! run against the fake engine without a window.
//!
//! Engine calls can block (a model load, a provider request), so they run on Tauri's blocking pool and
//! never on the event loop.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tauri::{State, WebviewWindow};
use voice_engine::{ApiError, Engine, PillSnapshot};

use crate::catalog::Catalog;
use crate::pill::place::Layout;
use crate::pill::Pill;
use crate::shell::Shell;
use crate::sink::AppSink;
use crate::tray::Tray;

/// The app's state, managed by Tauri.
pub struct Shared {
    pub engine: Engine,
    pub pill: Arc<Pill>,
    pub tray: Arc<Tray>,
    pub sink: Arc<AppSink>,
    pub shell: Arc<dyn Shell>,
    pub catalog: Arc<Catalog>,
    /// The WebView2 profile, inside Voice's data folder, so a development copy never shares the
    /// installed one's.
    pub webview_dir: PathBuf,
    /// How often the Settings page asked for its state: it loaded and its IPC works.
    pub settings_loads: AtomicU64,
    pub shut_down: AtomicBool,
}

async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|e| ApiError::new("failed", format!("The command stopped: {e}")))?
}

pub mod ops {
    use std::path::Path;

    use voice_engine::{ApiError, EngineApi, Folder};

    use crate::catalog::{keys, Catalog};
    use crate::shell::Shell;

    /// The addresses Settings may open. The page names a link and never sends a URL.
    pub fn link_url(link: &str) -> Result<&'static str, ApiError> {
        match link {
            "orukeet" => Ok("https://huggingface.co/oruk/orukeet"),
            "nemotron" => Ok("https://huggingface.co/nvidia/Nemotron-3-Diarization"),
            _ => Err(ApiError::new(
                "invalid_value",
                format!("Unknown link {link}"),
            )),
        }
    }

    pub fn folder(name: &str) -> Result<Folder, ApiError> {
        match name {
            "logs" => Ok(Folder::Logs),
            "bridge" => Ok(Folder::Bridge),
            "recordings" => Ok(Folder::Recordings),
            "models" => Ok(Folder::Models),
            _ => Err(ApiError::new(
                "invalid_value",
                format!("Unknown folder {name}"),
            )),
        }
    }

    pub fn open_link(shell: &dyn Shell, link: &str) -> Result<(), ApiError> {
        shell
            .open_url(link_url(link)?)
            .map_err(|e| ApiError::new("failed", e))
    }

    pub fn open_folder(
        engine: &dyn EngineApi,
        shell: &dyn Shell,
        name: &str,
    ) -> Result<(), ApiError> {
        let path = engine.folder_path(folder(name)?).ok_or_else(|| {
            ApiError::new("not_found", format!("Voice has no {name} folder yet."))
        })?;
        if !Path::new(&path).is_dir() {
            return Err(ApiError::new(
                "not_found",
                format!("{} doesn't exist yet.", path.display()),
            ));
        }
        shell
            .open_path(&path)
            .map_err(|e| ApiError::new("failed", e))
    }

    /// Shows the folder picker and saves the choice; `None` when the user cancelled.
    pub fn choose_folder(
        engine: &dyn EngineApi,
        shell: &dyn Shell,
        catalog: &Catalog,
        purpose: &str,
        owner: Option<isize>,
    ) -> Result<Option<String>, ApiError> {
        let (title_key, folder) = match purpose {
            "models" => (keys::FOLDER_MODELS, Folder::Models),
            "recordings" => (keys::FOLDER_RECORDINGS, Folder::Recordings),
            _ => {
                return Err(ApiError::new(
                    "invalid_value",
                    format!("Unknown folder purpose {purpose}"),
                ))
            }
        };
        let title = catalog.text(engine.ui().locale, title_key);
        // The picker opens where the folder is now, so a change starts from what the user sees.
        let start = engine.folder_path(folder);
        let Some(path) = shell
            .pick_folder(&title, start.as_deref(), owner)
            .map_err(|e| ApiError::new("failed", e))?
        else {
            return Ok(None);
        };
        engine.settings_set_folder(purpose, path.clone())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    }
}

// ---------- the pill ----------

#[tauri::command]
pub async fn pill_snapshot(
    window: WebviewWindow,
    state: State<'_, Shared>,
) -> Result<PillSnapshot, ApiError> {
    state.pill.page_called(window.label());
    let engine = state.engine.clone();
    blocking(move || Ok(engine.pill_snapshot())).await
}

#[tauri::command]
pub async fn pill_ack(seq: u64, state: State<'_, Shared>) -> Result<(), ApiError> {
    state.pill.ack(seq);
    let engine = state.engine.clone();
    blocking(move || {
        engine.pill_ack(seq);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn pill_layout(layout: Layout, state: State<'_, Shared>) -> Result<(), ApiError> {
    state.pill.layout(layout);
    Ok(())
}

// ---------- Settings ----------

#[tauri::command]
pub async fn settings_state(state: State<'_, Shared>) -> Result<Value, ApiError> {
    state.settings_loads.fetch_add(1, Ordering::Relaxed);
    let engine = state.engine.clone();
    blocking(move || Ok(engine.settings_state())).await
}

#[tauri::command]
pub async fn settings_set(
    key: String,
    value: Value,
    state: State<'_, Shared>,
) -> Result<(), ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_set(&key, value)).await
}

#[tauri::command]
pub async fn settings_save_provider(save: Value, state: State<'_, Shared>) -> Result<(), ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_save_provider(save)).await
}

#[tauri::command]
pub async fn settings_remove_key(target: String, state: State<'_, Shared>) -> Result<(), ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_remove_key(&target)).await
}

#[tauri::command]
pub async fn settings_list_models(
    request: Value,
    state: State<'_, Shared>,
) -> Result<Vec<String>, ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_list_models(request)).await
}

#[tauri::command]
pub async fn settings_model(
    model: String,
    action: String,
    state: State<'_, Shared>,
) -> Result<(), ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_model(&model, &action)).await
}

#[tauri::command]
pub async fn settings_choose_folder(
    purpose: String,
    window: WebviewWindow,
    state: State<'_, Shared>,
) -> Result<Option<String>, ApiError> {
    #[cfg(windows)]
    let owner = window.hwnd().ok().map(|hwnd| hwnd.0 as isize);
    #[cfg(target_os = "macos")]
    let owner = window.ns_window().ok().map(|window| window as isize);
    #[cfg(not(any(windows, target_os = "macos")))]
    let owner = {
        let _ = &window;
        None
    };
    let (engine, shell, catalog) = (
        state.engine.clone(),
        state.shell.clone(),
        state.catalog.clone(),
    );
    blocking(move || ops::choose_folder(&*engine, &*shell, &catalog, &purpose, owner)).await
}

#[tauri::command]
pub async fn settings_open_folder(
    folder: String,
    state: State<'_, Shared>,
) -> Result<(), ApiError> {
    let (engine, shell) = (state.engine.clone(), state.shell.clone());
    blocking(move || ops::open_folder(&*engine, &*shell, &folder)).await
}

#[tauri::command]
pub async fn settings_open_link(link: String, state: State<'_, Shared>) -> Result<(), ApiError> {
    let shell = state.shell.clone();
    blocking(move || ops::open_link(&*shell, &link)).await
}

#[tauri::command]
pub async fn settings_clean_preview(
    preview: Value,
    state: State<'_, Shared>,
) -> Result<String, ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_clean_preview(preview)).await
}

#[tauri::command]
pub async fn settings_check_updates(state: State<'_, Shared>) -> Result<Value, ApiError> {
    let engine = state.engine.clone();
    blocking(move || Ok(engine.settings_check_updates())).await
}

#[tauri::command]
pub async fn settings_new_token(state: State<'_, Shared>) -> Result<(), ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_new_token()).await
}

#[tauri::command]
pub async fn settings_import_ownkey(state: State<'_, Shared>) -> Result<Value, ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_import_ownkey()).await
}

#[tauri::command]
pub async fn settings_sync_test(state: State<'_, Shared>) -> Result<Value, ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_sync_test()).await
}

#[tauri::command]
pub async fn settings_sync_save_secret(
    secret: String,
    state: State<'_, Shared>,
) -> Result<(), ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_sync_save_secret(secret)).await
}

#[tauri::command]
pub async fn settings_sync_key(
    action: String,
    code: Option<String>,
    state: State<'_, Shared>,
) -> Result<Value, ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_sync_key(&action, code)).await
}

#[tauri::command]
pub async fn settings_sync_key_copy(state: State<'_, Shared>) -> Result<(), ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_sync_key_copy()).await
}

#[tauri::command]
pub async fn settings_sync_restore(
    action: String,
    state: State<'_, Shared>,
) -> Result<Value, ApiError> {
    let engine = state.engine.clone();
    blocking(move || engine.settings_sync_restore(&action)).await
}

/// Hides the Settings window from screen capture and screen sharing while a key dialog is open,
/// as the pill hides itself. A new Settings window starts without it.
#[tauri::command]
pub async fn settings_protect(on: bool, window: WebviewWindow) -> Result<(), ApiError> {
    window.set_content_protected(on).map_err(|e| {
        ApiError::new(
            "failed",
            format!("Settings couldn't change its capture protection: {e}"),
        )
    })
}

#[tauri::command]
pub async fn settings_close(window: WebviewWindow) -> Result<(), ApiError> {
    window
        .close()
        .map_err(|e| ApiError::new("failed", format!("Could not close Settings: {e}")))
}

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        pill_snapshot,
        pill_ack,
        pill_layout,
        settings_state,
        settings_set,
        settings_save_provider,
        settings_remove_key,
        settings_list_models,
        settings_model,
        settings_choose_folder,
        settings_open_folder,
        settings_open_link,
        settings_clean_preview,
        settings_check_updates,
        settings_new_token,
        settings_import_ownkey,
        settings_sync_test,
        settings_sync_save_secret,
        settings_sync_key,
        settings_sync_key_copy,
        settings_sync_restore,
        settings_protect,
        settings_close,
    ]
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use serde_json::json;
    use voice_engine::{EngineApi, Locale};

    use super::*;
    use crate::fake::tests::engine;

    /// Where tests put the folders the fake engine names.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("voice-app-commands-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// What the fake shell heard: title, starting folder and owner window.
    type PickedWith = (String, Option<PathBuf>, Option<isize>);

    #[derive(Default)]
    struct FakeShell {
        opened: Mutex<Vec<String>>,
        pick: Mutex<Option<PathBuf>>,
        picked_with: Mutex<Option<PickedWith>>,
    }

    impl Shell for FakeShell {
        fn open_path(&self, path: &Path) -> Result<(), String> {
            self.opened.lock().unwrap().push(path.display().to_string());
            Ok(())
        }
        fn open_url(&self, url: &str) -> Result<(), String> {
            self.opened.lock().unwrap().push(url.to_string());
            Ok(())
        }
        fn pick_folder(
            &self,
            title: &str,
            start: Option<&Path>,
            owner: Option<isize>,
        ) -> Result<Option<PathBuf>, String> {
            *self.picked_with.lock().unwrap() =
                Some((title.to_string(), start.map(Path::to_path_buf), owner));
            Ok(self.pick.lock().unwrap().clone())
        }
    }

    #[test]
    fn links_come_from_the_apps_table_only() {
        let shell = FakeShell::default();
        ops::open_link(&shell, "orukeet").unwrap();
        ops::open_link(&shell, "nemotron").unwrap();
        assert_eq!(
            ops::open_link(&shell, "https://example.com")
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            *shell.opened.lock().unwrap(),
            vec![
                "https://huggingface.co/oruk/orukeet".to_string(),
                "https://huggingface.co/nvidia/Nemotron-3-Diarization".to_string()
            ]
        );
    }

    #[test]
    fn folders_open_only_when_they_exist() {
        let (engine, _) = engine("idle");
        let shell = FakeShell::default();
        assert_eq!(
            ops::open_folder(&*engine, &shell, "desktop")
                .unwrap_err()
                .code,
            "invalid_value"
        );
        let models = scratch("models");
        engine
            .settings_set_folder("models", models.clone())
            .unwrap();
        ops::open_folder(&*engine, &shell, "models").unwrap();
        assert_eq!(
            *shell.opened.lock().unwrap(),
            vec![models.display().to_string()]
        );
        engine
            .settings_set_folder("models", models.join("missing"))
            .unwrap();
        assert_eq!(
            ops::open_folder(&*engine, &shell, "models")
                .unwrap_err()
                .code,
            "not_found"
        );
        engine.shutdown();
    }

    #[test]
    fn choosing_a_folder_saves_it_and_cancelling_changes_nothing() {
        let (engine, recorder) = engine("idle");
        let shell = FakeShell::default();
        let catalog = Catalog::embedded();
        assert_eq!(
            ops::choose_folder(&*engine, &shell, &catalog, "models", Some(42)).unwrap(),
            None
        );
        assert_eq!(
            engine.settings_state()["config"]["local"]["modelFolder"],
            ""
        );

        let chosen = scratch("models");
        *shell.pick.lock().unwrap() = Some(chosen.clone());
        let result = ops::choose_folder(&*engine, &shell, &catalog, "models", Some(42)).unwrap();
        assert_eq!(result, Some(chosen.to_string_lossy().into_owned()));
        assert_eq!(
            engine.settings_state()["config"]["local"]["modelFolder"],
            json!(chosen.clone())
        );
        let (title, _first_start, owner) = shell.picked_with.lock().unwrap().clone().unwrap();
        assert_eq!(
            title,
            catalog.text(engine.ui().locale, crate::catalog::keys::FOLDER_MODELS)
        );
        assert_eq!(owner, Some(42));
        // The next time, the panel opens in the folder the setting holds now.
        *shell.pick.lock().unwrap() = None;
        assert_eq!(
            ops::choose_folder(&*engine, &shell, &catalog, "models", Some(42)).unwrap(),
            None
        );
        assert_eq!(
            shell.picked_with.lock().unwrap().as_ref().unwrap().1,
            Some(chosen.clone())
        );
        // The page hears about it through settings://state.
        let last = recorder.settings.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last["config"]["local"]["modelFolder"], json!(chosen));

        assert_eq!(
            ops::choose_folder(&*engine, &shell, &catalog, "desktop", None)
                .unwrap_err()
                .code,
            "invalid_value"
        );
        engine.shutdown();
    }

    #[test]
    fn settings_values_are_checked_and_each_change_sends_a_newer_state() {
        let (engine, recorder) = engine("idle");
        let before = engine.settings_state()["seq"].as_u64().unwrap();
        engine
            .settings_set("fillers.custom", json!("zeg maar"))
            .unwrap();
        let state = engine.settings_state();
        assert_eq!(state["config"]["fillers"]["custom"], "zeg maar");
        assert!(state["seq"].as_u64().unwrap() > before);
        assert_eq!(
            recorder.settings.lock().unwrap().last().unwrap()["seq"],
            state["seq"]
        );

        assert_eq!(
            engine
                .settings_set("dictation.hotkey", json!("f14"))
                .unwrap_err()
                .code,
            "same_key"
        );
        assert_eq!(
            engine
                .settings_set("dictation.startAtLogin", json!(true))
                .unwrap_err()
                .code,
            "dev_mode"
        );
        assert_eq!(
            engine
                .settings_set("ui.theme", json!("purple"))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            engine
                .settings_set("local.idleMinutes", json!(1441))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        assert_eq!(
            engine
                .settings_set("transcription.provider", json!("openai"))
                .unwrap_err()
                .code,
            "invalid_value"
        );
        engine.shutdown();
    }

    #[test]
    fn the_theme_and_language_reach_the_pill_settings_and_shell() {
        let (engine, recorder) = engine("idle");
        let changes = *recorder.ui_changes.lock().unwrap();
        engine.settings_set("ui.theme", json!("light")).unwrap();
        engine.settings_set("ui.language", json!("nl")).unwrap();
        let ui = engine.ui();
        assert_eq!(
            (ui.theme, ui.locale),
            (voice_engine::Theme::Light, Locale::Nl)
        );
        assert!(*recorder.ui_changes.lock().unwrap() >= changes + 2);
        let pill = recorder.last_pill();
        assert_eq!(
            (pill.theme, pill.locale),
            (voice_engine::Theme::Light, Locale::Nl)
        );
        let state = engine.settings_state();
        assert_eq!(
            (state["theme"].as_str(), state["locale"].as_str()),
            (Some("light"), Some("nl"))
        );
        engine.shutdown();
    }

    #[test]
    fn provider_forms_follow_the_key_and_consent_rules() {
        let (engine, _) = engine("idle");
        let save = |v: Value| engine.settings_save_provider(v);
        assert_eq!(
            save(json!({ "target": "meetings", "provider": "mistral", "endpoint": "", "model": "", "apiKey": null }))
                .unwrap_err()
                .code,
            "need_endpoint_model"
        );
        save(json!({ "target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions",
                     "model": "voxtral-mini-latest", "apiKey": "sk-test-4711", "consent": true }))
        .unwrap();
        let meetings = &engine.settings_state()["config"]["meetings"];
        assert_eq!(meetings["keySaved"], true);
        assert_eq!(
            meetings["consent"],
            json!({ "provider": "mistral", "host": "api.mistral.ai", "mode": "windows" })
        );
        // The key never shows in the state.
        assert!(!engine.settings_state().to_string().contains("sk-test-4711"));
        // null keeps the key for the same provider, but never carries it to another one.
        save(json!({ "target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions",
                     "model": "voxtral-small-latest", "apiKey": null, "consent": true }))
        .unwrap();
        assert_eq!(
            engine.settings_state()["config"]["meetings"]["keySaved"],
            true
        );
        save(json!({ "target": "meetings", "provider": "openai", "endpoint": "https://api.openai.com/v1/audio/transcriptions",
                     "model": "whisper-1", "apiKey": null, "consent": false }))
        .unwrap();
        let meetings = &engine.settings_state()["config"]["meetings"];
        assert_eq!(
            (meetings["keySaved"].clone(), meetings["consent"].clone()),
            (json!(false), Value::Null)
        );
        engine.settings_remove_key("rewriting").unwrap();
        assert_eq!(
            engine.settings_remove_key("dictation").unwrap_err().code,
            "invalid_value"
        );
        engine.shutdown();
    }

    #[test]
    fn models_lists_previews_and_the_rest_pass_through() {
        let (engine, _) = engine("idle");
        let models = engine
            .settings_list_models(json!({ "target": "rewriting", "provider": "mistral", "endpoint": "x", "apiKey": null }))
            .unwrap();
        let mut sorted = models.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(models, sorted);
        assert_eq!(
            engine
                .settings_list_models(json!({ "target": "rewriting", "provider": "custom", "endpoint": "http://localhost:1/x" }))
                .unwrap_err()
                .code,
            "network"
        );
        let cleaned = engine
            .settings_clean_preview(json!({ "text": "Um, so I think, uh, we should go.", "enabled": true, "languages": ["en"], "custom": "" }))
            .unwrap();
        assert!(
            !cleaned.to_lowercase().contains("um") && !cleaned.contains("uh"),
            "{cleaned}"
        );
        let untouched = engine
            .settings_clean_preview(
                json!({ "text": "Um, so.", "enabled": false, "languages": ["en"], "custom": "" }),
            )
            .unwrap();
        assert_eq!(untouched, "Um, so.");
        assert_eq!(
            engine.settings_check_updates(),
            json!({ "state": "upToDate", "version": null })
        );
        assert_eq!(
            engine.settings_import_ownkey().unwrap_err().code,
            "dev_mode"
        );
        assert_eq!(
            engine
                .settings_model("orukeet", "remove")
                .map_err(|e| e.code),
            Ok(())
        );
        assert_eq!(
            engine.settings_model("orukeet", "remove").unwrap_err().code,
            "not_removable"
        );
        assert_eq!(
            engine
                .settings_model("whisper", "download")
                .unwrap_err()
                .code,
            "not_found"
        );
        // Bucket sync: Test connection needs the secret first, then the switch can turn on.
        assert_eq!(engine.settings_sync_test().unwrap_err().code, "no_secret");
        engine
            .settings_sync_save_secret("bucket-secret-4711".into())
            .unwrap();
        assert_eq!(
            engine.settings_sync_test().unwrap(),
            json!({ "otherKeys": 0 })
        );
        engine
            .settings_set("storage.sync.enabled", json!(true))
            .unwrap();
        assert!(!engine
            .settings_state()
            .to_string()
            .contains("bucket-secret-4711"));
        engine.shutdown();
    }

    #[test]
    fn the_first_run_opens_settings_at_the_import_once() {
        let (engine, _) = engine("first-run");
        assert_eq!(engine.settings_start_page().as_deref(), Some("import"));
        assert_eq!(engine.settings_start_page(), None);
        engine.shutdown();
    }

    #[test]
    fn errors_reach_the_page_as_code_and_message() {
        let error = ApiError::new("same_key", "f14 is already in use");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({ "code": "same_key", "message": "f14 is already in use" })
        );
    }
}
