//! Whether the macOS session is locked. The screen shows the lock screen then, so the screen
//! process stops capturing and writes `lock`, and on `unlock` it starts a new capture session.

use std::ptr::NonNull;

use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString};

extern "C" {
    /// The session's attributes, or null outside a GUI session. Retained, so it is released here.
    fn CGSessionCopyCurrentDictionary() -> *mut CFDictionary;
}

/// The key `CGSessionCopyCurrentDictionary` holds the lock flag under.
const SCREEN_LOCKED: &str = "CGSSessionScreenIsLocked";

/// Locked, meaning the screen shows the lock screen. Unlike Windows there is no second desktop
/// to tell apart: a locked session is the only state that hides the screen from this process.
pub fn locked() -> bool {
    // SAFETY: the call returns a retained dictionary or null; it never hands out a borrowed one.
    let dict = NonNull::new(unsafe { CGSessionCopyCurrentDictionary() });
    let Some(dict) = dict else {
        return false;
    };
    // SAFETY: the dictionary came back with a reference of its own, which this drops.
    let dict = unsafe { CFRetained::from_raw(dict) };
    // SAFETY: the session dictionary stores the flag, when it is there, as a CFBoolean.
    let dict: &CFDictionary<CFString, CFBoolean> =
        unsafe { &*(&*dict as *const CFDictionary as *const CFDictionary<CFString, CFBoolean>) };
    let key = CFString::from_static_str(SCREEN_LOCKED);
    dict.get(&key).is_some_and(|flag| flag.value())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests run in a GUI session, on a desktop somebody may or may not have locked. The call
    /// has to answer either way and to agree with itself.
    #[test]
    fn the_lock_flag_reads_the_session() {
        let first = locked();
        assert_eq!(first, locked());
    }
}
