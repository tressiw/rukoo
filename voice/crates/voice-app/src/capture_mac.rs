//! A PNG of one of the app's own pages through WKWebView's `takeSnapshotWithConfiguration:`, for the
//! development self-test and the `capture` hook. WebKit draws the web content itself, so it never reads
//! the screen, another app's windows or anything else on the display, and it needs no Screen Recording
//! permission.
//!
//! The snapshot is the view's size in points times its backing scale factor, so the PNG has the pixels
//! the display shows: the window's content at that display's sharpness, which is what a capture of the
//! window means on Windows too. A window on a 2x display therefore writes twice the pixels of its
//! points, which the log line and the self-test's report both name.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_foundation::{NSDictionary, NSError};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use tauri::WebviewWindow;

/// WebKit answers on the main thread's next turn; a snapshot that doesn't come back by then is stuck.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The page's PNG and the pixel size WebKit drew it at.
type Png = (Vec<u8>, u32, u32);

pub fn capture_png(window: &WebviewWindow, path: &Path) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let target = path.to_path_buf();
    window
        .with_webview(move |webview| {
            if let Err(error) = start(webview.inner().cast(), &target, tx.clone()) {
                let _ = tx.send(Err(error));
            }
        })
        .map_err(|e| e.to_string())?;
    // Started on the main thread and waited for here: the completion handler needs the event loop too,
    // so nothing on the main thread may wait for it.
    let (png, width, height) = rx.recv_timeout(TIMEOUT).map_err(|_| {
        format!(
            "No capture of {} within {} s",
            path.display(),
            TIMEOUT.as_secs()
        )
    })??;
    fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))?;
    log::info!("capture: wrote {} ({width}x{height} px)", path.display());
    Ok(())
}

// Runs on the main thread, which owns the web view.
fn start(
    view: *mut WKWebView,
    path: &Path,
    done: mpsc::Sender<Result<Png, String>>,
) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("the snapshot has to start on the main thread")?;
    // SAFETY: `with_webview` hands out the live web view and only on the main thread.
    let view = unsafe { view.as_ref() }.ok_or("this window has no web view")?;
    let (points, scale) = (
        view.bounds().size,
        view.window().map_or(1.0, |w| w.backingScaleFactor()),
    );
    log::info!(
        "capture: {}: {}x{} pt at {scale}x, so {}x{} px",
        path.display(),
        points.width,
        points.height,
        points.width * scale,
        points.height * scale
    );
    let handler = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        let _ = done.send(png_of(image, error));
    });
    // SAFETY: a fresh configuration for the main thread, and WebKit copies the handler before it returns.
    let configuration = unsafe { WKSnapshotConfiguration::new(mtm) };
    // SAFETY: called on the main thread with a live view.
    unsafe {
        view.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &handler);
    }
    Ok(())
}

// WebKit hands over an image in memory; the file is written once it is complete, so nobody reads it half
// done.
fn png_of(image: *mut NSImage, error: *mut NSError) -> Result<Png, String> {
    unsafe {
        if let Some(error) = error.as_ref() {
            return Err(format!(
                "the snapshot failed: {}",
                error.localizedDescription()
            ));
        }
        let image = image.as_ref().ok_or("the snapshot came back empty")?;
        let tiff = image
            .TIFFRepresentation()
            .ok_or("the snapshot held no image data")?;
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)
            .ok_or("the snapshot's image data isn't a bitmap")?;
        let png = rep
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
            .ok_or("the snapshot couldn't be written as PNG")?;
        Ok((
            png.to_vec(),
            rep.pixelsWide() as u32,
            rep.pixelsHigh() as u32,
        ))
    }
}
