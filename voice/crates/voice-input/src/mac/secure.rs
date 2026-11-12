//! Secure Event Input: macOS' counterpart of Windows' UIPI.
//!
//! While a password field or Terminal's Secure Keyboard Entry is active,
//! macOS delivers no synthetic keys to other apps and drops the ones already
//! posted. Reading the flag needs no grant.

use crate::InputError;

// Carbon's `Boolean` is an `unsigned char`, not C's `_Bool`.
#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

/// Whether a password field or Terminal's Secure Keyboard Entry turned
/// Secure Event Input on.
pub(crate) fn is_secure_event_input_enabled() -> bool {
    // The call only reads a process-wide flag, so it is safe from any thread.
    unsafe { IsSecureEventInputEnabled() != 0 }
}

/// The reason keys can't be posted right now, if any.
pub(crate) fn check() -> Result<(), InputError> {
    if is_secure_event_input_enabled() {
        Err(InputError::Blocked)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_secure_event_input_without_a_grant() {
        // Nothing on this desktop has a password field or Secure Keyboard
        // Entry open while the tests run.
        assert!(!is_secure_event_input_enabled());
        assert_eq!(check(), Ok(()));
        // The answer must agree with itself, not be a stale constant.
        assert_eq!(check().is_err(), is_secure_event_input_enabled());
    }
}
