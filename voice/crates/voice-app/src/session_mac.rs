//! What this Mac's GUI session looks like from the app's side: whether it is the one on the console and
//! whether its screen is locked. Both come from `CGSessionCopyCurrentDictionary`.

use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString, CFType};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unlocked_session_on_the_console_counts() {
        assert!(!at_the_console_from(None, None));
        assert!(!at_the_console_from(Some(false), Some(false)));
        assert!(at_the_console_from(Some(true), None));
        assert!(at_the_console_from(Some(true), Some(false)));
        // A locked screen is nobody, however it was reached.
        assert!(!at_the_console_from(Some(true), Some(true)));
    }
}
