//! Keystroke sequences for the platform's key posting, built without touching
//! the OS so they can be tested anywhere.

pub(crate) const VK_RETURN: u16 = 0x0D;
pub(crate) const VK_SHIFT: u16 = 0x10;
pub(crate) const VK_CONTROL: u16 = 0x11;
pub(crate) const VK_C: u16 = 0x43;
pub(crate) const VK_V: u16 = 0x56;

/// The macOS key codes the chords use, from `HIToolbox/Events.h`'s `kVK_...`.
#[cfg(target_os = "macos")]
pub(crate) mod mac_key {
    pub(crate) const COMMAND: u16 = 0x37; // kVK_Command
    pub(crate) const SHIFT: u16 = 0x38; // kVK_Shift
    pub(crate) const OPTION: u16 = 0x3A; // kVK_Option
    pub(crate) const CONTROL: u16 = 0x3B; // kVK_Control
    pub(crate) const RETURN: u16 = 0x24; // kVK_Return
    pub(crate) const V: u16 = 0x09; // kVK_ANSI_V
    pub(crate) const C: u16 = 0x08; // kVK_ANSI_C
}

/// One keyboard event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stroke {
    /// A Windows virtual key going down or up.
    Key { vk: u16, up: bool },
    /// A macOS virtual key code (`kVK_...`) going down or up.
    #[cfg(target_os = "macos")]
    MacKey { code: u16, up: bool },
    /// One UTF-16 code unit sent with `KEYEVENTF_UNICODE`, independent of the
    /// keyboard layout.
    Unicode { unit: u16, up: bool },
}

fn down(vk: u16) -> Stroke {
    Stroke::Key { vk, up: false }
}

fn up(vk: u16) -> Stroke {
    Stroke::Key { vk, up: true }
}

/// Ctrl plus a key. Plain VK_CONTROL maps to the left Ctrl key, as Ownkey sent it.
fn ctrl_chord(vk: u16) -> Vec<Stroke> {
    vec![down(VK_CONTROL), down(vk), up(vk), up(VK_CONTROL)]
}

/// Cmd plus a key, with the Command key posted around it: macOS' chords carry the modifier as a
/// key of their own, unlike Windows' Ctrl, which `SendInput` tracks in the chord's flags.
#[cfg(target_os = "macos")]
fn cmd_chord(code: u16) -> Vec<Stroke> {
    use mac_key::COMMAND;
    vec![
        mac_down(COMMAND),
        mac_down(code),
        mac_up(code),
        mac_up(COMMAND),
    ]
}

#[cfg(target_os = "macos")]
fn mac_down(code: u16) -> Stroke {
    Stroke::MacKey { code, up: false }
}

#[cfg(target_os = "macos")]
fn mac_up(code: u16) -> Stroke {
    Stroke::MacKey { code, up: true }
}

pub(crate) fn paste() -> Vec<Stroke> {
    #[cfg(target_os = "macos")]
    {
        cmd_chord(mac_key::V)
    }
    #[cfg(not(target_os = "macos"))]
    {
        ctrl_chord(VK_V)
    }
}

pub(crate) fn copy() -> Vec<Stroke> {
    #[cfg(target_os = "macos")]
    {
        cmd_chord(mac_key::C)
    }
    #[cfg(not(target_os = "macos"))]
    {
        ctrl_chord(VK_C)
    }
}

/// Shift+Return on macOS, for a newline in typed text: the same intent as
/// Windows' Shift+Enter.
#[cfg(target_os = "macos")]
pub(crate) fn newline() -> Vec<Stroke> {
    vec![
        mac_down(mac_key::SHIFT),
        mac_down(mac_key::RETURN),
        mac_up(mac_key::RETURN),
        mac_up(mac_key::SHIFT),
    ]
}

/// The batches that type `text`, one per character, so the caller can pace
/// them (Ownkey waits 5 ms between characters).
///
/// A newline is Shift+Enter rather than Enter, so a chat app doesn't send the
/// message halfway through a dictation. This differs from Ownkey on purpose.
pub(crate) fn typing(text: &str) -> Vec<Vec<Stroke>> {
    let mut batches = Vec::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                batches.push(vec![
                    down(VK_SHIFT),
                    down(VK_RETURN),
                    up(VK_RETURN),
                    up(VK_SHIFT),
                ]);
            }
            // Other control characters aren't text a text box should receive.
            c if c.is_control() && c != '\t' => {}
            c => {
                let mut buf = [0u16; 2];
                let units = c.encode_utf16(&mut buf);
                // All downs, then all ups: a surrogate pair is two units of one character.
                let downs = units
                    .iter()
                    .map(|&unit| Stroke::Unicode { unit, up: false });
                let ups = units.iter().map(|&unit| Stroke::Unicode { unit, up: true });
                batches.push(downs.chain(ups).collect());
            }
        }
    }
    batches
}

/// The key-ups that undo a batch the platform inserted only partly, so a
/// cut-off Cmd+V can't leave Command held down.
pub(crate) fn release_after(inserted: &[Stroke]) -> Vec<Stroke> {
    let mut held: Vec<Stroke> = Vec::new();
    for stroke in inserted {
        match *stroke {
            Stroke::Key { vk, up: false } => held.push(Stroke::Key { vk, up: true }),
            #[cfg(target_os = "macos")]
            Stroke::MacKey { code, up: false } => held.push(Stroke::MacKey { code, up: true }),
            Stroke::Unicode { unit, up: false } => held.push(Stroke::Unicode { unit, up: true }),
            released => held.retain(|h| *h != released),
        }
    }
    held.reverse();
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(unit: u16, up: bool) -> Stroke {
        Stroke::Unicode { unit, up }
    }

    fn shift_enter() -> Vec<Stroke> {
        vec![down(VK_SHIFT), down(VK_RETURN), up(VK_RETURN), up(VK_SHIFT)]
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn paste_is_left_ctrl_v() {
        assert_eq!(
            paste(),
            [
                Stroke::Key {
                    vk: 0x11,
                    up: false
                },
                Stroke::Key {
                    vk: 0x56,
                    up: false
                },
                Stroke::Key { vk: 0x56, up: true },
                Stroke::Key { vk: 0x11, up: true },
            ]
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn copy_is_left_ctrl_c() {
        assert_eq!(copy(), [down(0x11), down(0x43), up(0x43), up(0x11)]);
    }

    /// macOS' chords post Command as a key of its own, with the V or C between its halves.
    #[test]
    #[cfg(target_os = "macos")]
    fn paste_and_copy_are_command_chords_with_macs_key_codes() {
        let cmd = mac_key::COMMAND;
        assert_eq!(
            paste(),
            [
                mac_down(cmd),
                mac_down(mac_key::V),
                mac_up(mac_key::V),
                mac_up(cmd),
            ]
        );
        assert_eq!(
            copy(),
            [
                mac_down(cmd),
                mac_down(mac_key::C),
                mac_up(mac_key::C),
                mac_up(cmd),
            ]
        );
        // The codes are the keyboard's own: kVK_Command, kVK_ANSI_V, kVK_ANSI_C.
        assert_eq!((cmd, mac_key::V, mac_key::C), (0x37, 0x09, 0x08));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_mac_newline_is_shift_return() {
        assert_eq!(
            newline(),
            [mac_down(0x38), mac_down(0x24), mac_up(0x24), mac_up(0x38),]
        );
    }

    #[test]
    fn types_each_character_as_a_unicode_down_and_up() {
        assert_eq!(
            typing("Hé"),
            [
                vec![u(0x48, false), u(0x48, true)],
                vec![u(0xE9, false), u(0xE9, true)]
            ]
        );
    }

    #[test]
    fn sends_a_surrogate_pair_as_two_downs_then_two_ups() {
        // U+1F600 is D83D DE00 in UTF-16.
        assert_eq!(
            typing("😀"),
            [vec![
                u(0xD83D, false),
                u(0xDE00, false),
                u(0xD83D, true),
                u(0xDE00, true)
            ]]
        );
    }

    #[test]
    fn a_newline_is_shift_enter() {
        let batches = typing("a\nb");
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[1], shift_enter());
    }

    #[test]
    fn crlf_and_a_lone_cr_are_one_newline_each() {
        assert_eq!(typing("\r\n"), [shift_enter()]);
        assert_eq!(typing("\r"), [shift_enter()]);
        assert_eq!(typing("\n\n"), [shift_enter(), shift_enter()]);
        assert_eq!(typing("\r\r\n"), [shift_enter(), shift_enter()]);
    }

    #[test]
    fn keeps_tabs_and_drops_other_control_characters() {
        assert_eq!(typing("\t"), [vec![u(0x09, false), u(0x09, true)]]);
        assert!(typing("\u{0}\u{8}\u{1b}\u{7f}").is_empty());
    }

    #[test]
    fn empty_text_types_nothing() {
        assert!(typing("").is_empty());
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn a_cut_off_chord_releases_what_went_down() {
        let chord = paste();
        assert!(release_after(&chord[..0]).is_empty());
        assert_eq!(release_after(&chord[..1]), [up(VK_CONTROL)]);
        assert_eq!(release_after(&chord[..2]), [up(VK_V), up(VK_CONTROL)]);
        assert_eq!(release_after(&chord[..3]), [up(VK_CONTROL)]);
        assert!(release_after(&chord).is_empty());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_cut_off_mac_chord_releases_what_went_down() {
        let chord = paste();
        assert_eq!(release_after(&chord[..1]), [mac_up(mac_key::COMMAND)]);
        assert_eq!(
            release_after(&chord[..2]),
            [mac_up(mac_key::V), mac_up(mac_key::COMMAND)]
        );
        assert_eq!(release_after(&chord[..3]), [mac_up(mac_key::COMMAND)]);
        assert!(release_after(&chord).is_empty());
    }

    #[test]
    fn a_cut_off_surrogate_pair_releases_its_units() {
        let pair = &typing("😀")[0];
        assert_eq!(
            release_after(&pair[..2]),
            [u(0xDE00, true), u(0xD83D, true)]
        );
        assert_eq!(release_after(&pair[..3]), [u(0xDE00, true)]);
    }
}
