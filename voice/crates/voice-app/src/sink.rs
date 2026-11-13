//! Where the engine's output goes: Tauri events for the pages, and the pill, tray and Settings windows.

use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Emitter};
use voice_engine::{Notification, PillRepair, PillSnapshot, TrayState, UiSink};

use crate::catalog::Catalog;
use crate::engine::EngineSlot;
use crate::pill::Pill;
use crate::settings_window;
use crate::tray::Tray;

pub struct AppSink {
    pub app: AppHandle,
    pub engine: EngineSlot,
    pub pill: Arc<Pill>,
    pub tray: Arc<Tray>,
    pub catalog: Arc<Catalog>,
}

impl AppSink {
    /// Re-reads the engine's UI options and applies them to every window and the tray.
    pub fn refresh_ui(&self) {
        let Some(engine) = self.engine.get() else {
            return;
        };
        let ui = engine.ui();
        self.tray.set_locale(ui.locale);
        self.pill.set_ui(ui.clone());
        settings_window::apply_ui(&self.app, &ui, &self.catalog);
    }
}

impl UiSink for AppSink {
    fn pill(&self, snapshot: PillSnapshot) {
        self.pill.snapshot(snapshot);
    }

    fn level(&self, level: f32) {
        self.pill.level(level);
    }

    fn settings(&self, state: Value) {
        let _ = self
            .app
            .emit_to(settings_window::LABEL, "settings://state", state);
    }

    fn tray(&self, tray: TrayState) {
        self.tray.update(tray);
    }

    fn notify(&self, notification: Notification) {
        self.tray.notify(&notification);
    }

    fn screen_permission_needed(&self) {
        #[cfg(target_os = "macos")]
        crate::screen_permission_mac::request();
    }

    fn ui_changed(&self) {
        self.refresh_ui();
    }

    fn screen_menu(&self, info: Value) {
        self.pill.screen_menu(info);
    }

    fn pill_repair(&self, seq: u64, repair: PillRepair) {
        self.pill.engine_repair(seq, repair);
    }
}
