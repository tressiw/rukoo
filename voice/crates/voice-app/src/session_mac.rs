//! What this Mac's GUI session looks like from the app's side: whether it is the one on the console and
//! whether its screen is locked. Both come from `CGSessionCopyCurrentDictionary`.
//!
//! It also hears the moments when the session goes away while a hotkey may be held: a fast user switch,
//! the lock screen, the Mac sleeping and the screens sleeping. Voice reads its hotkeys by polling, and a
//! key-up that happens while the session is gone never reaches the poll. Each of those events therefore
//! releases a held hotkey, so push-to-talk can't stay on (Ownkey #11).

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFNotificationCenter, CFNotificationName,
    CFNotificationSuspensionBehavior, CFRetained, CFString, CFType,
};
use objc2_foundation::{NSNotification, NSNotificationName};

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
    /// Every event Voice observes.
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

type OnRelease = Arc<dyn Fn(Release) + Send + Sync>;

/// Calls `on_release` for every session event that can swallow a hotkey's key-up, until Voice quits.
/// Call it once, from the main thread: both notification centers deliver on the main run loop.
pub fn watch_releases(on_release: impl Fn(Release) + Send + Sync + 'static) {
    let on: OnRelease = Arc::new(on_release);
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
    // The lock comes from another process. An app that isn't active, and Voice is an accessory
    // app that almost never is, gets distributed notifications held back unless the observer asks
    // for them at once; the block API can't, Core Foundation's can.
    let Some(distributed) = CFNotificationCenter::distributed_center() else {
        log::warn!("session: no distributed notification center; the lock isn't observed");
        return;
    };
    let name = CFString::from_str(SCREEN_LOCKED);
    // Leaked on purpose, like the observers above: the center keeps the pointer until Voice quits.
    let observer: *const OnRelease = Box::into_raw(Box::new(on));
    // SAFETY: the observer pointer stays valid for the life of the process, and the callback reads
    // it as the `OnRelease` it is.
    unsafe {
        distributed.add_observer(
            observer.cast::<c_void>(),
            Some(screen_locked),
            Some(&name),
            std::ptr::null(),
            CFNotificationSuspensionBehavior::DeliverImmediately,
        );
    }
}

/// Core Foundation's callback for [`SCREEN_LOCKED`].
unsafe extern "C-unwind" fn screen_locked(
    _center: *mut CFNotificationCenter,
    observer: *mut c_void,
    _name: *const CFNotificationName,
    _object: *const c_void,
    _info: *const CFDictionary,
) {
    // SAFETY: `watch_releases` registered a leaked `OnRelease` as the observer.
    let on = unsafe { &*observer.cast::<OnRelease>() };
    dispatch(on, Release::ScreenLocked);
}

/// Runs the handler for an event. Stopping a dictation closes a microphone, which must not hold up
/// AppKit, so it runs on a thread of its own, or right here when no thread can start. The Mac
/// about to sleep is the exception: the handler runs inside the notification, which AppKit lets
/// hold sleep back for a moment, so the recording stops before the Mac sleeps rather than at wake,
/// hours later, inserting its text into whatever has focus then.
fn dispatch(on: &OnRelease, event: Release) {
    if event == Release::WillSleep {
        on(event);
        return;
    }
    let for_thread = Arc::clone(on);
    let spawned = std::thread::Builder::new()
        .name("session-release".into())
        .spawn(move || for_thread(event));
    if spawned.is_err() {
        on(event);
    }
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

    /// The three AppKit events reach the handler through the observers `watch_releases` adds.
    /// Posting on NSWorkspace's own notification center stays inside this process; nothing else
    /// on the Mac hears it. The lock comes from another process, so it can't be posted from here
    /// without every app hearing it; its callback is called directly below.
    #[test]
    fn watch_releases_hears_the_workspace_events() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        watch_releases({
            let seen = Arc::clone(&seen);
            move |event| seen.lock().unwrap().push(event)
        });
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        for (name, _) in workspace_events() {
            // SAFETY: a notification with no object and no user info; the observers ignore both.
            unsafe { center.postNotificationName_object(name, None) };
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut seen = seen.lock().unwrap().clone();
        seen.sort_by_key(|event| format!("{event:?}"));
        assert_eq!(
            seen,
            [
                Release::ScreensSleep,
                Release::SessionResigned,
                Release::WillSleep
            ]
        );
    }

    #[test]
    fn the_lock_callback_reaches_the_handler() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let on: OnRelease = {
            let seen = Arc::clone(&seen);
            Arc::new(move |event| seen.lock().unwrap().push(event))
        };
        let observer: *const OnRelease = &on;
        // SAFETY: the observer is an `OnRelease`, as `watch_releases` registers it; the other
        // arguments are unused.
        unsafe {
            screen_locked(
                std::ptr::null_mut(),
                observer.cast_mut().cast(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(*seen.lock().unwrap(), [Release::ScreenLocked]);
    }

    /// Sleep is handled before the notification returns, on its own thread; every other event on
    /// a thread of its own, so AppKit isn't held up.
    #[test]
    fn only_sleep_is_handled_inside_the_notification() {
        let threads = Arc::new(Mutex::new(Vec::new()));
        let on: OnRelease = {
            let threads = Arc::clone(&threads);
            Arc::new(move |event| {
                let name = std::thread::current().name().map(str::to_string);
                threads.lock().unwrap().push((event, name));
            })
        };
        let here = std::thread::current().name().map(str::to_string);
        dispatch(&on, Release::WillSleep);
        assert_eq!(
            *threads.lock().unwrap(),
            [(Release::WillSleep, here)],
            "ran before dispatch returned, on the notifying thread"
        );
        for event in Release::ALL {
            if event != Release::WillSleep {
                dispatch(&on, event);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while threads.lock().unwrap().len() < Release::ALL.len() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        let threads = threads.lock().unwrap();
        assert_eq!(threads.len(), Release::ALL.len());
        for (event, name) in threads.iter().skip(1) {
            assert_eq!(name.as_deref(), Some("session-release"), "{event:?}");
        }
    }
}
