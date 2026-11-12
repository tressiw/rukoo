//! `rukoo-voice`: Rukoo Voice's tray app. It shows the engine's state in the pill, its native stop
//! window, Settings and the tray, and passes the pages' commands to the engine (voice/ui/API.md). Voice's
//! behavior lives in voice-engine; this crate only wires windows to it.

// A release build has no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appearance;
mod args;
#[cfg(windows)]
mod capture;
mod catalog;
mod commands;
mod devhook;
mod engine;
mod fake;
mod logging;
mod pill;
mod selftest;
mod settings_window;
mod shell;
mod single_instance;
mod sink;
mod system;
mod tray;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use voice_engine::{ApiError, Edge, Notification, Theme, UiOptions};

use crate::args::Args;
use crate::catalog::{keys, Catalog};
use crate::commands::Shared;
use crate::engine::EngineSlot;
use crate::pill::Pill;
use crate::shell::SystemShell;
use crate::single_instance::{Guard, Instance};
use crate::sink::AppSink;
use crate::tray::{Tray, TrayAction};

/// Exit codes: 2 for a bad command line, 3 when the engine doesn't start.
const EXIT_USAGE: i32 = 2;
const EXIT_ENGINE: i32 = 3;

fn main() {
    std::process::exit(run());
}

/// The development flags only run in a development copy, so they can never meet the real profile.
fn check_dev_flags(args: &Args, dev_mode: bool) -> Result<(), String> {
    let dev_flag =
        args.fake_engine.is_some() || args.self_test.is_some() || args.dev_hooks.is_some();
    if dev_flag && !dev_mode {
        return Err(
            "--fake-engine, --self-test and --dev-hooks are for development copies. Set RUKOO_VOICE_DATA_DIR \
             to a scratch folder first."
                .to_string(),
        );
    }
    if args.self_test.is_some() && args.fake_engine.is_none() {
        return Err("--self-test runs with --fake-engine.".to_string());
    }
    if let Some(scenario) = args.fake_engine.as_deref() {
        if !fake::SCENARIOS.contains(&scenario) {
            return Err(format!(
                "Unknown --fake-engine scenario {scenario}; use one of {}.",
                fake::SCENARIOS.join(", ")
            ));
        }
    }
    Ok(())
}

fn run() -> i32 {
    let args = match args::parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            system::fail(&format!("{error}\n\n{}", args::USAGE));
            return EXIT_USAGE;
        }
    };
    let dev_mode = voice_core::paths::dev_mode();
    // Before anything touches the data folder: a development flag without RUKOO_VOICE_DATA_DIR would
    // otherwise write to the installed Voice's folder.
    if let Err(error) = check_dev_flags(&args, dev_mode) {
        system::fail(&error);
        return EXIT_USAGE;
    }
    let Some(data_dir) = voice_core::paths::data_dir() else {
        system::fail("Rukoo Voice can't find its data folder: LOCALAPPDATA isn't set.");
        return 1;
    };
    let log = logging::init(&voice_core::paths::logs_dir(&data_dir));
    log::info!(
        "Rukoo Voice {} starting{}; data folder {}, log {}",
        env!("CARGO_PKG_VERSION"),
        if dev_mode { " (development copy)" } else { "" },
        data_dir.display(),
        log.display()
    );
    let guard = match single_instance::acquire(&data_dir) {
        Instance::First(guard) => guard,
        Instance::Running => {
            log::info!("Voice already runs for this data folder; asking it to open Settings");
            if !single_instance::signal(&data_dir) {
                log::warn!("the running Voice didn't take the signal");
            }
            return 0;
        }
    };
    run_app(data_dir, dev_mode, args, guard)
}

fn run_app(data_dir: PathBuf, dev_mode: bool, args: Args, guard: Guard) -> i32 {
    let exit_code = Arc::new(AtomicI32::new(0));
    let setup_code = exit_code.clone();
    let built = tauri::Builder::default()
        .invoke_handler(commands::handler())
        .setup(move |app| {
            setup(app.handle(), data_dir, dev_mode, args, guard, &setup_code);
            Ok(())
        })
        .build(tauri::generate_context!());
    let app = match built {
        Ok(app) => app,
        Err(error) => {
            system::fail(&format!("Rukoo Voice couldn't open its windows: {error}"));
            return 1;
        }
    };
    let code = app.run_return(|app, event| match event {
        // A tray app outlives its windows; only Quit (an exit with a code) ends it.
        RunEvent::ExitRequested {
            api, code: None, ..
        } => api.prevent_exit(),
        RunEvent::WindowEvent {
            event: WindowEvent::ThemeChanged(_),
            ..
        } => {
            system_appearance_changed(app);
        }
        RunEvent::Exit => shut_down(app),
        _ => {}
    });
    match exit_code.load(Ordering::Relaxed) {
        0 => code,
        failed => failed,
    }
}

/// The system's theme changed: the engine learns the new look for the Settings that follow the
/// system, and the windows take what it says. macOS says so through
/// [`appearance::watch_system`], Windows through tao's `ThemeChanged`. It runs on a thread of its
/// own because the engine and the windows both take their own locks.
///
/// macOS posts its notification before the user defaults the engine reads always show the new
/// style, so a second look follows a moment later. It finds nothing to do when the first one
/// already saw the change.
fn system_appearance_changed(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        for pause in [Duration::ZERO, Duration::from_millis(500)] {
            thread::sleep(pause);
            let Some(shared) = app.try_state::<Shared>() else {
                return;
            };
            shared.engine.system_looks_changed();
            shared.sink.refresh_ui();
        }
    });
}

/// The webview's profile in the data folder, so a development copy never shares the installed
/// one's. WKWebView keeps its own store and ignores this, so macOS passes none and a fresh data
/// folder there has no empty `WebView2/` folder.
fn webview_profile(data_dir: &std::path::Path) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        None
    } else {
        Some(data_dir.join("WebView2"))
    }
}

fn setup(
    app: &AppHandle,
    data_dir: PathBuf,
    dev_mode: bool,
    args: Args,
    guard: Guard,
    exit_code: &AtomicI32,
) {
    let catalog = Arc::new(Catalog::embedded());
    let webview_dir = webview_profile(&data_dir);
    let slot = EngineSlot::default();
    // Until the engine answers: the system's theme and language.
    let ui = UiOptions {
        theme: if system::prefers_light_theme() {
            Theme::Light
        } else {
            Theme::Dark
        },
        theme_follows_system: true,
        locale: system::locale(),
        pill_edge: Edge::Bottom,
        hide_pill_from_capture: true,
        dev: dev_mode,
    };
    let pill = Pill::new(
        app.clone(),
        slot.clone(),
        catalog.clone(),
        webview_dir.clone(),
        ui.clone(),
    );
    let tray = Tray::new(app, catalog.clone(), ui.locale);
    let sink = Arc::new(AppSink {
        app: app.clone(),
        engine: slot.clone(),
        pill: pill.clone(),
        tray: tray.clone(),
        catalog: catalog.clone(),
    });

    // The engine first: when it can't start, nothing shows but the reason.
    let options = engine::options(data_dir, dev_mode, args.import_ownkey.clone());
    let engine = match engine::start(options, sink.clone(), args.fake_engine.as_deref()) {
        Ok(engine) => engine,
        Err(error) => {
            log::error!("engine: {error}");
            system::fail(&engine::start_failure(&error, dev_mode));
            exit_code.store(EXIT_ENGINE, Ordering::Relaxed);
            app.exit(EXIT_ENGINE);
            return;
        }
    };
    slot.set(&engine);
    if args.fake_engine.is_some() {
        log::warn!(
            "running the fake engine (development only): nothing records and no keys are read"
        );
    }

    let weak = Arc::downgrade(&pill);
    let weak_too = weak.clone();
    pill::window::set_callbacks(pill::window::Callbacks {
        display_changed: Box::new(move || {
            if let Some(pill) = weak.upgrade() {
                pill.display_changed();
            }
        }),
        foreground_changed: Box::new(move || {
            if let Some(pill) = weak_too.upgrade() {
                pill.foreground_changed();
            }
        }),
    });

    app.manage(Shared {
        engine: engine.clone(),
        pill: pill.clone(),
        tray: tray.clone(),
        sink: sink.clone(),
        shell: Arc::new(SystemShell),
        catalog,
        webview_dir,
        settings_loads: AtomicU64::new(0),
        shut_down: AtomicBool::new(false),
    });

    sink.refresh_ui();
    let handle = app.clone();
    appearance::watch_system(move || system_appearance_changed(&handle));
    tray.update(engine.tray_state());
    let handle = app.clone();
    if let Err(error) = tray.install(move |action| tray_action(&handle, action)) {
        log::error!("tray: {error}");
    }
    if let Err(error) = pill.create() {
        log::error!("pill: could not create the window: {error}");
    }
    // The engine may have shown the pill already, e.g. for a meeting it reattached to.
    pill.snapshot(engine.pill_snapshot());

    let handle = app.clone();
    guard.on_signal(move || open_settings(&handle, true, None));

    if let Some(page) = engine.settings_start_page() {
        let handle = app.clone();
        thread::spawn(move || open_settings(&handle, true, Some(page)));
    }
    if let Some(out) = args.self_test {
        selftest::spawn(app.clone(), out);
    }
    if let Some(dir) = args.dev_hooks {
        devhook::spawn(app.clone(), dir);
    }
}

/// Shows Settings at `page`, or where the engine says. Never call on the event loop thread.
fn open_settings(app: &AppHandle, focus: bool, page: Option<String>) {
    let Some(shared) = app.try_state::<Shared>() else {
        return;
    };
    let page = page.or_else(|| shared.engine.settings_start_page());
    let result = settings_window::open(
        app,
        &*shared.engine,
        &shared.catalog,
        shared.webview_dir.clone(),
        focus,
        page,
    );
    if let Err(error) = result {
        log::error!("settings: could not open: {error}");
    }
}

/// Tray clicks arrive on the event loop; everything they start runs on its own thread.
fn tray_action(app: &AppHandle, action: TrayAction) {
    let app = app.clone();
    thread::spawn(move || {
        let Some(shared) = app.try_state::<Shared>() else {
            return;
        };
        let engine = shared.engine.clone();
        let result: Result<(), ApiError> = match action {
            TrayAction::Settings => {
                open_settings(&app, true, None);
                Ok(())
            }
            TrayAction::Quit => {
                quit(&app);
                Ok(())
            }
            TrayAction::Start => engine
                .start_meeting()
                .map(|id| log::info!("tray: started meeting {id}")),
            TrayAction::Pause => engine.pause_meeting(),
            TrayAction::Resume => engine.resume_meeting(),
            TrayAction::Stop => engine.stop_meeting(voice_engine::StopReason::User),
        };
        if let Err(error) = result {
            log::warn!("tray: {action:?}: {error}");
            shared.tray.notify(&Notification {
                title_key: keys::TRAY_TOOLTIP.to_string(),
                body: error.message,
            });
        }
    });
}

/// Quit from the tray. A meeting that records keeps recording in its own process (architecture.md).
fn quit(app: &AppHandle) {
    log::info!("quitting");
    shut_down(app);
    app.exit(0);
}

fn shut_down(app: &AppHandle) {
    let Some(shared) = app.try_state::<Shared>() else {
        return;
    };
    if shared.shut_down.swap(true, Ordering::SeqCst) {
        return;
    }
    shared.engine.shutdown();
    log::info!("stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(fake: Option<&str>, self_test: bool) -> Args {
        Args {
            fake_engine: fake.map(str::to_string),
            self_test: self_test.then(|| PathBuf::from("shots")),
            import_ownkey: None,
            dev_hooks: None,
        }
    }

    #[test]
    fn development_flags_need_a_development_copy() {
        assert!(check_dev_flags(&args(None, false), false).is_ok());
        assert!(check_dev_flags(&args(Some("idle"), false), false).is_err());
        assert!(check_dev_flags(&args(Some("idle"), false), true).is_ok());
        assert!(check_dev_flags(&args(Some("meeting"), true), true).is_ok());
        assert!(check_dev_flags(&args(None, true), true).is_err());
        assert!(check_dev_flags(&args(Some("party"), false), true).is_err());
        // The hooks drive the real engine too, but only in a development copy.
        let hooks = Args {
            dev_hooks: Some(PathBuf::from("hooks")),
            ..Args::default()
        };
        assert!(check_dev_flags(&hooks, false).is_err());
        assert!(check_dev_flags(&hooks, true).is_ok());
    }

    #[test]
    fn macos_keeps_no_webview2_folder_in_the_data_folder() {
        let data = std::path::Path::new("/tmp/voice-data");
        let profile = webview_profile(data);
        if cfg!(target_os = "macos") {
            // WKWebView ignores a profile folder, so asking for one under the data folder only left
            // an empty `WebView2/` there.
            assert_eq!(profile, None);
        } else {
            assert_eq!(profile, Some(data.join("WebView2")));
        }
    }
}
