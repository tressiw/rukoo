//! Puts Rukoo Voice in System Settings' Screen & System Audio Recording list. macOS lists an app
//! there only once it asked for the permission, and the screen helper never asks (it would prompt
//! in the helper's name, mid-meeting). So the app asks, with `CGRequestScreenCaptureAccess`, at
//! the moments the user wants the screen recorded: a meeting with the screen on started without
//! the grant, or the user pressed Open for Screen Recording in Settings. It asks at most once per
//! app run and never at startup. macOS prompts once and adds the entry switched off; after that
//! the call only answers.

use std::sync::atomic::{AtomicBool, Ordering};

use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};

static ASKED: AtomicBool = AtomicBool::new(false);

/// Asks macOS for Screen Recording unless this process has it or asked already in this run. The
/// call returns at once and the system shows its own prompt, so it runs on a thread of its own:
/// neither the main thread nor a command waits for the user.
pub fn request() {
    if CGPreflightScreenCaptureAccess() {
        return;
    }
    if ASKED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("screen-permission".into())
        .spawn(|| {
            let granted = CGRequestScreenCaptureAccess();
            log::info!("screen recording: asked macOS for the permission; granted now: {granted}");
        });
}
