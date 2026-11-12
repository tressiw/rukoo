//! The pill window: shows, places, raises and hides it from the engine's snapshots, keeps the native stop
//! window over the stop button and a second one over the screen button, and repairs a window that stopped
//! painting (pill-settings-design.md 3.2-3.3, 3.7).
//!
//! The engine owns the state machine and its timers; this module only turns snapshots into window
//! changes. Window calls run on the event loop thread. Never dispatch to it while holding `inner`: on that
//! thread `run_on_main_thread` runs the task at once.

pub mod place;
pub mod window;

#[cfg(windows)]
pub mod hit;
#[cfg(target_os = "macos")]
pub mod hit_mac;
#[cfg(any(target_os = "macos", test))]
pub mod pointer;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, ContextMenu, Menu, MenuItem, PredefinedMenuItem};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use voice_engine::{PillPhase, PillRepair, PillSnapshot, UiOptions};

use crate::catalog::{keys, Catalog};
use crate::engine::EngineSlot;
use place::Layout;

pub const LABEL: &str = "pill";

/// The webview plays its 140 ms exit; the window hides 40 ms after that, unless a newer visible
/// snapshot came (Ownkey's margin).
const HIDE_AFTER: Duration = Duration::from_millis(180);
/// Without an ack this long after a show, nudge the window's size so WebView2 paints again (#13) ...
const NUDGE_AFTER: Duration = Duration::from_millis(500);
/// ... and this long after, build a new window and replay the snapshot.
const REBUILD_AFTER: Duration = Duration::from_millis(2000);
/// A cold WebView2 can take a second or more to load the page the first time; the watchdog only counts
/// from the page's first `pill_snapshot`, up to this long.
const PAGE_LOAD_LIMIT: Duration = Duration::from_secs(10);
/// Display changes come in bursts; repair once they settle.
const SETTLE: Duration = Duration::from_millis(300);
/// How often the paint watchdog looks.
const WATCH_POLL: Duration = Duration::from_millis(100);
/// At most one raise per this long from the foreground hook.
const RAISE_INTERVAL: Duration = Duration::from_millis(250);

/// What the self-test and logs read about the pill.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PillStatus {
    pub label: String,
    pub shown: bool,
    pub seq: u64,
    pub acked: u64,
    pub page_loaded: bool,
    pub rebuilds: u32,
    pub hit_visible: bool,
    pub screen_hit_visible: bool,
}

struct Inner {
    label: String,
    rebuilds: u32,
    snapshot: Option<PillSnapshot>,
    /// The window is shown, or a show is on its way to the event loop.
    shown: bool,
    page_loaded: bool,
    acked: u64,
    hide_gen: u64,
    watch_gen: u64,
    repair_gen: u64,
    /// The newest snapshot that draws a stop or screen button; older layouts are stale.
    stop_seq: u64,
    layout: Option<Layout>,
    ui: UiOptions,
    last_raise: Option<Instant>,
    raise_queued: bool,
    hit_visible: bool,
    screen_hit_visible: bool,
    monitor: Option<window::Monitor>,
    /// The newest snapshot repaired, and how far: the app's watchdog and the engine's
    /// `pill_repair` both watch the paint, and one of them is enough.
    repaired: (u64, Repair),
}

/// How far a repair went for one snapshot; a rebuild includes a nudge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Repair {
    #[default]
    None,
    Nudge,
    Rebuild,
}

/// The paint watchdog's timing: a nudge `NUDGE_AFTER` and a rebuild `REBUILD_AFTER` after the page
/// could paint and didn't. Time while it couldn't (a covered pill or a locked screen on macOS) doesn't
/// count, and once it can again it gets the whole grace period afresh.
struct PaintClock {
    since: Instant,
    nudged: bool,
}

impl PaintClock {
    fn new(now: Instant) -> PaintClock {
        PaintClock {
            since: now,
            nudged: false,
        }
    }

    /// The repair due at `now`, given whether the page can paint.
    fn step(&mut self, now: Instant, expected: bool) -> Option<Repair> {
        if !expected {
            self.since = now;
            self.nudged = false;
            return None;
        }
        let waited = now.saturating_duration_since(self.since);
        if waited >= REBUILD_AFTER {
            Some(Repair::Rebuild)
        } else if waited >= NUDGE_AFTER && !self.nudged {
            self.nudged = true;
            Some(Repair::Nudge)
        } else {
            None
        }
    }
}

/// Whether a repair of snapshot `seq` still has work: it shows, isn't painted, and nobody
/// repaired it (or a newer one) this far yet.
fn repair_due(shown: bool, acked: u64, seq: u64, repaired: (u64, Repair), want: Repair) -> bool {
    shown && acked < seq && (repaired.0 < seq || (repaired.0 == seq && repaired.1 < want))
}

pub struct Pill {
    app: AppHandle,
    engine: EngineSlot,
    catalog: Arc<Catalog>,
    webview_dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    screen_actions: Mutex<HashMap<String, ScreenChoice>>,
    screen_menu: Mutex<Option<Menu<tauri::Wry>>>,
}

#[derive(Clone)]
struct ScreenChoice {
    meeting: String,
    mode: String,
    monitor: Option<String>,
}

/// The native window over a pill button: a layered window on Windows, a non-activating panel on macOS.
#[cfg(windows)]
use hit::{HitEvent as ButtonEvent, HitHandler as ButtonHandler, HitWindow as ButtonWindow};
#[cfg(target_os = "macos")]
use hit_mac::{HitEvent as ButtonEvent, HitHandler as ButtonHandler, HitPanel as ButtonWindow};

#[cfg(any(windows, target_os = "macos"))]
thread_local! {
    // The stop window lives on the event loop thread, like the pill.
    static HIT: std::cell::RefCell<Option<ButtonWindow>> = const { std::cell::RefCell::new(None) };
    // So does the screen button's window.
    static SCREEN_HIT: std::cell::RefCell<Option<ButtonWindow>> = const { std::cell::RefCell::new(None) };
}

/// Where a pill button is on screen. Windows scales CSS pixels to the monitor's DPI from the client
/// origin; on macOS a CSS pixel is a point and only the y axis turns around.
#[cfg(windows)]
type ButtonRect = place::Rect;
#[cfg(target_os = "macos")]
type ButtonRect = place::Frame;

#[cfg(windows)]
fn button_rect(win: &WebviewWindow, css: &place::CssRect) -> Option<ButtonRect> {
    window::origin_and_scale(win)
        .and_then(|(origin, scale)| place::css_to_physical(origin, scale, css))
}

#[cfg(target_os = "macos")]
fn button_rect(win: &WebviewWindow, css: &place::CssRect) -> Option<ButtonRect> {
    window::button_frame(win, css)
}

#[cfg(windows)]
fn exclude_from_capture(button: &ButtonWindow, excluded: bool) {
    window::set_capture_excluded(button.hwnd(), excluded);
}

#[cfg(target_os = "macos")]
fn exclude_from_capture(button: &ButtonWindow, excluded: bool) {
    button.set_capture_excluded(excluded);
}

/// Which monitor a show uses.
#[derive(Clone, Copy)]
enum Place {
    /// Where the user works: a hidden pill that shows, or a new key press.
    Choose,
    /// Where the window is, or the nearest monitor: after a display change.
    Current,
    /// Where it was last placed: a rebuilt window.
    Last,
}

enum Action {
    Show,
    Raise { replace: bool },
    Hit,
    HideLater(u64),
}

fn has_stop(s: &PillSnapshot) -> bool {
    s.visible && s.meeting.as_ref().is_some_and(|m| m.stop_enabled)
}

/// The pill draws its screen button while screen controls are available.
fn has_screen(s: &PillSnapshot) -> bool {
    s.visible
        && s.phase == PillPhase::Meeting
        && s.meeting.as_ref().is_some_and(|m| m.screen_switch)
}

impl Pill {
    pub fn new(
        app: AppHandle,
        engine: EngineSlot,
        catalog: Arc<Catalog>,
        webview_dir: Option<PathBuf>,
        ui: UiOptions,
    ) -> Arc<Pill> {
        let pill = Arc::new(Pill {
            app: app.clone(),
            engine,
            catalog,
            webview_dir,
            inner: Mutex::new(Inner {
                label: LABEL.to_string(),
                rebuilds: 0,
                snapshot: None,
                shown: false,
                page_loaded: false,
                acked: 0,
                hide_gen: 0,
                watch_gen: 0,
                repair_gen: 0,
                stop_seq: 0,
                layout: None,
                ui,
                last_raise: None,
                raise_queued: false,
                hit_visible: false,
                screen_hit_visible: false,
                monitor: None,
                repaired: (0, Repair::None),
            }),
            screen_actions: Mutex::new(HashMap::new()),
            screen_menu: Mutex::new(None),
        });
        let weak = Arc::downgrade(&pill);
        app.on_menu_event(move |_, event| {
            let Some(pill) = weak.upgrade() else { return };
            let choice = pill
                .screen_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(event.id().as_ref())
                .cloned();
            if let (Some(choice), Some(engine)) = (choice, pill.engine.get()) {
                thread::spawn(move || {
                    if let Err(error) = engine.pill_screen_set(
                        &choice.meeting,
                        &choice.mode,
                        choice.monitor.as_deref(),
                    ) {
                        log::warn!("screen: pill selection failed: {error}");
                    }
                });
            }
        });
        pill
    }

    pub fn screen_menu(self: &Arc<Self>, info: Value) {
        let pill = Arc::clone(self);
        let _ = self.app.run_on_main_thread(move || {
            if let Err(error) = pill.show_screen_menu(info) {
                log::warn!("screen: could not show the pill menu: {error}");
            }
        });
    }

    fn show_screen_menu(&self, info: Value) -> tauri::Result<()> {
        let Some(window) = self.window() else {
            return Ok(());
        };
        let Some(meeting) = info["meeting"].as_str() else {
            return Ok(());
        };
        if !self.lock().snapshot.as_ref().is_some_and(has_screen) {
            return Ok(());
        }
        let locale = self.lock().ui.locale;
        let current = info["screen"].as_str().unwrap_or("off");
        let menu = Menu::new(&self.app)?;
        let mut actions = HashMap::new();
        for (mode, key) in [
            ("off", keys::SCREEN_OFF),
            ("frames", keys::SCREEN_FRAMES),
            ("video", keys::SCREEN_VIDEO),
        ] {
            let id = format!("pill-screen-{meeting}-{mode}");
            menu.append(&CheckMenuItem::with_id(
                &self.app,
                &id,
                self.catalog.text(locale, key),
                true,
                mode == current,
                None::<&str>,
            )?)?;
            actions.insert(
                id,
                ScreenChoice {
                    meeting: meeting.into(),
                    mode: mode.into(),
                    monitor: None,
                },
            );
        }
        if let Some(monitors) = info["monitors"].as_array().filter(|m| !m.is_empty()) {
            menu.append(&PredefinedMenuItem::separator(&self.app)?)?;
            menu.append(&MenuItem::new(
                &self.app,
                self.catalog.text(locale, keys::SCREEN_MONITOR),
                false,
                None::<&str>,
            )?)?;
            for (i, monitor) in monitors.iter().enumerate() {
                let Some(value) = monitor["id"].as_str() else {
                    continue;
                };
                let id = format!("pill-monitor-{meeting}-{i}");
                let name = monitor["name"].as_str().unwrap_or(value).replace('&', "&&");
                menu.append(&CheckMenuItem::with_id(
                    &self.app,
                    &id,
                    name,
                    true,
                    info["monitor"] == value,
                    None::<&str>,
                )?)?;
                actions.insert(
                    id,
                    ScreenChoice {
                        meeting: meeting.into(),
                        mode: current.into(),
                        monitor: Some(value.into()),
                    },
                );
            }
        }
        *self
            .screen_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = actions;
        *self.screen_menu.lock().unwrap_or_else(|e| e.into_inner()) = Some(menu.clone());
        menu.popup(window.as_ref().window())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn label(&self) -> String {
        self.lock().label.clone()
    }

    pub fn status(&self) -> PillStatus {
        let i = self.lock();
        PillStatus {
            label: i.label.clone(),
            shown: i.shown,
            seq: i.snapshot.as_ref().map_or(0, |s| s.seq),
            acked: i.acked,
            page_loaded: i.page_loaded,
            rebuilds: i.rebuilds,
            hit_visible: i.hit_visible,
            screen_hit_visible: i.screen_hit_visible,
        }
    }

    pub fn window(&self) -> Option<WebviewWindow> {
        let label = self.label();
        self.app.get_webview_window(&label)
    }

    /// Builds the hidden pill window and its stop window. Call from setup or from a thread other than the
    /// event loop's: building a WebView2 window from inside an event handler deadlocks.
    pub fn create(self: &Arc<Self>) -> tauri::Result<()> {
        let label = self.label();
        let mut builder =
            WebviewWindowBuilder::new(&self.app, &label, WebviewUrl::App("pill.html".into()))
                .title("Rukoo Voice")
                .inner_size(place::WINDOW_W, place::WINDOW_H)
                .min_inner_size(place::WINDOW_W, place::WINDOW_H)
                .max_inner_size(place::WINDOW_W, place::WINDOW_H)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .visible_on_all_workspaces(true)
                .skip_taskbar(true)
                .focusable(false)
                .focused(false)
                .visible(false)
                .disable_drag_drop_handler();
        if let Some(dir) = &self.webview_dir {
            builder = builder.data_directory(dir.clone());
        }
        let window = builder.build()?;
        let this = self.clone();
        self.app.run_on_main_thread(move || {
            window::prepare(&window);
            this.ensure_hit();
            // A snapshot that came before the window did (or before a rebuild) shows now.
            let seq = {
                let i = this.lock();
                i.shown.then(|| i.snapshot.as_ref().map_or(0, |s| s.seq))
            };
            if let Some(seq) = seq {
                this.show_now(seq, Place::Last);
            }
        })?;
        Ok(())
    }

    /// The stop window's styles; event loop thread only.
    #[cfg(windows)]
    pub fn hit_styles(&self) -> Option<window::Styles> {
        HIT.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|hit| window::styles_of(hit.hwnd()))
        })
    }

    /// The stop panel's recipe in the shape of Windows' styles; main thread only.
    #[cfg(target_os = "macos")]
    pub fn hit_styles(&self) -> Option<window::Styles> {
        HIT.with(|cell| cell.borrow().as_ref().map(ButtonWindow::styles))
    }

    /// The native recipe of the pill and its two panels, for the `status` dev hook: level, Spaces
    /// behaviour, sharing type and the rest of `window::Recipe`. Main thread only.
    #[cfg(target_os = "macos")]
    pub fn native_status(&self) -> Value {
        let panel = |cell: &std::cell::RefCell<Option<ButtonWindow>>| {
            json!(cell.borrow().as_ref().map(ButtonWindow::recipe))
        };
        json!({
            "pill": self.window().and_then(|w| window::recipe(&w)),
            "stopPanel": HIT.with(panel),
            "screenPanel": SCREEN_HIT.with(panel),
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn native_status(&self) -> Value {
        Value::Null
    }

    // ---------- from the engine ----------

    pub fn snapshot(self: &Arc<Self>, s: PillSnapshot) {
        let (label, action) = {
            let mut i = self.lock();
            // Snapshots from different engine threads can arrive out of order; the newest wins, as on
            // the page.
            if i.snapshot.as_ref().is_some_and(|p| p.seq >= s.seq) {
                return;
            }
            let prev_phase = i.snapshot.as_ref().map(|p| p.phase);
            if has_stop(&s) || has_screen(&s) {
                i.stop_seq = i.stop_seq.max(s.seq);
            }
            let action = if s.visible {
                // A visible snapshot cancels a pending hide, like Ownkey's show().
                i.hide_gen += 1;
                if !i.shown {
                    i.shown = true;
                    Action::Show
                } else if prev_phase != Some(s.phase) {
                    // A new key press may come from another monitor; a meeting pill otherwise stays put.
                    Action::Raise {
                        replace: s.phase == PillPhase::Arming,
                    }
                } else {
                    Action::Hit
                }
            } else {
                i.hide_gen += 1;
                Action::HideLater(i.hide_gen)
            };
            i.snapshot = Some(s.clone());
            (i.label.clone(), action)
        };
        let _ = self.app.emit_to(label.as_str(), "pill://snapshot", &s);
        let seq = s.seq;
        match action {
            Action::Show => self.on_main(move |p| p.show_now(seq, Place::Choose)),
            Action::Raise { replace } => self.on_main(move |p| {
                if replace {
                    p.replace_if_moved();
                }
                p.raise_now();
                p.arm_watchdog(seq);
            }),
            Action::Hit => self.on_main(|p| p.update_hit()),
            Action::HideLater(generation) => {
                // Hide the stop window at once; the pill itself waits for its exit animation.
                self.on_main(|p| p.update_hit());
                let this = self.clone();
                thread::spawn(move || {
                    thread::sleep(HIDE_AFTER);
                    this.on_main(move |p| p.hide_now(generation));
                });
            }
        }
    }

    /// The engine's paint watchdog (`UiSink::pill_repair`): the page didn't ack `seq`. A nudge
    /// reapplies size, position, styles and topmost; a rebuild makes a new window, off the event
    /// loop thread, where building a WebView2 window would deadlock.
    pub fn engine_repair(self: &Arc<Self>, seq: u64, repair: PillRepair) {
        let want = match repair {
            PillRepair::Nudge => Repair::Nudge,
            PillRepair::Rebuild => Repair::Rebuild,
        };
        // A pill nobody can see doesn't paint on macOS; the app's own watchdog repairs it once it can
        // be seen again and still doesn't.
        if !self.paint_expected() {
            log::info!("pill: the engine saw no paint of snapshot {seq} while the pill can't be seen; no {repair:?}");
            return;
        }
        if !self.claim_repair(seq, want) {
            return;
        }
        log::warn!("pill: the engine saw no paint of snapshot {seq}; {repair:?}");
        match repair {
            PillRepair::Nudge => self.on_main(|p| p.nudge_now()),
            PillRepair::Rebuild => {
                let this = self.clone();
                thread::spawn(move || this.rebuild(seq));
            }
        }
    }

    /// Takes on a repair unless it is done or not needed.
    fn claim_repair(&self, seq: u64, want: Repair) -> bool {
        let mut i = self.lock();
        if !repair_due(i.shown, i.acked, seq, i.repaired, want) {
            return false;
        }
        i.repaired = (seq, want);
        true
    }

    pub fn level(&self, level: f32) {
        let label = self.label();
        let _ = self
            .app
            .emit_to(label.as_str(), "pill://level", json!({ "level": level }));
    }

    /// Theme, language or capture exclusion changed.
    pub fn set_ui(self: &Arc<Self>, ui: UiOptions) {
        self.lock().ui = ui;
        self.on_main(|p| p.apply_ui());
    }

    // ---------- from the page ----------

    /// `pill_snapshot` from a pill page: that page has loaded.
    pub fn page_called(&self, label: &str) {
        let mut i = self.lock();
        if i.label == label {
            i.page_loaded = true;
        }
    }

    pub fn ack(&self, seq: u64) {
        let mut i = self.lock();
        i.acked = i.acked.max(seq);
    }

    pub fn layout(self: &Arc<Self>, layout: Layout) {
        {
            let mut i = self.lock();
            if layout.seq < i.stop_seq {
                return;
            }
            i.layout = Some(layout);
        }
        self.on_main(|p| p.update_hit());
    }

    // ---------- event loop thread ----------

    fn on_main(self: &Arc<Self>, task: impl FnOnce(&Arc<Pill>) + Send + 'static) {
        let this = self.clone();
        let _ = self.app.run_on_main_thread(move || task(&this));
    }

    /// The show sequence: restore the size (#13), place on the monitor the user works on, show without
    /// activating, reapply the styles, raise (#14), put the stop window back, then watch for the paint.
    fn show_now(self: &Arc<Self>, seq: u64, at: Place) {
        let (edge, excluded, last) = {
            let i = self.lock();
            if !i.shown {
                return;
            }
            let Some(s) = i.snapshot.as_ref() else { return };
            (s.edge, i.ui.hide_pill_from_capture, i.monitor)
        };
        let Some(win) = self.window() else {
            // The window comes later (startup) or is being rebuilt; create() shows it then.
            return;
        };
        let _ = win.set_size(LogicalSize::new(place::WINDOW_W, place::WINDOW_H));
        let monitor = match (at, last) {
            (Place::Last, Some(last)) => Some(last),
            (Place::Choose, _) | (Place::Last, None) => Some(window::target_monitor()),
            (Place::Current, _) => window::current_monitor(&win),
        };
        if let Some(monitor) = monitor {
            window::place(&win, monitor, edge);
            self.lock().monitor = Some(monitor);
        }
        let _ = win.set_content_protected(excluded);
        window::show(&win);
        window::raise(&win);
        self.update_hit();
        window::foreground_hook(true);
        self.arm_watchdog(seq);
    }

    fn hide_now(self: &Arc<Self>, generation: u64) {
        {
            let mut i = self.lock();
            let still_hidden = i.snapshot.as_ref().is_some_and(|s| !s.visible);
            if i.hide_gen != generation || !still_hidden {
                return;
            }
            i.shown = false;
            i.watch_gen += 1;
        }
        window::foreground_hook(false);
        self.update_hit();
        if let Some(win) = self.window() {
            window::hide(&win);
        }
    }

    fn raise_now(self: &Arc<Self>) {
        if !self.lock().shown {
            return;
        }
        if let Some(win) = self.window() {
            window::apply_styles(&win);
            window::raise(&win);
        }
        self.update_hit();
        self.lock().last_raise = Some(Instant::now());
    }

    /// Size, position, styles and topmost again, so WebView2 paints (#13) and the pill is on top
    /// (#14). Event loop thread.
    fn nudge_now(self: &Arc<Self>) {
        if let Some(win) = self.window() {
            let _ = win.set_size(LogicalSize::new(place::WINDOW_W + 1.0, place::WINDOW_H));
            let _ = win.set_size(LogicalSize::new(place::WINDOW_W, place::WINDOW_H));
            let (monitor, edge) = {
                let i = self.lock();
                (i.monitor, i.snapshot.as_ref().map(|s| s.edge))
            };
            if let (Some(monitor), Some(edge)) = (monitor, edge) {
                window::place(&win, monitor, edge);
            }
            window::apply_styles(&win);
        }
        self.raise_now();
    }

    fn replace_if_moved(self: &Arc<Self>) {
        let Some(win) = self.window() else { return };
        let target = window::target_monitor();
        if window::current_monitor(&win) != Some(target) {
            let Some(edge) = self.lock().snapshot.as_ref().map(|s| s.edge) else {
                return;
            };
            let _ = win.set_size(LogicalSize::new(place::WINDOW_W, place::WINDOW_H));
            window::place(&win, target, edge);
            self.lock().monitor = Some(target);
        }
    }

    fn apply_ui(self: &Arc<Self>) {
        let ui = self.lock().ui.clone();
        if let Some(win) = self.window() {
            // Hide from capture takes effect at once, shown or not, so a hidden pill doesn't show up
            // in a capture during its first frames after the next show.
            let _ = win.set_content_protected(ui.hide_pill_from_capture);
            #[cfg(target_os = "macos")]
            crate::appearance::apply_to_window(&win, &ui);
            if self.lock().shown {
                window::apply_styles(&win);
            }
        }
        #[cfg(any(windows, target_os = "macos"))]
        for (cell, key) in [
            (&HIT, keys::STOP_BUTTON),
            (&SCREEN_HIT, keys::SCREEN_BUTTON),
        ] {
            cell.with(|hit| {
                if let Some(hit) = hit.borrow().as_ref() {
                    hit.set_title(&self.catalog.text(ui.locale, key));
                    #[cfg(target_os = "macos")]
                    {
                        hit.set_appearance(&ui);
                        hit.set_capture_excluded(ui.hide_pill_from_capture);
                    }
                }
            });
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = (&self.catalog, keys::STOP_BUTTON, keys::SCREEN_BUTTON);
        // The button windows take the new capture setting too.
        self.update_hit();
    }

    /// Called from the foreground hook: raise at once, or once the interval has passed.
    pub(crate) fn foreground_changed(self: &Arc<Self>) {
        let wait = {
            let mut i = self.lock();
            if !i.shown || i.raise_queued {
                return;
            }
            match i.last_raise.map(|at| at.elapsed()) {
                Some(since) if since < RAISE_INTERVAL => {
                    i.raise_queued = true;
                    Some(RAISE_INTERVAL - since)
                }
                _ => None,
            }
        };
        match wait {
            None => self.raise_now(),
            Some(wait) => {
                let this = self.clone();
                thread::spawn(move || {
                    thread::sleep(wait);
                    this.on_main(|p| {
                        p.lock().raise_queued = false;
                        p.raise_now();
                    });
                });
            }
        }
    }

    /// Called from the window procedure on a display change.
    pub(crate) fn display_changed(self: &Arc<Self>) {
        let generation = {
            let mut i = self.lock();
            i.repair_gen += 1;
            i.repair_gen
        };
        let this = self.clone();
        thread::spawn(move || {
            thread::sleep(SETTLE);
            this.on_main(move |p| p.repair(generation));
        });
    }

    fn repair(self: &Arc<Self>, generation: u64) {
        let (shown, seq) = {
            let i = self.lock();
            if i.repair_gen != generation {
                return;
            }
            (i.shown, i.snapshot.as_ref().map_or(0, |s| s.seq))
        };
        log::info!(
            "pill: display changed; repairing the {} pill",
            if shown { "visible" } else { "hidden" }
        );
        if shown {
            // Same monitor, or the nearest one if it went away.
            self.show_now(seq, Place::Current);
        } else if let Some(win) = self.window() {
            let _ = win.set_size(LogicalSize::new(place::WINDOW_W, place::WINDOW_H));
            window::apply_styles(&win);
        }
    }

    // ---------- the stop and screen windows ----------

    #[cfg(any(windows, target_os = "macos"))]
    fn ensure_hit(self: &Arc<Self>) {
        let ui = self.lock().ui.clone();
        for (cell, target, key) in [
            (&HIT, Target::Stop, keys::STOP_BUTTON),
            (&SCREEN_HIT, Target::Screen, keys::SCREEN_BUTTON),
        ] {
            cell.with(|cell| {
                if cell.borrow().is_some() {
                    return;
                }
                let title = self.catalog.text(ui.locale, key);
                match ButtonWindow::create(&title, self.hit_handler(target)) {
                    Ok(window) => {
                        #[cfg(target_os = "macos")]
                        window.set_appearance(&ui);
                        *cell.borrow_mut() = Some(window)
                    }
                    Err(error) => {
                        log::error!("pill: could not create the {target:?} window: {error}")
                    }
                }
            });
        }
    }

    /// What a click or a pointer move on one of the native button windows does.
    #[cfg(any(windows, target_os = "macos"))]
    fn hit_handler(self: &Arc<Self>, target: Target) -> ButtonHandler {
        let this = Arc::downgrade(self);
        Box::new(move |event: ButtonEvent| {
            let Some(pill) = this.upgrade() else { return };
            match event {
                ButtonEvent::Pointer { hover, pressed } => {
                    let label = pill.label();
                    let _ = pill.app.emit_to(
                        label.as_str(),
                        "pill://pointer",
                        json!({ "hover": hover, "pressed": pressed, "target": target.as_str() }),
                    );
                }
                ButtonEvent::Click => {
                    // Both talk to a process of their own; keep them off the event loop.
                    if let Some(engine) = pill.engine.get() {
                        thread::spawn(move || match target {
                            Target::Stop => engine.pill_stop_clicked(),
                            Target::Screen => engine.pill_screen_clicked(),
                        });
                    }
                }
            }
        })
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn ensure_hit(self: &Arc<Self>) {}

    /// Shows the stop window over the stop button and the screen window over the screen button
    /// while the pill shows them, else hides them.
    #[cfg(any(windows, target_os = "macos"))]
    fn update_hit(self: &Arc<Self>) {
        let (want_stop, want_screen, button, screen_button, excluded) = {
            let i = self.lock();
            let snapshot = i.snapshot.as_ref().filter(|_| i.shown);
            (
                snapshot.is_some_and(has_stop),
                snapshot.is_some_and(has_screen),
                i.layout.as_ref().and_then(|l| l.button),
                i.layout.as_ref().and_then(|l| l.screen_button),
                i.ui.hide_pill_from_capture,
            )
        };
        let win = self.window();
        let rect = |want: bool, button: Option<place::CssRect>| match (want, button, &win) {
            (true, Some(button), Some(win)) => button_rect(win, &button),
            _ => None,
        };
        let show = |cell: &std::cell::RefCell<Option<ButtonWindow>>, rect: Option<ButtonRect>| {
            let cell = cell.borrow();
            let Some(hit) = cell.as_ref() else {
                return false;
            };
            match rect {
                Some(r) => {
                    exclude_from_capture(hit, excluded);
                    hit.show_at(r);
                }
                None => hit.hide(),
            }
            hit.is_visible()
        };
        let stop = rect(want_stop, button);
        let screen = rect(want_screen, screen_button);
        let visible = HIT.with(|cell| show(cell, stop));
        let screen_visible = SCREEN_HIT.with(|cell| show(cell, screen));
        let mut i = self.lock();
        i.hit_visible = visible;
        i.screen_hit_visible = screen_visible;
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn update_hit(self: &Arc<Self>) {}

    // ---------- the paint watchdog ----------

    /// The page acks each snapshot after painting it. No ack for a shown snapshot means WebView2 shows
    /// nothing (#13): nudge the size after 500 ms, rebuild the window after 2 s.
    fn arm_watchdog(self: &Arc<Self>, seq: u64) {
        let generation = {
            let mut i = self.lock();
            i.watch_gen += 1;
            i.watch_gen
        };
        let this = self.clone();
        thread::spawn(move || this.watch(seq, generation));
    }

    /// `None` while the watch still matters and the snapshot isn't painted; the page's load state.
    fn watch_state(&self, seq: u64, generation: u64) -> Option<bool> {
        let i = self.lock();
        if i.watch_gen != generation || !i.shown || i.acked >= seq {
            return None;
        }
        Some(i.page_loaded)
    }

    fn watch(self: Arc<Self>, seq: u64, generation: u64) {
        let started = Instant::now();
        loop {
            match self.watch_state(seq, generation) {
                None => return,
                Some(true) => break,
                Some(false) if started.elapsed() >= PAGE_LOAD_LIMIT => break,
                Some(false) => thread::sleep(WATCH_POLL),
            }
        }
        let mut clock = PaintClock::new(Instant::now());
        let mut waiting_logged = false;
        loop {
            thread::sleep(WATCH_POLL);
            if self.watch_state(seq, generation).is_none() {
                return;
            }
            let expected = self.paint_expected();
            if !expected && !waiting_logged {
                log::info!(
                    "pill: snapshot {seq} not painted while the pill can't be seen; the watchdog waits"
                );
                waiting_logged = true;
            }
            match clock.step(Instant::now(), expected) {
                None => {}
                Some(Repair::Nudge) => {
                    if self.claim_repair(seq, Repair::Nudge) {
                        log::warn!(
                            "pill: no paint {} ms after showing snapshot {seq}; nudging the window",
                            NUDGE_AFTER.as_millis()
                        );
                        self.on_main(|p| p.nudge_now());
                    }
                }
                Some(_) => {
                    if self.claim_repair(seq, Repair::Rebuild) {
                        self.rebuild(seq);
                    }
                    return;
                }
            }
        }
    }

    /// Whether the page can paint now. macOS' WebKit runs no frames while the pill can't be seen (the
    /// window wholly covered, the lock screen, a switched-out session), so a missing acknowledgement
    /// then says nothing about the window. Asked on the event loop thread, which owns the window; an
    /// event loop that doesn't answer counts as painting, so the watchdog still repairs.
    #[cfg(target_os = "macos")]
    fn paint_expected(self: &Arc<Self>) -> bool {
        let ask = |pill: &Pill| pill.window().is_none_or(|win| window::paint_expected(&win));
        if objc2::MainThreadMarker::new().is_some() {
            return ask(self);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.on_main(move |p| {
            let _ = tx.send(ask(p));
        });
        rx.recv_timeout(Duration::from_secs(1)).unwrap_or(true)
    }

    #[cfg(not(target_os = "macos"))]
    fn paint_expected(self: &Arc<Self>) -> bool {
        true
    }

    /// Replaces the pill window with a new one. Runs on the watchdog's thread, never the event loop's.
    fn rebuild(self: &Arc<Self>, seq: u64) {
        let (old, label, rebuilds) = {
            let mut i = self.lock();
            if i.rebuilds > 0 && i.rebuilds % 3 == 0 {
                // Three rebuilds in a row didn't help; wait for the next show rather than loop.
                log::error!(
                    "pill: still no paint after {} rebuilds; waiting for the next show",
                    i.rebuilds
                );
                i.rebuilds += 1;
                return;
            }
            i.rebuilds += 1;
            let old = i.label.clone();
            i.label = format!("{LABEL}-r{}", i.rebuilds);
            i.page_loaded = false;
            (old, i.label.clone(), i.rebuilds)
        };
        log::warn!("pill: no paint {} ms after showing snapshot {seq}; rebuilding the window as {label} (rebuild {rebuilds})", REBUILD_AFTER.as_millis());
        if let Some(win) = self.app.get_webview_window(&old) {
            let _ = win.destroy();
        }
        // create() shows the new window; its page asks for the current snapshot when it loads.
        if let Err(error) = self.create() {
            log::error!("pill: could not rebuild the window: {error}");
        }
    }
}

/// Which native button window took the pointer. Windows and macOS have those windows; the pill's tests
/// cover this on every platform.
#[cfg(any(windows, target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Stop,
    Screen,
}

#[cfg(any(windows, target_os = "macos", test))]
impl Target {
    /// `pill://pointer`'s `target`.
    fn as_str(self) -> &'static str {
        match self {
            Target::Stop => "stop",
            Target::Screen => "screen",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(phase: PillPhase, stop: bool, switch: bool) -> PillSnapshot {
        PillSnapshot {
            seq: 1,
            visible: true,
            phase,
            activity: voice_engine::Activity::Dictate,
            notice: None,
            words: None,
            meeting: Some(voice_engine::MeetingPill {
                state: "recording".into(),
                at_ms: 0,
                screen: "frames".into(),
                screen_switch: switch,
                compact: false,
                confirm_stop: false,
                stop_enabled: stop,
                rukoo_connected: false,
            }),
            edge: voice_engine::Edge::Bottom,
            theme: voice_engine::Theme::Dark,
            locale: voice_engine::Locale::En,
            dev: false,
        }
    }

    #[test]
    fn the_screen_button_shows_only_on_a_meeting_pill_that_can_switch() {
        assert!(has_screen(&snapshot(PillPhase::Meeting, true, true)));
        assert!(!has_screen(&snapshot(PillPhase::Meeting, true, false)));
        // A dictation during the meeting covers the meeting's buttons.
        assert!(!has_screen(&snapshot(PillPhase::Listening, false, true)));
        let mut hidden = snapshot(PillPhase::Meeting, true, true);
        hidden.visible = false;
        assert!(!has_screen(&hidden) && !has_stop(&hidden));
        assert_eq!(Target::Screen.as_str(), "screen");
        assert_eq!(Target::Stop.as_str(), "stop");
        let layout: Layout = serde_json::from_str(
            r#"{ "seq": 9, "pill": null, "button": { "x": 257, "y": 28, "width": 28, "height": 28 }, "screenButton": { "x": 225, "y": 28, "width": 28, "height": 28 } }"#,
        )
        .unwrap();
        assert_eq!(layout.screen_button.unwrap().x, 225.0);
        let old: Layout =
            serde_json::from_str(r#"{ "seq": 3, "pill": null, "button": null }"#).unwrap();
        assert!(old.screen_button.is_none());
    }

    #[test]
    fn the_watchdog_nudges_then_rebuilds_and_only_counts_while_the_pill_can_paint() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut clock = PaintClock::new(t0);
        assert_eq!(clock.step(t0 + ms(400), true), None);
        assert_eq!(clock.step(t0 + ms(500), true), Some(Repair::Nudge));
        assert_eq!(clock.step(t0 + ms(600), true), None, "one nudge");
        assert_eq!(clock.step(t0 + ms(2000), true), Some(Repair::Rebuild));

        // Covered or locked from 300 ms to a minute later: nothing, however long it lasts.
        let mut clock = PaintClock::new(t0);
        assert_eq!(clock.step(t0 + ms(300), true), None);
        for at in [400, 2_000, 30_000, 60_000] {
            assert_eq!(clock.step(t0 + ms(at), false), None, "at {at} ms");
        }
        // Seen again: the full grace period from there.
        assert_eq!(clock.step(t0 + ms(60_400), true), None);
        assert_eq!(clock.step(t0 + ms(60_500), true), Some(Repair::Nudge));
        assert_eq!(clock.step(t0 + ms(61_999), true), None);
        assert_eq!(clock.step(t0 + ms(62_000), true), Some(Repair::Rebuild));

        // Nudged, then covered: after it is seen again it may nudge once more before the rebuild.
        let mut clock = PaintClock::new(t0);
        assert_eq!(clock.step(t0 + ms(500), true), Some(Repair::Nudge));
        assert_eq!(clock.step(t0 + ms(1_900), false), None);
        assert_eq!(clock.step(t0 + ms(2_400), true), Some(Repair::Nudge));
        assert_eq!(clock.step(t0 + ms(3_900), true), Some(Repair::Rebuild));
    }

    #[test]
    fn one_repair_per_snapshot_and_step() {
        let none = (0, Repair::None);
        assert!(repair_due(true, 4, 5, none, Repair::Nudge));
        // Painted, or hidden: nothing to repair.
        assert!(!repair_due(true, 5, 5, none, Repair::Nudge));
        assert!(!repair_due(false, 4, 5, none, Repair::Rebuild));
        // The other watchdog nudged this snapshot already; a rebuild can still follow.
        let nudged = (5, Repair::Nudge);
        assert!(!repair_due(true, 4, 5, nudged, Repair::Nudge));
        assert!(repair_due(true, 4, 5, nudged, Repair::Rebuild));
        assert!(!repair_due(
            true,
            4,
            5,
            (5, Repair::Rebuild),
            Repair::Rebuild
        ));
        // A late repair of an older snapshot after a newer one was handled does nothing.
        assert!(!repair_due(true, 3, 4, nudged, Repair::Rebuild));
        // A newer snapshot gets its own repairs.
        assert!(repair_due(true, 5, 6, (5, Repair::Rebuild), Repair::Nudge));
    }
}
