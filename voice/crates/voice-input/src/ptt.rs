//! Push-to-talk on top of hotkey events: a port of Ownkey's `_on_press`,
//! `_on_release` and `restart_listener` (`ownkey.py:2908-2980`).
//!
//! One debounce flag ("a hotkey is down") guards against starting a second
//! recording while one is held. While a recording runs, keys map to modes
//! through the hotkeys it started with, so releasing the key that started it
//! still stops it after a Settings change.

use crate::{HotkeyEvent, Key};

/// What a recording is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Transcribe and insert.
    Dictate,
    /// Rewrite the selected text with a spoken instruction.
    Rewrite,
}

/// The configured hotkeys. `rewrite: None` is Ownkey's `"off"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotkeyConfig {
    pub dictate: Key,
    pub rewrite: Option<Key>,
}

impl HotkeyConfig {
    /// The mode a key starts. The dictation key wins if both are the same.
    pub fn mode_for(&self, key: Key) -> Option<Mode> {
        if key == self.dictate {
            Some(Mode::Dictate)
        } else if self.rewrite == Some(key) {
            Some(Mode::Rewrite)
        } else {
            None
        }
    }
}

/// What the app should do with the microphone after an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start recording in this mode. Call [`PushToTalk::recording_ended`] if
    /// the microphone doesn't open.
    Start(Mode),
    /// Stop the recording that is running.
    Stop,
}

/// The push-to-talk state for one hotkey poller.
///
/// Feed it every [`HotkeyEvent`] in order. When it's shared with Settings,
/// keep it behind a mutex, but don't hold that lock while the microphone opens
/// or closes.
#[derive(Clone, Debug)]
pub struct PushToTalk {
    live: HotkeyConfig,
    /// The hotkeys a running recording started with.
    recording: Option<HotkeyConfig>,
    /// The mode of the last press that started a recording, kept after a failed start.
    mode: Mode,
    /// A hotkey is held.
    down: bool,
}

impl PushToTalk {
    pub fn new(config: HotkeyConfig) -> PushToTalk {
        PushToTalk {
            live: config,
            recording: None,
            mode: Mode::Dictate,
            down: false,
        }
    }

    pub fn handle(&mut self, event: HotkeyEvent) -> Option<Action> {
        match event {
            HotkeyEvent::Pressed(key) => self.press(key).map(Action::Start),
            HotkeyEvent::Released(key) => self.release(key).then_some(Action::Stop),
        }
    }

    fn current(&self) -> HotkeyConfig {
        self.recording.unwrap_or(self.live)
    }

    fn press(&mut self, key: Key) -> Option<Mode> {
        if self.down {
            return None;
        }
        let mode = self.current().mode_for(key)?;
        self.down = true;
        self.mode = mode;
        self.recording = Some(self.live);
        Some(mode)
    }

    fn release(&mut self, key: Key) -> bool {
        if !self.down || self.current().mode_for(key) != Some(self.mode) {
            return false;
        }
        self.down = false;
        // A start that failed left nothing to stop.
        self.recording.take().is_some()
    }

    /// The recording ended without a release: the microphone didn't open, or
    /// the app stopped it. The key stays debounced until it's released.
    pub fn recording_ended(&mut self) {
        self.recording = None;
    }

    /// Treats the held hotkey as released: its key-up may never arrive, for
    /// example when the Mac's session resigns, the screen locks or the Mac
    /// sleeps. A key-up that does arrive later is ignored. Returns whether a
    /// recording was running, which the caller then stops.
    pub fn release_all(&mut self) -> bool {
        self.down = false;
        self.recording.take().is_some()
    }

    /// Settings changed the hotkeys. The poller keeps running, because it
    /// polls every offered key and the new config applies from the next event.
    pub fn set_config(&mut self, config: HotkeyConfig) {
        self.live = config;
        // A hold that never started a recording may no longer match any
        // hotkey, so don't let it keep the debounce set.
        if self.recording.is_none() {
            self.down = false;
        }
    }

    pub fn config(&self) -> HotkeyConfig {
        self.live
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Whether a hotkey is held (the debounce flag).
    pub fn is_down(&self) -> bool {
        self.down
    }

    /// The mode of the running recording, or of the last one.
    pub fn mode(&self) -> Mode {
        self.mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotkeyEvent::{Pressed, Released};

    const OWNKEY_DEFAULTS: HotkeyConfig = HotkeyConfig {
        dictate: Key::RightAlt,
        rewrite: Some(Key::RightCtrl),
    };

    #[test]
    fn dictation_key_is_checked_first() {
        let same = HotkeyConfig {
            dictate: Key::F13,
            rewrite: Some(Key::F13),
        };
        assert_eq!(same.mode_for(Key::F13), Some(Mode::Dictate));
        assert_eq!(
            OWNKEY_DEFAULTS.mode_for(Key::RightCtrl),
            Some(Mode::Rewrite)
        );
        assert_eq!(OWNKEY_DEFAULTS.mode_for(Key::F13), None);
        let off = HotkeyConfig {
            dictate: Key::RightAlt,
            rewrite: None,
        };
        assert_eq!(off.mode_for(Key::RightCtrl), None);
    }

    #[test]
    fn one_hold_starts_and_stops_one_recording() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        assert_eq!(
            ptt.handle(Pressed(Key::RightAlt)),
            Some(Action::Start(Mode::Dictate))
        );
        assert!(ptt.is_down() && ptt.is_recording());
        assert_eq!(ptt.handle(Released(Key::RightAlt)), Some(Action::Stop));
        assert!(!ptt.is_down() && !ptt.is_recording());
    }

    #[test]
    fn unrelated_keys_do_nothing() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        assert_eq!(ptt.handle(Pressed(Key::F15)), None);
        assert_eq!(ptt.handle(Released(Key::F15)), None);
        assert!(!ptt.is_down());
    }

    #[test]
    fn the_other_hotkey_is_ignored_while_one_is_held() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        ptt.handle(Pressed(Key::RightAlt));
        assert_eq!(ptt.handle(Pressed(Key::RightCtrl)), None);
        assert_eq!(ptt.handle(Released(Key::RightCtrl)), None);
        assert!(ptt.is_recording());
        assert_eq!(ptt.mode(), Mode::Dictate);
        assert_eq!(ptt.handle(Released(Key::RightAlt)), Some(Action::Stop));
    }

    #[test]
    fn a_failed_start_keeps_the_key_debounced_until_its_release() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        ptt.handle(Pressed(Key::RightCtrl));
        ptt.recording_ended(); // the microphone didn't open
        assert!(ptt.is_down());
        assert_eq!(ptt.handle(Pressed(Key::RightAlt)), None);
        // Nothing to stop, but the release clears the debounce.
        assert_eq!(ptt.handle(Released(Key::RightCtrl)), None);
        assert!(!ptt.is_down());
        assert_eq!(
            ptt.handle(Pressed(Key::RightAlt)),
            Some(Action::Start(Mode::Dictate))
        );
    }

    #[test]
    fn a_running_recording_keeps_the_hotkeys_it_started_with() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        ptt.handle(Pressed(Key::RightAlt));
        ptt.set_config(HotkeyConfig {
            dictate: Key::F13,
            rewrite: Some(Key::F14),
        });
        assert!(ptt.is_down(), "a running recording keeps its debounce");
        assert_eq!(ptt.handle(Released(Key::RightAlt)), Some(Action::Stop));
        assert_eq!(ptt.handle(Pressed(Key::RightAlt)), None);
        assert_eq!(
            ptt.handle(Pressed(Key::F14)),
            Some(Action::Start(Mode::Rewrite))
        );
    }

    #[test]
    fn a_settings_change_clears_a_debounce_without_a_recording() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        ptt.handle(Pressed(Key::RightCtrl));
        ptt.recording_ended();
        ptt.set_config(HotkeyConfig {
            dictate: Key::RightAlt,
            rewrite: None,
        });
        assert!(!ptt.is_down());
        assert_eq!(ptt.handle(Released(Key::RightCtrl)), None);
        assert_eq!(
            ptt.handle(Pressed(Key::RightAlt)),
            Some(Action::Start(Mode::Dictate))
        );
    }

    /// A session event that can swallow a key-up (a fast user switch, the
    /// lock screen, sleep) releases the held key once and leaves nothing for
    /// the real release to stop a second time. Which event it was doesn't
    /// reach push-to-talk; the app's tests cover the observers.
    #[test]
    fn release_all_stops_a_held_recording_once() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        assert_eq!(
            ptt.handle(Pressed(Key::RightAlt)),
            Some(Action::Start(Mode::Dictate))
        );
        assert!(ptt.release_all(), "the recording is stopped");
        assert!(!ptt.is_down() && !ptt.is_recording());
        // The key-up arrives after the session came back: nothing runs twice.
        assert_eq!(ptt.handle(Released(Key::RightAlt)), None);
        assert!(!ptt.release_all(), "no second stop");
        assert_eq!(
            ptt.handle(Pressed(Key::RightAlt)),
            Some(Action::Start(Mode::Dictate)),
            "the next hold works"
        );
    }

    #[test]
    fn releasing_without_a_hold_does_nothing() {
        let mut ptt = PushToTalk::new(OWNKEY_DEFAULTS);
        assert!(!ptt.release_all());
        ptt.handle(Pressed(Key::RightAlt));
        ptt.recording_ended();
        assert!(!ptt.release_all(), "a failed start left nothing to stop");
        assert!(!ptt.is_down());
        assert_eq!(
            ptt.handle(Pressed(Key::RightAlt)),
            Some(Action::Start(Mode::Dictate))
        );
    }
}
