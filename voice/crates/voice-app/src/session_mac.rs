//! What this Mac's GUI session looks like from the app's side: whether it is the one on the console and
//! whether its screen is locked. Both come from `CGSessionCopyCurrentDictionary`.
//!
//! It also hears the moments when the session goes away while a hotkey may be held: a fast user switch,
//! the lock screen, the Mac sleeping and the screens sleeping. Voice reads its hotkeys by polling, and a
//! key-up that happens while the session is gone never reaches the poll. Each of those events therefore
//! releases a held hotkey, so push-to-talk can't stay on (Ownkey #11).

use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationName, NSString,
};

/// `kCGSessionOnConsoleKey` and `CGSSessionScreenIsLocked` of `CGSessionCopyCurrentDictionary`. A missing
/// dictionary means no GUI session at all, which reads as neither flag.
pub fn session_flags() -> (Option<bool>, Option<bool>) {
    let Some(dictionary) = objc2_core_graphics::CGSessionCopyCurrentDictionary() else {
        return (None, None);
    };
    // SAFETY: a session dictionary has string keys and CoreFoundation values, and the dictionary outlives
    // both reads below.
    let dictionary: CFRetained<CFDictionary<CFString, CFType>> =
        unsafe { CFRetained::cast_unchecked(dictionary) };
    let read = |name: &str| {
        let key = CFString::from_str(name);
        // SAFETY: as above; the value is borrowed for the length of the closure body.
        unsafe { dictionary.get_unchecked(&key) }
            .and_then(CFType::downcast_ref::<CFBoolean>)
            .map(CFBoolean::as_bool)
    };
    (
        read("kCGSSessionOnConsoleKey"),
        read("CGSSessionScreenIsLocked"),
    )
}

/// The rule both callers act on, with the two flags passed in: the session is on the console and its
/// screen isn't locked.
pub fn at_the_console_from(on_console: Option<bool>, screen_locked: Option<bool>) -> bool {
    on_console == Some(true) && screen_locked != Some(true)
}

/// Whether anyone is at this Mac's console looking at the app's windows: the GUI session on the console
/// with its screen unlocked. A switched-out session or a locked screen counts as nobody.
pub fn at_the_console() -> bool {
    let (on_console, screen_locked) = session_flags();
    at_the_console_from(on_console, screen_locked)
}

/// `com.apple.screenIsLocked`, which the system posts on the distributed notification center when the
/// screen locks (including for a fast user switch, where the session resigns too).
const SCREEN_LOCKED: &str = "com.apple.screenIsLocked";

/// A moment at which the key-up of a held hotkey can be lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Fast user switching, logout or the screensaver with a password.
    SessionResigned,
    /// The screen locked (also posted for a switch).
    ScreenLocked,
    /// The Mac is about to sleep.
    WillSleep,
    /// The displays sleep while the Mac stays awake.
    ScreensSleep,
}

impl Release {
    /// Every event Voice observes; the tests hold this list against the observers.
    #[cfg(test)]
    pub const ALL: [Release; 4] = [
        Release::SessionResigned,
        Release::ScreenLocked,
        Release::WillSleep,
        Release::ScreensSleep,
    ];

    /// How the gap is named in the log.
    pub fn description(self) -> &'static str {
        match self {
            Release::SessionResigned => "the session resigned",
            Release::ScreenLocked => "the screen locked",
            Release::WillSleep => "the Mac is about to sleep",
            Release::ScreensSleep => "the screens slept",
        }
    }

    /// The workspace notification for an event, if it is one of the AppKit ones.
    #[cfg(test)]
    fn from_workspace(name: &str) -> Option<Release> {
        workspace_events()
            .into_iter()
            .find(|(event_name, _)| event_name.to_string() == name)
            .map(|(_, event)| event)
    }

    /// The distributed notification for an event, if it is one of those.
    #[cfg(test)]
    fn from_distributed(name: &str) -> Option<Release> {
        (name == SCREEN_LOCKED).then_some(Release::ScreenLocked)
    }
}

/// The AppKit notifications that mean a key-up may be lost, each with the event it is.
fn workspace_events() -> [(&'static NSNotificationName, Release); 3] {
    // SAFETY: AppKit's notification names are immutable strings that last as long as the
    // process.
    unsafe {
        [
            (
                NSWorkspaceSessionDidResignActiveNotification,
                Release::SessionResigned,
            ),
            (NSWorkspaceWillSleepNotification, Release::WillSleep),
            (
                NSWorkspaceScreensDidSleepNotification,
                Release::ScreensSleep,
            ),
        ]
    }
}

/// Calls `on_release` for every session event that can swallow a hotkey's key-up, until Voice quits.
/// Call it once, from the main thread: both notification centers deliver on the main run loop.
pub fn watch_releases(on_release: impl Fn(Release) + Send + Sync + 'static) {
    let on: Arc<dyn Fn(Release) + Send + Sync> = Arc::new(on_release);
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    for (name, event) in workspace_events() {
        let on = Arc::clone(&on);
        let block = RcBlock::new(move |_: NonNull<NSNotification>| dispatch(&on, event));
        // SAFETY: the block is `Send + Sync`, ignores the notification it is handed and only
        // starts a thread. The observer is leaked on purpose: AppKit keeps the block, and Voice
        // listens until it quits.
        let observer = unsafe {
            workspace.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(observer);
    }
    let distributed = NSDistributedNotificationCenter::defaultCenter();
    let name = NSString::from_str(SCREEN_LOCKED);
    let on_this = Arc::clone(&on);
    let block =
        RcBlock::new(move |_: NonNull<NSNotification>| dispatch(&on_this, Release::ScreenLocked));
    // SAFETY: as above. The distributed center's observer gets the same treatment.
    let observer = unsafe {
        distributed.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
    };
    std::mem::forget(observer);
}

/// Runs the handler off the notification's thread: stopping a dictation closes a microphone, and that
/// must not hold up AppKit or the poller.
fn dispatch(on: &Arc<dyn Fn(Release) + Send + Sync>, event: Release) {
    let on = Arc::clone(on);
    let _ = std::thread::Builder::new()
        .name("session-release".into())
        .spawn(move || on(event));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[test]
    fn only_an_unlocked_session_on_the_console_counts() {
        assert!(!at_the_console_from(None, None));
        assert!(!at_the_console_from(Some(false), Some(false)));
        assert!(at_the_console_from(Some(true), None));
        assert!(at_the_console_from(Some(true), Some(false)));
        // A locked screen is nobody, however it was reached.
        assert!(!at_the_console_from(Some(true), Some(true)));
    }

    /// Each of the four events is observed: three from the workspace and the lock from the
    /// distributed center.
    #[test]
    fn every_session_event_has_an_observer() {
        let mut seen = Vec::new();
        for (name, event) in workspace_events() {
            assert_eq!(
                Release::from_workspace(&name.to_string()),
                Some(event),
                "{event:?} is heard on the workspace center"
            );
            seen.push(event);
        }
        assert_eq!(
            Release::from_distributed(SCREEN_LOCKED),
            Some(Release::ScreenLocked)
        );
        seen.push(Release::ScreenLocked);
        seen.sort_by_key(|event| format!("{event:?}"));
        let mut all = Release::ALL.to_vec();
        all.sort_by_key(|event| format!("{event:?}"));
        assert_eq!(seen, all);
        assert_eq!(
            Release::from_workspace("NSWorkspaceDidWakeNotification"),
            None,
            "waking isn't a release: the monitor notification already was one"
        );
        assert_eq!(
            Release::from_distributed("com.apple.screenIsUnlocked"),
            None
        );
    }

    /// The handler runs for every event, off the notifying thread.
    #[test]
    fn every_event_reaches_the_release_handler() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let on: Arc<dyn Fn(Release) + Send + Sync> = {
            let seen = Arc::clone(&seen);
            Arc::new(move |event| seen.lock().unwrap().push(event))
        };
        for event in Release::ALL {
            dispatch(&on, event);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().len() < Release::ALL.len() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut seen = seen.lock().unwrap().clone();
        seen.sort_by_key(|event| format!("{event:?}"));
        let mut all = Release::ALL.to_vec();
        all.sort_by_key(|event| format!("{event:?}"));
        assert_eq!(seen, all);
    }
}
