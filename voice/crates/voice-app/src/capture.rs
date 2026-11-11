//! A PNG of one of the app's own pages, for the development self-test and the `capture` hook: WebView2's
//! CapturePreview on Windows, WKWebView's snapshot on macOS (`capture_mac.rs`). Both render the page
//! itself, so neither reads the screen or another app's windows.

#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use std::sync::mpsc;
#[cfg(windows)]
use std::time::Duration;

#[cfg(target_os = "macos")]
pub use crate::capture_mac::capture_png;

#[cfg(windows)]
use tauri::WebviewWindow;
#[cfg(windows)]
use webview2_com::CapturePreviewCompletedHandler;
#[cfg(windows)]
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller, COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
};
#[cfg(windows)]
use windows::Win32::System::Com::{IStream, STREAM_SEEK_SET};
#[cfg(windows)]
use windows::Win32::UI::Shell::SHCreateMemStream;

#[cfg(windows)]
type Done = mpsc::Sender<Result<Vec<u8>, String>>;

#[cfg(windows)]
pub fn capture_png(window: &WebviewWindow, path: &Path) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    window
        .with_webview(move |webview| {
            if let Err(error) = start(webview.controller(), tx.clone()) {
                let _ = tx.send(Err(error));
            }
        })
        .map_err(|e| e.to_string())?;
    let png = rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| format!("No capture of {} within 15 s", path.display()))??;
    fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))
}

// WebView2 writes into memory; the file is written here once it's complete, so nobody reads it half done.
#[cfg(windows)]
fn start(controller: ICoreWebView2Controller, done: Done) -> Result<(), String> {
    unsafe {
        let core = controller.CoreWebView2().map_err(|e| e.message())?;
        let stream = SHCreateMemStream(None).ok_or("Could not make a memory stream")?;
        let filled = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
            let png = result
                .map_err(|e| e.message())
                .and_then(|()| read_all(&filled));
            let _ = done.send(png);
            Ok(())
        }));
        core.CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            &stream,
            &handler,
        )
        .map_err(|e| e.message())
    }
}

#[cfg(windows)]
fn read_all(stream: &IStream) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    unsafe {
        stream
            .Seek(0, STREAM_SEEK_SET, None)
            .map_err(|e| e.message())?;
        loop {
            let mut read = 0u32;
            stream
                .Read(buf.as_mut_ptr().cast(), buf.len() as u32, Some(&mut read))
                .ok()
                .map_err(|e| e.message())?;
            if read == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&buf[..read as usize]);
        }
    }
}
