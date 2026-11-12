//! Whether the focused window looks like it takes text, as Ownkey judged it
//! (`is_text_input_selected`, `ownkey.py:631-658`).

/// Ownkey's text-target status. Only [`TextTarget::NotSelected`] has an
/// effect there: the pill says "Select a text box" and a notification warns
/// before inserting, but the text is inserted anyway, because many apps' text
/// fields can't be detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextTarget {
    /// A caret is showing or a known edit control has focus.
    Selected,
    /// The taskbar, desktop or Start menu has focus.
    NotSelected,
    /// Anything else, or Windows wouldn't say.
    Unknown,
}

impl TextTarget {
    /// The pill's value: `"selected"`, `"not_selected"` or `"unknown"`.
    pub fn as_str(self) -> &'static str {
        match self {
            TextTarget::Selected => "selected",
            TextTarget::NotSelected => "not_selected",
            TextTarget::Unknown => "unknown",
        }
    }
}

const TEXT_INPUT_CLASSES: [&str; 4] = ["Edit", "RichEdit20W", "RICHEDIT50W", "Scintilla"];
/// The taskbar, the desktop (two classes) and the Start menu.
const NON_INPUT_CLASSES: [&str; 4] = ["Shell_TrayWnd", "Progman", "WorkerW", "DV2ControlHost"];

/// Classifies the focus: `focus_class` is the class of the focused control
/// (or of the foreground window when nothing inside it has focus), `None`
/// when the focus couldn't be read.
pub fn classify(focus_class: Option<&str>, has_caret: bool) -> TextTarget {
    let Some(class) = focus_class else {
        return TextTarget::Unknown;
    };
    if has_caret || TEXT_INPUT_CLASSES.contains(&class) {
        TextTarget::Selected
    } else if NON_INPUT_CLASSES.contains(&class) {
        TextTarget::NotSelected
    } else {
        TextTarget::Unknown
    }
}

/// The AX roles of the elements that take text, as `AXUIElementCopyAttributeValue`
/// names them.
const MAC_TEXT_ROLES: [&str; 4] = ["AXTextField", "AXTextArea", "AXSearchField", "AXComboBox"];
/// Finder's desktop and the Dock have no text box of their own. Finder's search
/// field and rename field are text roles and win above this list.
const MAC_NON_TEXT_BUNDLES: [&str; 2] = ["com.apple.finder", "com.apple.dock"];

/// Classifies macOS' frontmost app and the AX role of its focused element.
/// Pure: the caller reads `NSWorkspace.frontmostApplication.bundleIdentifier`
/// and the role through the Accessibility API, which step 2 adds.
pub fn classify_mac(bundle_id: Option<&str>, ax_role: Option<&str>) -> TextTarget {
    if ax_role.is_some_and(|role| MAC_TEXT_ROLES.contains(&role)) {
        return TextTarget::Selected;
    }
    match bundle_id {
        Some(bundle) if MAC_NON_TEXT_BUNDLES.contains(&bundle) => TextTarget::NotSelected,
        _ => TextTarget::Unknown,
    }
}

/// The foreground window and its focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    /// The window handle as a number, for telling windows apart over time.
    pub hwnd: isize,
    pub process_id: u32,
    /// The foreground window's class.
    pub class_name: String,
    /// The focused control's class, or the foreground window's when nothing
    /// inside it has focus. `None` when Windows wouldn't say.
    pub focus_class: Option<String>,
    /// The focused thread shows a text caret.
    pub has_caret: bool,
    pub target: TextTarget,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caret_means_a_text_box() {
        assert_eq!(
            classify(Some("Chrome_RenderWidgetHostHWND"), true),
            TextTarget::Selected
        );
        assert_eq!(classify(Some("Shell_TrayWnd"), true), TextTarget::Selected);
    }

    #[test]
    fn known_edit_controls_are_text_boxes() {
        for class in TEXT_INPUT_CLASSES {
            assert_eq!(
                classify(Some(class), false),
                TextTarget::Selected,
                "{class}"
            );
        }
    }

    #[test]
    fn taskbar_desktop_and_start_menu_are_not() {
        for class in NON_INPUT_CLASSES {
            assert_eq!(
                classify(Some(class), false),
                TextTarget::NotSelected,
                "{class}"
            );
        }
    }

    #[test]
    fn anything_else_is_unknown() {
        assert_eq!(
            classify(Some("Chrome_WidgetWin_1"), false),
            TextTarget::Unknown
        );
        assert_eq!(
            classify(Some("edit"), false),
            TextTarget::Unknown,
            "class names are case-sensitive"
        );
        assert_eq!(
            classify(None, true),
            TextTarget::Unknown,
            "no focus information at all"
        );
    }

    #[test]
    fn mac_text_roles_are_text_targets() {
        for role in MAC_TEXT_ROLES {
            assert_eq!(
                classify_mac(Some("com.apple.TextEdit"), Some(role)),
                TextTarget::Selected,
                "{role}"
            );
            assert_eq!(
                classify_mac(Some("com.apple.finder"), Some(role)),
                TextTarget::Selected,
                "Finder's search field, {role}"
            );
        }
    }

    #[test]
    fn finders_desktop_and_the_dock_are_not() {
        for bundle in MAC_NON_TEXT_BUNDLES {
            for role in [None, Some("AXScrollArea"), Some("AXList"), Some("AXButton")] {
                assert_eq!(
                    classify_mac(Some(bundle), role),
                    TextTarget::NotSelected,
                    "{bundle} {role:?}"
                );
            }
        }
    }

    #[test]
    fn other_mac_apps_are_unknown_without_a_text_role() {
        assert_eq!(
            classify_mac(Some("com.apple.Safari"), Some("AXButton")),
            TextTarget::Unknown
        );
        assert_eq!(
            classify_mac(Some("com.apple.Safari"), None),
            TextTarget::Unknown
        );
        assert_eq!(classify_mac(None, None), TextTarget::Unknown);
        assert_eq!(
            classify_mac(None, Some("AXTextField")),
            TextTarget::Selected,
            "the role alone is enough"
        );
    }

    #[test]
    fn ax_roles_are_case_sensitive() {
        assert_eq!(
            classify_mac(Some("com.apple.TextEdit"), Some("axtextfield")),
            TextTarget::Unknown
        );
    }

    #[test]
    fn pill_values_match_ownkey() {
        assert_eq!(TextTarget::Selected.as_str(), "selected");
        assert_eq!(TextTarget::NotSelected.as_str(), "not_selected");
        assert_eq!(TextTarget::Unknown.as_str(), "unknown");
    }
}
