//! Opening folders and links and choosing a folder, behind a trait so the commands can be tested without
//! opening anything.

use std::path::{Path, PathBuf};

pub trait Shell: Send + Sync {
    fn open_path(&self, path: &Path) -> Result<(), String>;
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// The system's folder picker over `owner` (a window handle), starting in `start` when the platform
    /// uses it. `Ok(None)` when the user cancels.
    fn pick_folder(
        &self,
        title: &str,
        start: Option<&Path>,
        owner: Option<isize>,
    ) -> Result<Option<PathBuf>, String>;
}

pub struct SystemShell {
    /// The app, so the folder picker can reach the main thread: AppKit panels and windows live there.
    #[cfg(target_os = "macos")]
    app: tauri::AppHandle,
}

impl SystemShell {
    #[cfg(target_os = "macos")]
    pub fn new(app: &tauri::AppHandle) -> SystemShell {
        SystemShell { app: app.clone() }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn new(_app: &tauri::AppHandle) -> SystemShell {
        SystemShell {}
    }
}

/// How the macOS folder panel ended, kept apart from AppKit's constants so the mapping below is testable
/// everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum PanelAnswer {
    Ok,
    Cancel,
    Abort,
}

/// The panel's answer as the picker's result: `Ok(None)` when the user cancelled, so the caller leaves the
/// setting as it was; an error when the panel showed no folder or couldn't open.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn folder_from_panel(
    answer: PanelAnswer,
    folder: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    match answer {
        PanelAnswer::Ok => match folder.filter(|path| !path.as_os_str().is_empty()) {
            Some(path) => Ok(Some(path)),
            None => Err("The folder panel closed without a folder.".to_string()),
        },
        PanelAnswer::Cancel => Ok(None),
        PanelAnswer::Abort => Err("The folder panel couldn't open.".to_string()),
    }
}

#[cfg(windows)]
impl Shell for SystemShell {
    fn open_path(&self, path: &Path) -> Result<(), String> {
        shell_open(&path.to_string_lossy())
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        shell_open(url)
    }

    fn pick_folder(
        &self,
        title: &str,
        _start: Option<&Path>,
        owner: Option<isize>,
    ) -> Result<Option<PathBuf>, String> {
        let title = title.to_string();
        // Its own single-threaded apartment, as the dialog requires; the command thread may have another.
        std::thread::spawn(move || pick_folder_sta(&title, owner))
            .join()
            .map_err(|_| "The folder picker stopped unexpectedly.".to_string())?
    }
}

#[cfg(windows)]
fn shell_open(target: &str) -> Result<(), String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success as a value above 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "Windows could not open {target} (error {}).",
            result.0 as isize
        ))
    }
}

#[cfg(windows)]
fn pick_folder_sta(title: &str, owner: Option<isize>) -> Result<Option<PathBuf>, String> {
    use windows::core::{HRESULT, HSTRING};
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS,
        SIGDN_FILESYSPATH,
    };

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = (|| -> windows::core::Result<Option<PathBuf>> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            let options = dialog.GetOptions()?;
            dialog
                .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
            dialog.SetTitle(&HSTRING::from(title))?;
            if let Err(error) = dialog.Show(owner.map(|h| HWND(h as *mut _))) {
                if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                    return Ok(None);
                }
                return Err(error);
            }
            let item = dialog.GetResult()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const _));
            Ok(Some(PathBuf::from(path.map_err(|e| {
                windows::core::Error::new(HRESULT(-1), e.to_string())
            })?)))
        })();
        CoUninitialize();
        result.map_err(|e| e.message())
    }
}

#[cfg(target_os = "macos")]
impl Shell for SystemShell {
    fn open_path(&self, path: &Path) -> Result<(), String> {
        open_with_system(&path.to_string_lossy())
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        open_with_system(url)
    }

    fn pick_folder(
        &self,
        title: &str,
        start: Option<&Path>,
        owner: Option<isize>,
    ) -> Result<Option<PathBuf>, String> {
        mac::pick_folder(&self.app, title, start, owner)
    }
}

/// A sheet needs a run loop of its own, and the caller here already blocks the thread it runs on, so the
/// panel goes to the main thread and the answer comes back through a channel.
#[cfg(target_os = "macos")]
mod mac {
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSModalResponse, NSModalResponseAbort, NSModalResponseCancel, NSModalResponseOK,
        NSOpenPanel, NSWindow,
    };
    use objc2_foundation::{NSString, NSURL};

    use super::{folder_from_panel, PanelAnswer};

    pub fn pick_folder(
        app: &tauri::AppHandle,
        title: &str,
        start: Option<&Path>,
        owner: Option<isize>,
    ) -> Result<Option<PathBuf>, String> {
        let title = title.to_string();
        let start = start.map(Path::to_path_buf);
        // Only the commands call this, on Tauri's blocking pool. From the main thread a sheet would wait on
        // a loop this call never returns to, so show the panel modally instead, as the ownerless case does.
        if let Some(mtm) = MainThreadMarker::new() {
            let panel = panel(mtm, &title, start.as_deref());
            return answer(panel.runModal(), &panel);
        }
        let (tx, rx) = mpsc::channel();
        app.run_on_main_thread(move || present(tx, title, start, owner))
            .map_err(|e| format!("The folder picker couldn't reach the main thread: {e}"))?;
        rx.recv()
            .map_err(|_| "The folder picker stopped unexpectedly.".to_string())?
    }

    /// The panel with Voice's rules, on the main thread.
    fn panel(
        mtm: MainThreadMarker,
        title: &str,
        start: Option<&Path>,
    ) -> objc2::rc::Retained<NSOpenPanel> {
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(true);
        panel.setCanChooseFiles(false);
        panel.setCanCreateDirectories(true);
        panel.setAllowsMultipleSelection(false);
        panel.setResolvesAliases(true);
        panel.setTitle(Some(&NSString::from_str(title)));
        if let Some(start) = start {
            let path = NSString::from_str(&start.display().to_string());
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&path)));
        }
        panel
    }

    /// Shows the panel and sends its answer: a sheet when Settings' window is known, else application-modal.
    fn present(
        reply: mpsc::Sender<Result<Option<PathBuf>, String>>,
        title: String,
        start: Option<PathBuf>,
        owner: Option<isize>,
    ) {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = reply.send(Err("The folder picker needs the main thread.".to_string()));
            return;
        };
        let panel = panel(mtm, &title, start.as_deref());
        log::info!(
            "folder picker: opening {}",
            if owner.is_some() {
                "as a sheet on Settings"
            } else {
                "modally, without an owner window"
            }
        );
        match owner {
            Some(owner) => {
                // The window outlives the sheet: Settings can't be used while the sheet is up.
                let window = unsafe { &*(owner as *const NSWindow) };
                // The panel reads its URL after the sheet closes; the copy keeps it alive for that.
                let answered = panel.clone();
                let handler = RcBlock::new(move |response: NSModalResponse| {
                    let _ = reply.send(answer(response, &answered));
                });
                panel.beginSheetModalForWindow_completionHandler(window, &handler);
            }
            None => {
                let result = answer(panel.runModal(), &panel);
                let _ = reply.send(result);
            }
        }
    }

    /// The panel's response and chosen URL as the picker's result.
    fn answer(response: NSModalResponse, panel: &NSOpenPanel) -> Result<Option<PathBuf>, String> {
        let answer = if response == NSModalResponseOK {
            PanelAnswer::Ok
        } else if response == NSModalResponseCancel {
            PanelAnswer::Cancel
        } else if response == NSModalResponseAbort {
            PanelAnswer::Abort
        } else {
            return Err(format!(
                "The folder panel stopped unexpectedly (response {response})."
            ));
        };
        let folder = panel
            .URL()
            .and_then(|url| url.path())
            .map(|path| PathBuf::from(path.to_string()));
        folder_from_panel(answer, folder)
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
impl Shell for SystemShell {
    fn open_path(&self, path: &Path) -> Result<(), String> {
        open_with_system(&path.to_string_lossy())
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        open_with_system(url)
    }

    fn pick_folder(
        &self,
        _title: &str,
        _start: Option<&Path>,
        _owner: Option<isize>,
    ) -> Result<Option<PathBuf>, String> {
        Err("Choosing a folder is only supported on Windows and macOS for now.".to_string())
    }
}

#[cfg(not(windows))]
fn open_with_system(target: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open {target}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_the_panel_leaves_the_setting_alone() {
        assert_eq!(folder_from_panel(PanelAnswer::Cancel, None), Ok(None));
        // macOS can report a URL even on cancel; it still means "no folder".
        assert_eq!(
            folder_from_panel(PanelAnswer::Cancel, Some(PathBuf::from("/tmp"))),
            Ok(None)
        );
    }

    #[test]
    fn a_chosen_folder_is_the_choice() {
        assert_eq!(
            folder_from_panel(PanelAnswer::Ok, Some(PathBuf::from("/tmp/models"))),
            Ok(Some(PathBuf::from("/tmp/models")))
        );
    }

    #[test]
    fn ok_without_a_folder_and_a_panel_that_never_opened_are_errors() {
        assert!(folder_from_panel(PanelAnswer::Ok, None).is_err());
        assert!(folder_from_panel(PanelAnswer::Ok, Some(PathBuf::new())).is_err());
        assert!(folder_from_panel(PanelAnswer::Abort, None).is_err());
    }
}
