//! The tray icon: its menu and tooltip follow the engine's [`TrayState`] and the catalogs, and it shows the
//! engine's notifications, as balloons on Windows and in Notification Center on macOS (notify_mac.rs).

use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};
use voice_engine::{Locale, Notification, TrayState};

use crate::catalog::{keys, Catalog};

pub const ID: &str = "voice";

/// Windows cuts a tray tooltip at 127 characters.
const TOOLTIP_MAX: usize = 127;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Settings,
    Start,
    Pause,
    Resume,
    Stop,
    Quit,
}

const ACTIONS: [TrayAction; 6] = [
    TrayAction::Settings,
    TrayAction::Start,
    TrayAction::Pause,
    TrayAction::Resume,
    TrayAction::Stop,
    TrayAction::Quit,
];

impl TrayAction {
    pub fn id(self) -> &'static str {
        match self {
            TrayAction::Settings => "settings",
            TrayAction::Start => "start",
            TrayAction::Pause => "pause",
            TrayAction::Resume => "resume",
            TrayAction::Stop => "stop",
            TrayAction::Quit => "quit",
        }
    }

    pub fn from_id(id: &str) -> Option<TrayAction> {
        ACTIONS.into_iter().find(|a| a.id() == id)
    }

    fn key(self) -> &'static str {
        match self {
            TrayAction::Settings => keys::TRAY_SETTINGS,
            TrayAction::Start => keys::TRAY_START,
            TrayAction::Pause => keys::TRAY_PAUSE,
            TrayAction::Resume => keys::TRAY_RESUME,
            TrayAction::Stop => keys::TRAY_STOP,
            TrayAction::Quit => keys::TRAY_QUIT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Item(TrayAction),
    Separator,
}

/// The menu for a state. Stop is always there while a meeting records or is paused, because the pill
/// can't take keyboard focus and stopping must work without it.
pub fn entries(state: &TrayState) -> Vec<Entry> {
    use Entry::{Item, Separator};
    let mut entries = vec![Item(TrayAction::Settings), Separator];
    match state.meeting.as_deref() {
        Some("recording") => entries.extend([Item(TrayAction::Pause), Item(TrayAction::Stop)]),
        Some("paused") => entries.extend([Item(TrayAction::Resume), Item(TrayAction::Stop)]),
        // No meeting, or the last one still saving: a new one can start.
        _ => entries.push(Item(TrayAction::Start)),
    }
    entries.extend([Separator, Item(TrayAction::Quit)]);
    entries
}

pub fn tooltip(state: &TrayState, catalog: &Catalog, locale: Locale) -> String {
    let recording = matches!(state.meeting.as_deref(), Some("recording" | "paused"));
    let base = catalog.text(
        locale,
        if recording {
            keys::TRAY_TOOLTIP_RECORDING
        } else {
            keys::TRAY_TOOLTIP
        },
    );
    let text = match state.status.as_deref() {
        Some(status) if !status.trim().is_empty() => format!("{base}\n{}", status.trim()),
        _ => base,
    };
    cut(&text, TOOLTIP_MAX)
}

/// At most `max` UTF-16 units, ending in an ellipsis when cut.
pub fn cut(text: &str, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        if used + c.len_utf16() > max - 1 {
            break;
        }
        used += c.len_utf16();
        out.push(c);
    }
    out.push('…');
    out
}

/// Copies `text` into a fixed, NUL-terminated UTF-16 field such as NOTIFYICONDATAW's. Windows' balloon
/// is its only caller; the test that pins the cutting runs on every platform.
#[cfg(any(windows, test))]
pub fn fill_wide(field: &mut [u16], text: &str) {
    let text = cut(text, field.len().saturating_sub(1));
    field.fill(0);
    for (slot, unit) in field.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
    }
}

pub struct Tray {
    app: AppHandle,
    catalog: Arc<Catalog>,
    state: Mutex<(TrayState, Locale)>,
    /// The tray icon's window, which a Windows balloon needs.
    #[cfg(windows)]
    icon_window: Mutex<isize>,
}

impl Tray {
    pub fn new(app: &AppHandle, catalog: Arc<Catalog>, locale: Locale) -> Arc<Tray> {
        Arc::new(Tray {
            app: app.clone(),
            catalog,
            state: Mutex::new((TrayState::default(), locale)),
            #[cfg(windows)]
            icon_window: Mutex::new(0),
        })
    }

    /// Puts the icon in the tray. Call on the event loop thread (setup), once the engine runs.
    pub fn install(
        &self,
        on_action: impl Fn(TrayAction) + Send + Sync + 'static,
    ) -> tauri::Result<()> {
        let on_action = Arc::new(on_action);
        let on_menu = on_action.clone();
        let icon = self
            .app
            .default_window_icon()
            .cloned()
            .ok_or_else(|| tauri::Error::AssetNotFound("icons/icon.ico".into()))?;
        let built = TrayIconBuilder::with_id(ID)
            .icon(icon)
            .tooltip(self.tooltip())
            .menu(&self.menu()?)
            .show_menu_on_left_click(false)
            .on_menu_event(move |_, event| {
                if let Some(action) = TrayAction::from_id(event.id().as_ref()) {
                    on_menu(action);
                }
            })
            .on_tray_icon_event(move |_, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    on_action(TrayAction::Settings);
                }
            })
            // A menu bar icon is a template image: macOS draws it from its alpha in black or white to match
            // the menu bar, so it stays readable in both themes. The mark's outline becomes the glyph.
            .icon_as_template(cfg!(target_os = "macos"))
            .build(&self.app)?;
        #[cfg(windows)]
        {
            let window = built
                .with_inner_tray_icon(|inner| inner.window_handle() as isize)
                .unwrap_or(0);
            *self.icon_window.lock().unwrap_or_else(|e| e.into_inner()) = window;
        }
        #[cfg(not(windows))]
        let _ = built;
        Ok(())
    }

    fn tooltip(&self) -> String {
        let (state, locale) = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        tooltip(&state, &self.catalog, locale)
    }

    fn menu(&self) -> tauri::Result<Menu<Wry>> {
        let (state, locale) = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let menu = Menu::new(&self.app)?;
        for entry in entries(&state) {
            match entry {
                Entry::Item(action) => menu.append(&MenuItem::with_id(
                    &self.app,
                    action.id(),
                    self.catalog.text(locale, action.key()),
                    true,
                    None::<&str>,
                )?)?,
                Entry::Separator => menu.append(&PredefinedMenuItem::separator(&self.app)?)?,
            }
        }
        Ok(menu)
    }

    pub fn update(self: &Arc<Self>, state: TrayState) {
        {
            let mut current = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if current.0 == state {
                return;
            }
            current.0 = state;
        }
        self.refresh();
    }

    pub fn set_locale(self: &Arc<Self>, locale: Locale) {
        {
            let mut current = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if current.1 == locale {
                return;
            }
            current.1 = locale;
        }
        self.refresh();
    }

    fn refresh(self: &Arc<Self>) {
        let this = self.clone();
        let _ = self.app.run_on_main_thread(move || {
            let Some(icon) = this.app.tray_by_id(ID) else {
                return;
            };
            match this.menu() {
                Ok(menu) => {
                    let _ = icon.set_menu(Some(menu));
                }
                Err(error) => log::warn!("tray: could not build the menu: {error}"),
            }
            let _ = icon.set_tooltip(Some(this.tooltip()));
        });
    }

    pub fn notify(&self, notification: &Notification) {
        let locale = self.state.lock().unwrap_or_else(|e| e.into_inner()).1;
        let title = match self.catalog.get(locale, &notification.title_key) {
            Some(title) => title.to_string(),
            None => {
                log::warn!(
                    "tray: no catalog string {}; using the product name",
                    notification.title_key
                );
                self.catalog.text(locale, keys::TRAY_TOOLTIP)
            }
        };
        log::info!("notification: {title}: {}", notification.body);
        #[cfg(windows)]
        {
            let window = *self.icon_window.lock().unwrap_or_else(|e| e.into_inner());
            if window == 0 || !balloon(window, &title, &notification.body) {
                log::warn!("tray: could not show the notification");
            }
        }
        #[cfg(target_os = "macos")]
        crate::notify_mac::notify(&title, &notification.body);
    }
}

#[cfg(windows)]
fn balloon(window: isize, title: &str, body: &str) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_INFO, NIIF_INFO, NIIF_RESPECT_QUIET_TIME, NIM_MODIFY,
        NOTIFYICONDATAW,
    };

    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(window as *mut _),
        uFlags: NIF_INFO,
        dwInfoFlags: NIIF_INFO | NIIF_RESPECT_QUIET_TIME,
        ..Default::default()
    };
    fill_wide(&mut data.szInfoTitle, title);
    fill_wide(&mut data.szInfo, body);
    // tray-icon numbers its icons from 1 and keeps the number to itself. The window belongs to our one
    // icon, so the first number Windows accepts for it is ours; a wrong number changes nothing.
    (1..=8).any(|id| {
        data.uID = id;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tray(meeting: Option<&str>) -> TrayState {
        TrayState {
            meeting: meeting.map(str::to_string),
            meeting_at_ms: None,
            status: None,
        }
    }

    fn items(state: &TrayState) -> Vec<TrayAction> {
        entries(state)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item(a) => Some(a),
                Entry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_follows_the_meeting() {
        use TrayAction::*;
        assert_eq!(items(&tray(None)), vec![Settings, Start, Quit]);
        assert_eq!(
            items(&tray(Some("recording"))),
            vec![Settings, Pause, Stop, Quit]
        );
        assert_eq!(
            items(&tray(Some("paused"))),
            vec![Settings, Resume, Stop, Quit]
        );
        assert_eq!(
            items(&tray(Some("processing"))),
            vec![Settings, Start, Quit]
        );
        assert_eq!(entries(&tray(None)).first(), Some(&Entry::Item(Settings)));
        assert_eq!(entries(&tray(None)).last(), Some(&Entry::Item(Quit)));
    }

    #[test]
    fn ids_round_trip() {
        for action in ACTIONS {
            assert_eq!(TrayAction::from_id(action.id()), Some(action));
        }
        assert_eq!(TrayAction::from_id("nope"), None);
    }

    #[test]
    fn the_tooltip_says_when_a_meeting_records() {
        let catalog = Catalog::embedded();
        assert_eq!(tooltip(&tray(None), &catalog, Locale::En), "Rukoo Voice");
        assert_eq!(
            tooltip(&tray(Some("recording")), &catalog, Locale::En),
            "Rukoo Voice is recording a meeting"
        );
        assert_eq!(
            tooltip(&tray(Some("paused")), &catalog, Locale::Nl),
            "Rukoo Voice neemt een vergadering op"
        );
        let with_status = TrayState {
            status: Some("Orukeet loading".into()),
            ..tray(None)
        };
        assert_eq!(
            tooltip(&with_status, &catalog, Locale::En),
            "Rukoo Voice\nOrukeet loading"
        );
        let long = TrayState {
            status: Some("x".repeat(300)),
            ..tray(None)
        };
        let tip = tooltip(&long, &catalog, Locale::En);
        assert_eq!(tip.encode_utf16().count(), TOOLTIP_MAX);
        assert!(tip.ends_with('…'));
    }

    #[test]
    fn every_menu_label_is_translated() {
        let catalog = Catalog::embedded();
        for action in ACTIONS {
            let en = catalog.text(Locale::En, action.key());
            let nl = catalog.text(Locale::Nl, action.key());
            assert!(
                !en.starts_with("app.") && !nl.starts_with("app."),
                "{action:?}"
            );
        }
        assert_eq!(
            catalog.text(Locale::Nl, TrayAction::Stop.key()),
            "Vergadering stoppen en opslaan"
        );
    }

    #[test]
    fn wide_fields_are_cut_and_terminated() {
        let mut field = [0xffffu16; 8];
        fill_wide(&mut field, "Hi");
        assert_eq!(&field[..3], &[u16::from(b'H'), u16::from(b'i'), 0]);
        assert!(field[3..].iter().all(|u| *u == 0));
        fill_wide(&mut field, "A much longer title");
        assert_eq!(field[7], 0);
        assert_eq!(String::from_utf16_lossy(&field[..7]), "A much…");
        // A surrogate pair is never split.
        let mut small = [0u16; 4];
        fill_wide(&mut small, "ab😀c");
        assert_eq!(String::from_utf16_lossy(&small[..3]), "ab…");
    }
}
