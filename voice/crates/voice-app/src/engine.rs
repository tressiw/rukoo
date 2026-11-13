//! Starting the engine (or, in development, the fake one) and handing it to the parts that need it.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

use voice_engine::{ApiError, Engine, EngineApi, EngineOptions, UiSink};

/// The engine, for parts built before it starts: the sink, the pill's stop window, the tray.
#[derive(Clone, Default)]
pub struct EngineSlot(Arc<OnceLock<Weak<dyn EngineApi>>>);

impl EngineSlot {
    pub fn set(&self, engine: &Engine) {
        let _ = self.0.set(Arc::downgrade(engine));
    }

    pub fn get(&self) -> Option<Engine> {
        self.0.get()?.upgrade()
    }
}

pub fn options(
    data_dir: PathBuf,
    dev_mode: bool,
    import_ownkey_from: Option<PathBuf>,
) -> EngineOptions {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    // Test switches reach only a development copy.
    let dev_var = |name: &str| {
        dev_mode
            .then(|| std::env::var(name).ok())
            .flatten()
            .filter(|value| !value.is_empty())
    };
    EngineOptions {
        data_dir,
        dev_mode,
        exe_dir,
        test_sources: dev_var("RUKOO_VOICE_TEST_SOURCES"),
        test_screen: dev_var("RUKOO_VOICE_TEST_SCREEN"),
        test_monitor: dev_var("RUKOO_VOICE_TEST_MONITOR"),
        import_ownkey_from,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// What to tell the user when the engine doesn't start.
pub fn start_failure(error: &ApiError, dev_mode: bool) -> String {
    if error.code == "not_ready" {
        let mut text = format!(
            "This build of Rukoo Voice can't start: its engine isn't ready.\nDetails: {}",
            error.message
        );
        if dev_mode {
            text.push_str(
                "\n\nTo try the pill, its stop button, the tray and Settings, start it with --fake-engine \
                 (scenarios: idle, meeting, dictation, cycle, first-run).",
            );
        } else {
            text.push_str(
                "\n\nTo try the windows in a development copy, set RUKOO_VOICE_DATA_DIR to a scratch folder \
                 and start it with --fake-engine.",
            );
        }
        text
    } else {
        format!("Rukoo Voice couldn't start: {}", error.message)
    }
}

pub fn start(
    options: EngineOptions,
    sink: Arc<dyn UiSink>,
    fake: Option<&str>,
) -> Result<Engine, ApiError> {
    match fake {
        Some(scenario) => {
            let engine: Engine = crate::fake::FakeEngine::start(options, sink, scenario)?;
            Ok(engine)
        }
        None => {
            #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
            let mut services = voice_engine::Services::system(&options);
            // A press at the lock screen or from a switched-out session records nothing.
            #[cfg(target_os = "macos")]
            {
                services.at_console = Arc::new(crate::session_mac::at_the_console);
            }
            voice_engine::start_with(options, sink, services)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_engine_explains_itself_and_offers_the_fake() {
        let error = ApiError::new("not_ready", "The engine isn't built yet.");
        let dev = start_failure(&error, true);
        assert!(dev.contains("The engine isn't built yet."), "{dev}");
        assert!(dev.contains("--fake-engine"), "{dev}");
        let installed = start_failure(&error, false);
        assert!(
            installed.contains("RUKOO_VOICE_DATA_DIR") && installed.contains("--fake-engine"),
            "{installed}"
        );
        let other = start_failure(&ApiError::new("config", "config.json is damaged"), true);
        assert_eq!(other, "Rukoo Voice couldn't start: config.json is damaged");
    }

    #[test]
    fn test_sources_only_reach_a_development_engine() {
        let opts = options(PathBuf::from("x"), false, None);
        assert_eq!(opts.test_sources, None);
        assert_eq!(opts.test_screen, None);
        assert_eq!(opts.test_monitor, None);
        assert!(!opts.dev_mode);
        assert_eq!(opts.version, env!("CARGO_PKG_VERSION"));
    }
}
