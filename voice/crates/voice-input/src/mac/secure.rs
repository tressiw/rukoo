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
        // Whether a password field or Secure Keyboard Entry is open depends on whoever sits at this
        // Mac, so the test doesn't assert either state. It asserts that the reading works without a
        // grant and that `check()` agrees with it, on a reading that didn't change around the call.
        for _ in 0..50 {
            let before = is_secure_event_input_enabled();
            let checked = check();
            if before != is_secure_event_input_enabled() {
                // Somebody focused or left a password field between the two reads: read again.
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }
            if before {
                assert_eq!(checked, Err(InputError::Blocked));
            } else {
                assert_eq!(checked, Ok(()));
            }
            return;
        }
        panic!("Secure Event Input changed on each of 50 reads");
    }
}
