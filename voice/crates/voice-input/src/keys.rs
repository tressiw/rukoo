//! The hotkeys Voice offers, named as in Ownkey's config.

use std::fmt;
use std::str::FromStr;

/// A key that can start dictation or a rewrite.
///
/// The names are Ownkey's config strings (`"right alt"`, `"f13"`, ...), so an
/// imported Ownkey config keeps its keys. Only the left/right-specific keys
/// tell the right-hand modifiers apart from the left ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// Also AltGr on international layouts, and Right Option on a Mac.
    RightAlt,
    /// A Mac keyboard's right Command key; not offered on Windows.
    RightCommand,
    RightCtrl,
    RightShift,
    F13,
    F14,
    F15,
    Pause,
    ScrollLock,
}

impl Key {
    /// Every key Voice knows, on every platform. Poll all of them: which mode
    /// a key means is decided per event, so a Settings change never needs a
    /// new poller. A config copied from another platform keeps working too.
    pub const ALL: [Key; 9] = [
        Key::RightAlt,
        Key::RightCommand,
        Key::RightCtrl,
        Key::RightShift,
        Key::F13,
        Key::F14,
        Key::F15,
        Key::Pause,
        Key::ScrollLock,
    ];

    /// The keys a Windows keyboard has, in Ownkey's order.
    pub const WINDOWS: [Key; 8] = [
        Key::RightAlt,
        Key::RightCtrl,
        Key::RightShift,
        Key::F13,
        Key::F14,
        Key::F15,
        Key::Pause,
        Key::ScrollLock,
    ];

    /// The keys a Mac keyboard has. Pause and Scroll Lock don't exist there;
    /// Right Command is the one Mac key Voice adds.
    pub const MACOS: [Key; 7] = [
        Key::RightAlt,
        Key::RightCommand,
        Key::RightCtrl,
        Key::RightShift,
        Key::F13,
        Key::F14,
        Key::F15,
    ];

    /// The keys this platform can offer, so a key a keyboard lacks can't be
    /// chosen. `ALL` still parses every name, for a config from elsewhere.
    pub fn offered() -> &'static [Key] {
        if cfg!(target_os = "macos") {
            &Key::MACOS
        } else {
            &Key::WINDOWS
        }
    }

    /// The config and display name, for example `"right alt"`.
    pub fn name(self) -> &'static str {
        match self {
            Key::RightAlt => "right alt",
            Key::RightCommand => "right command",
            Key::RightCtrl => "right ctrl",
            Key::RightShift => "right shift",
            Key::F13 => "f13",
            Key::F14 => "f14",
            Key::F15 => "f15",
            Key::Pause => "pause",
            Key::ScrollLock => "scroll lock",
        }
    }

    /// Parses a config name. Surrounding whitespace and case are ignored, as in
    /// Ownkey's `sanitize_hotkey`. Falling back to a default is the caller's job.
    pub fn parse(name: &str) -> Option<Key> {
        let name = name.trim().to_lowercase();
        Key::ALL.into_iter().find(|key| key.name() == name)
    }

    /// The Windows virtual-key code, where Windows has the key.
    pub fn vk(self) -> Option<u16> {
        match self {
            Key::RightAlt => Some(0xA5),   // VK_RMENU
            Key::RightCtrl => Some(0xA3),  // VK_RCONTROL
            Key::RightShift => Some(0xA1), // VK_RSHIFT
            Key::F13 => Some(0x7C),
            Key::F14 => Some(0x7D),
            Key::F15 => Some(0x7E),
            Key::Pause => Some(0x13),
            Key::ScrollLock => Some(0x91),
            Key::RightCommand => None,
        }
    }

    /// The macOS virtual-key code, where a Mac keyboard has the key. Right
    /// Option is the same physical key as Right Alt, so it shares its code.
    pub fn mac_code(self) -> Option<u16> {
        match self {
            Key::RightAlt => Some(0x3D),     // kVK_RightOption
            Key::RightCommand => Some(0x36), // kVK_RightCommand
            Key::RightCtrl => Some(0x3E),    // kVK_RightControl
            Key::RightShift => Some(0x3C),   // kVK_RightShift
            Key::F13 => Some(0x69),
            Key::F14 => Some(0x6B),
            Key::F15 => Some(0x71),
            Key::Pause | Key::ScrollLock => None,
        }
    }

    /// Whether the selection can be copied while this key is held.
    ///
    /// Ownkey captures the selection at press for these keys, because racing
    /// the capture against the release is far less reliable. Holding Right Alt
    /// or Right Shift while Ctrl+C is sent would make it Ctrl+Alt+C or
    /// Ctrl+Shift+C, so those wait for the release.
    pub fn can_copy_while_held(self) -> bool {
        !matches!(self, Key::RightAlt | Key::RightShift)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The name didn't match any offered key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownKey(pub String);

impl fmt::Display for UnknownKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown hotkey {:?}", self.0)
    }
}

impl std::error::Error for UnknownKey {}

impl FromStr for Key {
    type Err = UnknownKey;

    fn from_str(name: &str) -> Result<Key, UnknownKey> {
        Key::parse(name).ok_or_else(|| UnknownKey(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ownkey's HOTKEY_VKS (windows_hotkeys.py:12-21), which a test there keeps
    // equal to HOTKEY_LIST.
    const OWNKEY: [(&str, u16); 8] = [
        ("right alt", 0xA5),
        ("right ctrl", 0xA3),
        ("right shift", 0xA1),
        ("f13", 0x7C),
        ("f14", 0x7D),
        ("f15", 0x7E),
        ("pause", 0x13),
        ("scroll lock", 0x91),
    ];

    #[test]
    fn windows_offers_exactly_ownkeys_keys_and_codes() {
        let ours: Vec<(&str, u16)> = Key::WINDOWS
            .iter()
            .map(|k| (k.name(), k.vk().expect("a Windows key has a virtual key")))
            .collect();
        assert_eq!(ours, OWNKEY);
        assert_eq!(Key::RightCommand.vk(), None);
    }

    #[test]
    fn macos_offers_the_keys_a_mac_keyboard_has() {
        let names: Vec<&str> = Key::MACOS.iter().map(|k| k.name()).collect();
        assert_eq!(
            names,
            [
                "right alt",
                "right command",
                "right ctrl",
                "right shift",
                "f13",
                "f14",
                "f15"
            ]
        );
        for key in Key::MACOS {
            assert!(key.mac_code().is_some(), "{key} has a Mac key code");
        }
        // Pause and Scroll Lock aren't on a Mac keyboard.
        assert_eq!(Key::Pause.mac_code(), None);
        assert_eq!(Key::ScrollLock.mac_code(), None);
    }

    #[test]
    fn the_mac_key_codes_match_the_keyboard() {
        let codes: Vec<(&str, u16)> = Key::MACOS
            .iter()
            .map(|k| (k.name(), k.mac_code().expect("a Mac key has a code")))
            .collect();
        assert_eq!(
            codes,
            [
                ("right alt", 0x3D),
                ("right command", 0x36),
                ("right ctrl", 0x3E),
                ("right shift", 0x3C),
                ("f13", 0x69),
                ("f14", 0x6B),
                ("f15", 0x71)
            ]
        );
    }

    #[test]
    fn names_round_trip() {
        for key in Key::ALL {
            assert_eq!(Key::parse(key.name()), Some(key));
            assert_eq!(key.to_string().parse::<Key>(), Ok(key));
        }
    }

    #[test]
    fn parse_ignores_case_and_surrounding_space() {
        assert_eq!(Key::parse("  Right Alt "), Some(Key::RightAlt));
        assert_eq!(Key::parse("RIGHT COMMAND"), Some(Key::RightCommand));
        assert_eq!(Key::parse("SCROLL LOCK"), Some(Key::ScrollLock));
    }

    #[test]
    fn parse_rejects_unknown_names() {
        assert_eq!(Key::parse(""), None);
        assert_eq!(Key::parse("off"), None);
        assert_eq!(Key::parse("rightalt"), None);
        assert_eq!(Key::parse("f16"), None);
        assert_eq!(
            "left alt".parse::<Key>(),
            Err(UnknownKey("left alt".into()))
        );
    }

    #[test]
    fn copy_while_held_matches_ownkeys_press_capture_keys() {
        // REWRITE_PRESS_CAPTURE_HOTKEYS, ownkey.py:195-199, plus Right Command.
        let press_capture: Vec<&str> = Key::ALL
            .into_iter()
            .filter(|k| k.can_copy_while_held())
            .map(Key::name)
            .collect();
        assert_eq!(
            press_capture,
            [
                "right command",
                "right ctrl",
                "f13",
                "f14",
                "f15",
                "pause",
                "scroll lock"
            ]
        );
    }
}
