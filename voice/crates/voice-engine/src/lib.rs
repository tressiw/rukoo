//! Rukoo Voice's behavior behind one API: dictation, the pill's state, Settings and meetings.
//!
//! The Tauri app (`voice-app`) drives it through [`api::EngineApi`] and renders what comes out of
//! [`api::UiSink`]. Nothing in this crate opens a window, so all of it can be tested without one.

pub mod api;
pub mod autostart;
mod dictation;
mod engine;
pub mod media;
pub mod meetings;
pub mod permissions;
mod pill;
mod platform;
pub mod services;
mod settings;
pub mod sync;

use std::fs;
use std::sync::{Arc, OnceLock, Weak};

use voice_bridge::EventLog;
use voice_core::config::{Config, ConfigError};
use voice_protocol::paths::{bridge_path, Platform};
use voice_protocol::Side;
use voice_screen_files::policy::reserve_ok;

pub use api::{
    Activity, ApiError, Edge, Engine, EngineApi, EngineOptions, Folder, Locale, MeetingPill,
    Notice, Notification, PillPhase, PillRepair, PillSnapshot, StopReason, Theme, TrayState,
    UiOptions, UiSink,
};
pub use services::Services;

use crate::dictation::Dictation;
use crate::engine::{Core, EngineImpl, Parts};
use crate::media::MediaService;
use crate::meetings::controller::{self, Controller, ControllerOptions};
use crate::meetings::screen::{ScreenOptions, ScreenSupervisor};
use crate::meetings::Meetings;
use crate::settings::SettingsRuntime;

/// Notification titles the engine uses besides dictation's (voice/ui/i18n `app.notify.*`).
pub const NOTIFY_SETTINGS: &str = "app.notify.settings";
pub const NOTIFY_WELCOME: &str = "app.notify.welcome";
pub use sync::NOTIFY_SYNC;

/// Every notification title the engine sends; voice-app checks that both catalogs have them.
pub const NOTIFY_TITLES: &[&str] = &[
    dictation::errors::TITLE_DICTATION,
    dictation::errors::TITLE_REWRITE,
    dictation::errors::TITLE_NO_TEXT_BOX,
    meetings::controller::NOTIFY_MEETING,
    NOTIFY_SETTINGS,
    NOTIFY_WELCOME,
    NOTIFY_SYNC,
];

/// Starts Voice: loads Settings (running the first-run Ownkey import when it applies), starts the
/// hotkeys, reattaches to a recorder and a screen process that are still recording, finishes
/// meetings that were cut off, and opens the bridge for Rukoo. Output goes to `sink` from here on.
pub fn start(options: EngineOptions, sink: Arc<dyn UiSink>) -> Result<Engine, ApiError> {
    let services = Services::system(&options);
    start_with(options, sink, services)
}

/// [`start`] on other services: tests run the engine on fakes this way.
pub fn start_with(
    options: EngineOptions,
    sink: Arc<dyn UiSink>,
    services: Services,
) -> Result<Engine, ApiError> {
    let data_dir = options.data_dir.clone();
    let dev_mode = options.dev_mode;
    fs::create_dir_all(&data_dir).map_err(|e| {
        ApiError::new(
            "failed",
            format!(
                "Voice couldn't make its data folder {}: {e}",
                data_dir.display()
            ),
        )
    })?;
    let first_run = Config::is_first_run(&data_dir);
    let mut notices: Vec<(&str, String)> = Vec::new();
    let mut cfg = match Config::load_as(&data_dir, dev_mode) {
        Ok(cfg) => cfg,
        Err(damaged @ ConfigError::Damaged { .. }) => {
            notices.push((NOTIFY_SETTINGS, damaged.to_string()));
            Config::defaults(dev_mode)
        }
        Err(e) => return Err(ApiError::new("config", e.to_string())),
    };
    let mut save = first_run;
    if first_run {
        // Voice transcribes on this computer unless the user picks a provider (voice/ui/API.md).
        cfg.switch_audio_provider("orukeet");
    }

    // The import: automatic on an installed copy's first start, and on request with
    // --import-ownkey <copy>, the only way a development copy imports.
    let mut start_page = None;
    let source = match &options.import_ownkey_from {
        Some(path) => Some(path.clone()),
        None if first_run && !dev_mode => services.ownkey_config.clone().filter(|p| p.is_file()),
        None => None,
    };
    if let Some(path) = source {
        match settings::ownkey::read(&path) {
            Ok((result, raw)) => {
                let report = settings::ownkey::apply(&result, &raw, &mut cfg, dev_mode);
                settings::ownkey::save_report(&data_dir, &report);
                start_page = Some("import".to_string());
                save = true;
            }
            Err(e) if options.import_ownkey_from.is_some() => {
                notices.push((
                    NOTIFY_SETTINGS,
                    format!("Ownkey's settings weren't imported: {}", e.message),
                ));
            }
            Err(_) => {}
        }
    }
    save |= cfg.resolve_same_meeting_provider();
    if save {
        cfg.save(&data_dir)
            .map_err(|e| ApiError::new("config", format!("Voice couldn't save Settings: {e}")))?;
    }
    // Decision 6: an installed copy starts at login from its first start; never re-enabled later.
    if first_run && !dev_mode {
        let _ = services.autostart.set(true);
    }

    let sources = match options.test_sources.as_deref().filter(|_| dev_mode) {
        Some(text) => controller::parse_sources(text).unwrap_or_else(|e| {
            notices.push((
                NOTIFY_SETTINGS,
                format!("RUKOO_VOICE_TEST_SOURCES was ignored: {e}"),
            ));
            controller::default_sources()
        }),
        None => controller::default_sources(),
    };
    let log = EventLog::open(&data_dir).ok();
    let discovery = services.discovery_path.clone().or_else(|| {
        let data = data_dir.display().to_string();
        let env = move |name: &str| {
            if name == voice_core::paths::DATA_DIR_ENV {
                Some(data.clone())
            } else {
                std::env::var(name).ok()
            }
        };
        bridge_path(Side::Voice, Platform::current(), &env).map(Into::into)
    });

    // The controller reports to the engine, which doesn't exist yet: it gets the engine once
    // it does. Nothing records before `recover` below, so no report is lost.
    let engine: Arc<OnceLock<Weak<Core>>> = Arc::new(OnceLock::new());
    let controller = Controller::new(
        ControllerOptions {
            data_dir: data_dir.clone(),
            recorder_exe: services.recorder_exe.clone(),
            sources,
            timing: services.controller.clone(),
        },
        {
            let engine = Arc::clone(&engine);
            move |change| {
                if let Some(core) = engine.get().and_then(Weak::upgrade) {
                    core.on_meeting(change);
                }
            }
        },
    )
    .map_err(|e| {
        ApiError::new(
            "failed",
            format!("Voice couldn't open its meetings folder: {e}"),
        )
    })?;
    // The screen supervisor reports to the engine the same way.
    let screen = ScreenSupervisor::new(
        ScreenOptions {
            data_dir: data_dir.clone(),
            exe: services.screen_exe.clone(),
            // Development copies only, like RUKOO_VOICE_TEST_SOURCES.
            test: options.test_screen.clone().filter(|_| dev_mode),
            test_monitor: options.test_monitor.clone().filter(|_| dev_mode),
            timing: services.screen_timing.clone(),
            free_bytes: Some({
                let system = Arc::clone(&services.system);
                Arc::new(move |path: &std::path::Path| system.free_bytes(path))
            }),
        },
        log.clone(),
        {
            let engine = Arc::clone(&engine);
            move |change| {
                if let Some(core) = engine.get().and_then(Weak::upgrade) {
                    core.on_screen(change);
                }
            }
        },
    )
    .map_err(|e| {
        ApiError::new(
            "failed",
            format!("Voice couldn't open its meetings folder: {e}"),
        )
    })?;
    // Before the bridge opens, so a meeting Rukoo starts at once keeps them too.
    controller.library().set_retention(cfg.meetings_retention);
    controller.set_screen(engine::screen_mode(&cfg), screen.available());

    // Bucket sync. When its store doesn't open (locked, damaged, or from a newer Voice), Voice
    // runs without it, and retention keeps every meeting's media while the switch is on.
    let (sync, sync_error) = match voice_sync::BucketSync::open(sync_options(
        &data_dir,
        &controller,
        &services,
        &engine,
    )) {
        Ok(sync) => (Some(Arc::new(sync)), None),
        Err(e) => {
            log::error!("sync: Voice runs without bucket sync: {}", e.message);
            (None, Some(e))
        }
    };
    if sync::migrate_secret(
        &mut cfg,
        sync.as_deref(),
        &data_dir,
        Arc::clone(&services.protector),
    ) {
        if let Err(e) = cfg.save(&data_dir) {
            log::error!("sync: config.json wasn't saved after moving the bucket secret: {e}");
        }
    }
    let media = MediaService::new(
        &data_dir,
        Arc::clone(&controller),
        sync.clone(),
        Arc::clone(&services.screen_media),
        {
            let system = Arc::clone(&services.system);
            Arc::new(move |path: &std::path::Path| system.free_bytes(path))
        },
    );
    let meetings = Meetings::new(
        controller,
        screen,
        log,
        discovery,
        &options.version,
        Arc::clone(&media),
    );
    let core = Arc::new(Core::new(
        options,
        services,
        sink,
        Parts {
            config: cfg,
            meetings,
            settings: SettingsRuntime::new(start_page),
            sync,
            media,
        },
    ));
    let _ = engine.set(Arc::downgrade(&core));
    core.set_this();
    if let Some(e) = &sync_error {
        core.sync_rt.store_failed(e);
    }

    let cfg = core.config();
    sync::configure_sync(&core, &cfg);
    core.svc
        .speech
        .local_configure(core.registry(&cfg), cfg.idle_timeout());
    if let Err(e) = Dictation::start(&core) {
        notices.push((
            dictation::errors::TITLE_DICTATION,
            format!("Keyboard setup is incomplete. {e}"),
        ));
    }
    // Reattaches to a recorder that still records and finalizes meetings that were cut off;
    // both start a pipeline. The screen supervisor then finishes the screen journals of the
    // meetings that don't record any more. Then meetings whose transcription didn't finish get
    // a pipeline too.
    core.meetings.controller.recover();
    let live = core.meetings.controller.current().map(|r| r.id);
    core.meetings.screen.recover(live.as_deref());
    core.meetings.screen.probe_async();
    for id in core.meetings.unfinished() {
        core.follow(&id);
    }
    core.sweep();
    core.meetings.publish_status();
    if let Some(head) = core.meetings.log.as_ref().map(|log| log.head()) {
        let weak = Arc::downgrade(&core);
        let _ = std::thread::Builder::new()
            .name("engine-watch".into())
            .spawn(move || engine::watch(weak, head));
    }
    let weak = Arc::downgrade(&core);
    let _ = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || settings::schedule_updates(weak));
    if first_run {
        let status = settings::state(&core)["dictation"]["status"].clone();
        if status == "modelMissing" || status == "noKey" {
            notices.push((
                NOTIFY_WELCOME,
                "Welcome to Rukoo Voice. Open Settings > Transcription to choose how your voice becomes text.".into(),
            ));
        }
    }
    for (title, body) in notices {
        core.notify(title, body);
    }
    core.sink.tray(core.tray());
    core.note_rukoo();
    core.settings_changed();
    Ok(Arc::new(EngineImpl(core)))
}

/// How voice-sync runs in this engine: in the data folder, with the engine's protector and
/// screen files, the free-space rule of every writer but the recorder, and status changes going
/// to Settings once the engine exists.
fn sync_options(
    data_dir: &std::path::Path,
    controller: &Arc<Controller>,
    services: &Services,
    engine: &Arc<OnceLock<Weak<Core>>>,
) -> voice_sync::SyncOptions {
    let reserve = {
        let controller = Arc::clone(controller);
        let system = Arc::clone(&services.system);
        let root = controller.library().root().to_path_buf();
        move |bytes: u64| {
            system
                .free_bytes(&root)
                .is_none_or(|free| reserve_ok(free, bytes, controller.current().is_some()))
        }
    };
    let on_change = {
        let engine = Arc::clone(engine);
        move || {
            if let Some(core) = engine.get().and_then(Weak::upgrade) {
                core.sync_changed();
            }
        }
    };
    voice_sync::SyncOptions {
        data_dir: data_dir.to_path_buf(),
        meetings_dir: controller.library().root().to_path_buf(),
        protector: Arc::clone(&services.protector),
        screen: Arc::clone(&services.screen_files),
        now: Arc::new(meetings::now_ms),
        reserve: Arc::new(reserve),
        timing: services.sync_timing.clone(),
        on_change: Arc::new(on_change),
    }
}
