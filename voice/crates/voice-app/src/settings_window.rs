//! The Settings window (pill-settings-design.md 4.1): built on demand, centred on the monitor under the
//! cursor, painted in the theme's background before the page loads so it never flashes white.

use std::path::PathBuf;

use tauri::window::Color;
use tauri::{
    AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use voice_engine::{EngineApi, Theme, UiOptions};

use crate::appearance;
use crate::catalog::{keys, Catalog};
use crate::pill::place::Rect;

pub const LABEL: &str = "settings";
const WIDTH: f64 = 760.0;
const HEIGHT: f64 = 680.0;
const MIN: f64 = 480.0;

/// The pages and groups `settings.html?page=` opens at (voice/ui/API.md).
pub const PAGES: &[&str] = &[
    "transcription",
    "dictionary",
    "fillers",
    "rewriting",
    "meetingTranscription",
    "speakers",
    "storage",
    "rukoo",
    "import",
    "status",
    "dictation",
    "meetings",
    "general",
    "about",
];

pub fn url(page: Option<&str>) -> String {
    match page {
        Some(page) if PAGES.contains(&page) => format!("settings.html?page={page}"),
        Some(page) => {
            log::warn!("settings: unknown start page {page}; opening the top page");
            "settings.html".to_string()
        }
        None => "settings.html".to_string(),
    }
}

/// The page's `--bg`.
pub fn background(theme: Theme) -> Color {
    match theme {
        Theme::Dark => Color(0x14, 0x15, 0x19, 0xff),
        Theme::Light => Color(0xf3, 0xf4, 0xf6, 0xff),
    }
}

/// The top left that centres a window of `size` in `work`, kept inside it.
pub fn centered(work: &Rect, size: (i32, i32)) -> (i32, i32) {
    let x = work.left + ((work.width() - size.0) / 2).max(0);
    let y = work.top + ((work.height() - size.1) / 2).max(0);
    (x, y)
}

/// Shows Settings, building it first at `page` when needed. Building blocks on the event loop, so call
/// this from any thread but that one.
pub fn open(
    app: &AppHandle,
    engine: &dyn EngineApi,
    catalog: &Catalog,
    webview_dir: Option<PathBuf>,
    focus: bool,
    page: Option<String>,
) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        window.show()?;
        if focus {
            window.set_focus()?;
        }
        return Ok(window);
    }
    let ui = engine.ui();
    let mut builder =
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(url(page.as_deref()).into()))
            .title(catalog.text(ui.locale, keys::SETTINGS_TITLE))
            .inner_size(WIDTH, HEIGHT)
            .min_inner_size(MIN, MIN)
            .resizable(true)
            .background_color(background(ui.theme))
            .focused(focus)
            .visible(false)
            .disable_drag_drop_handler();
    if let Some(dir) = webview_dir {
        builder = builder.data_directory(dir);
    }
    let window = builder.build()?;
    // Not through the builder's `.theme(..)`: on macOS tao would pin the whole app with it.
    appearance::apply_to_window(&window, &ui);
    appearance::log_state(&window, "settings window built");
    place_under_cursor(&window);
    window.show()?;
    if focus {
        window.set_focus()?;
    }
    Ok(window)
}

fn place_under_cursor(window: &WebviewWindow) {
    let Some((work, target_scale)) = crate::pill::window::cursor_work_area() else {
        let _ = window.center();
        return;
    };
    let (Ok(size), Ok(scale)) = (window.outer_size(), window.scale_factor()) else {
        return;
    };
    // The window may sit on a monitor with another scale until it moves; size it for the target.
    let factor = target_scale / scale;
    let size = (
        (f64::from(size.width) * factor).round() as i32,
        (f64::from(size.height) * factor).round() as i32,
    );
    let (x, y) = centered(&work, size);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Theme or language changed while Settings is open.
pub fn apply_ui(app: &AppHandle, ui: &UiOptions, catalog: &Catalog) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let _ = window.set_title(&catalog.text(ui.locale, keys::SETTINGS_TITLE));
    let _ = window.set_background_color(Some(background(ui.theme)));
    appearance::apply_to_window(&window, ui);
    appearance::log_state(&window, "settings window restyled");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_pages_open_there_and_others_open_the_top_page() {
        assert_eq!(url(None), "settings.html");
        assert_eq!(url(Some("import")), "settings.html?page=import");
        assert_eq!(
            url(Some("meetingTranscription")),
            "settings.html?page=meetingTranscription"
        );
        assert_eq!(url(Some("../../etc")), "settings.html");
    }

    #[test]
    fn the_background_is_the_pages_bg_token() {
        assert_eq!(background(Theme::Dark), Color(0x14, 0x15, 0x19, 0xff));
        assert_eq!(background(Theme::Light), Color(0xf3, 0xf4, 0xf6, 0xff));
        let tokens = include_str!("../../../ui/tokens.css").to_lowercase();
        assert!(
            tokens.contains("#141519") && tokens.contains("#f3f4f6"),
            "tokens.css changed its --bg"
        );
    }

    #[test]
    fn the_window_centres_in_the_work_area_and_stays_inside() {
        let work = Rect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        assert_eq!(centered(&work, (760, 680)), (-1920 + 580, 180));
        let small = Rect {
            left: 0,
            top: 0,
            right: 600,
            bottom: 500,
        };
        assert_eq!(centered(&small, (1140, 1020)), (0, 0));
    }
}
