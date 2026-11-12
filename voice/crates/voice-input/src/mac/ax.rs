//! The Accessibility API: whether Voice may drive other apps, the focused
//! element, its role and its selected text.
//!
//! Reading another app's accessibility objects and posting keys into it both
//! need the same grant, Privacy & Security › Accessibility. `AXIsProcessTrusted`
//! reports it without prompting, so Voice can say what to switch on instead of
//! failing silently; a prompt at the wrong moment would also steal the focus
//! Voice is about to type into.
//!
//! The attribute names are the ones `AXAttributeConstants.h` declares, spelled
//! out because the objc2 crate only binds the functions, not the constants.

use std::ptr::NonNull;

use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement};
use objc2_core_foundation::{CFRetained, CFString, CFType};

use crate::InputError;

/// The element the keyboard focus is in, of any app.
const FOCUSED_ELEMENT: &str = "AXFocusedUIElement";
/// The role of an element: `AXTextField`, `AXTextArea`, `AXButton`, ...
const ROLE: &str = "AXRole";
/// The selected text of a text element.
const SELECTED_TEXT: &str = "AXSelectedText";

/// Whether macOS lets Voice post keys into other apps and read their
/// accessibility objects. Never prompts.
pub(crate) fn is_trusted() -> bool {
    // The call only reads this process's TCC grant.
    unsafe { AXIsProcessTrusted() }
}

/// The reason Voice can't type right now, if the Accessibility permission is
/// missing.
pub(crate) fn check() -> Result<(), InputError> {
    if is_trusted() {
        Ok(())
    } else {
        Err(InputError::Accessibility)
    }
}

/// One attribute of an element. `None` when the app has no such attribute or
/// Voice may not read it.
fn attribute(element: &AXUIElement, name: &str) -> Option<CFRetained<CFType>> {
    let name = CFString::from_str(name);
    let mut value: *const CFType = std::ptr::null();
    // SAFETY: the element and the attribute name are valid, and the copy rule
    // hands over a +1 reference that `CFRetained` takes care of. A null answer
    // means the app has no such attribute.
    let error = unsafe { element.copy_attribute_value(&name, NonNull::from(&mut value)) };
    if error != AXError::Success {
        return None;
    }
    let value = NonNull::new(value.cast_mut())?;
    // SAFETY: the copy rule gave Voice the +1 reference the pointer holds.
    Some(unsafe { CFRetained::from_raw(value) })
}

/// The text of an attribute that is a `CFString`.
fn text(value: &CFType) -> Option<String> {
    value.downcast_ref::<CFString>().map(ToString::to_string)
}

/// The system-wide element, which every app's focus hangs off.
fn system_wide() -> CFRetained<AXUIElement> {
    // SAFETY: it takes no arguments and always hands back a new element.
    unsafe { AXUIElement::new_system_wide() }
}

/// The element that has the keyboard focus, in whatever app is in front.
pub(crate) fn focused_element() -> Option<CFRetained<AXUIElement>> {
    let focused = attribute(&system_wide(), FOCUSED_ELEMENT)?;
    focused.downcast::<AXUIElement>().ok()
}

/// The role of the focused element, such as `AXTextField`. `None` when Voice
/// may not read it, or the app publishes no focus.
pub(crate) fn focused_role() -> Option<String> {
    let element = focused_element()?;
    let role = attribute(&element, ROLE)?;
    text(&role)
}

/// The focused element's selected text. `None` when nothing is selected or the
/// app doesn't publish `AXSelectedText`, which is what sends the caller to
/// Cmd+C.
pub(crate) fn selected_text() -> Option<String> {
    let element = focused_element()?;
    let value = attribute(&element, SELECTED_TEXT)?;
    text(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads this process's own grant. It changes nothing and prompts nobody:
    /// the test binary either is or isn't trusted on the machine it runs on.
    #[test]
    fn the_trust_check_answers_without_prompting() {
        assert_eq!(is_trusted(), is_trusted(), "the answer must be stable");
        assert_eq!(check().is_err(), !is_trusted());
    }

    #[test]
    fn the_missing_permission_says_what_to_switch_on() {
        let message = InputError::Accessibility.to_string();
        assert!(message.contains("Accessibility"), "{message}");
        assert!(message.contains("Privacy & Security"), "{message}");
    }

    /// These read the real focus of whatever app is in front, so they need a
    /// desktop nobody is using and an Accessibility grant for the test binary.
    /// Run them there with `cargo test -p voice-input -- --ignored`.
    mod desktop {
        use super::super::*;

        #[test]
        #[ignore = "needs a dedicated desktop with an Accessibility grant and focus in a text field"]
        fn reads_the_selected_text_of_the_focused_element() {
            assert!(is_trusted(), "grant Accessibility to the test binary");
            assert!(focused_element().is_some(), "focus a text field first");
            assert!(focused_role().is_some());
            // The selection itself is whatever the person left selected.
            let _ = selected_text();
        }
    }
}
