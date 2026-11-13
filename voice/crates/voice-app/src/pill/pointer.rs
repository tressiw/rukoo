//! What the pointer does on one of the pill's native button panels on macOS (`hit_mac.rs`), apart from
//! AppKit so the tests run everywhere. It ports `hit.rs`'s window procedure: the page draws hover and
//! pressed from `pill://pointer`, and one press and release inside the button is one click. The
//! two-step stop ("Stop?", then stop) is the engine's: each click here is one step of it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitEvent {
    /// For `pill://pointer`: the page colours the button.
    Pointer { hover: bool, pressed: bool },
    /// One press and release inside the button.
    Click,
}

/// One panel's pointer, fed from its tracking area and its mouse events.
#[derive(Debug, Default)]
pub struct PointerState {
    hover: bool,
    /// A press that started on this button and hasn't been released yet.
    pressed: bool,
    /// The look the page was last told, so it hears each change once.
    reported: (bool, bool),
}

impl PointerState {
    fn report(&mut self, out: &mut Vec<HitEvent>) {
        // A press dragged off the button stops looking pressed, and looks pressed again when it
        // comes back, as an AppKit button does.
        let look = (self.hover, self.pressed && self.hover);
        if self.reported != look {
            self.reported = look;
            out.push(HitEvent::Pointer {
                hover: look.0,
                pressed: look.1,
            });
        }
    }

    /// `mouseEntered:`, or a move inside the button.
    pub fn entered(&mut self) -> Vec<HitEvent> {
        let mut out = Vec::new();
        self.hover = true;
        self.report(&mut out);
        out
    }

    /// `mouseExited:`. A press stays armed: released back on the button, it still clicks.
    pub fn exited(&mut self) -> Vec<HitEvent> {
        let mut out = Vec::new();
        self.hover = false;
        self.report(&mut out);
        out
    }

    /// `mouseDown:`; AppKit only sends it for a press on the panel.
    pub fn down(&mut self) -> Vec<HitEvent> {
        let mut out = Vec::new();
        self.hover = true;
        self.pressed = true;
        self.report(&mut out);
        out
    }

    /// `mouseUp:`, with whether the release is on the button. AppKit sends it to the panel that had
    /// the press even when the pointer left it. A release without a press here (the panel hid and
    /// came back in between) changes nothing: the tracking area says where the pointer is.
    pub fn up(&mut self, inside: bool) -> Vec<HitEvent> {
        let mut out = Vec::new();
        if !std::mem::replace(&mut self.pressed, false) {
            return out;
        }
        self.hover = inside;
        if inside {
            out.push(HitEvent::Click);
        }
        self.report(&mut out);
        out
    }

    /// The panel hid: a press in progress is dropped and the next show starts plain. The page clears
    /// a hidden button's look itself, so nothing is reported.
    pub fn reset(&mut self) {
        *self = PointerState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn look(hover: bool, pressed: bool) -> HitEvent {
        HitEvent::Pointer { hover, pressed }
    }

    #[test]
    fn hover_press_and_release_tell_the_page_and_click_once() {
        let mut p = PointerState::default();
        assert_eq!(p.entered(), [look(true, false)]);
        assert_eq!(p.entered(), [], "a second enter changes nothing");
        assert_eq!(p.down(), [look(true, true)]);
        assert_eq!(p.up(true), [HitEvent::Click, look(true, false)]);
        assert_eq!(p.exited(), [look(false, false)]);
    }

    #[test]
    fn a_release_without_a_press_is_not_a_click() {
        let mut p = PointerState::default();
        p.entered();
        assert_eq!(p.up(true), []);
        assert_eq!(p.up(true), []);
    }

    #[test]
    fn a_press_dragged_off_and_released_outside_does_nothing() {
        let mut p = PointerState::default();
        p.entered();
        p.down();
        assert_eq!(p.exited(), [look(false, false)]);
        assert_eq!(p.up(false), []);
        // The next press works as usual.
        p.entered();
        p.down();
        assert_eq!(p.up(true), [HitEvent::Click, look(true, false)]);
    }

    #[test]
    fn a_press_dragged_off_and_back_still_clicks() {
        let mut p = PointerState::default();
        p.entered();
        p.down();
        p.exited();
        assert_eq!(p.entered(), [look(true, true)]);
        assert_eq!(p.up(true), [HitEvent::Click, look(true, false)]);
    }

    #[test]
    fn the_two_step_stop_is_two_clicks() {
        // "Stop?" then stop: the engine counts the steps, the panel only reports each click.
        let mut p = PointerState::default();
        p.entered();
        let clicks = (0..2)
            .flat_map(|_| {
                let mut events = p.down();
                events.extend(p.up(true));
                events
            })
            .filter(|e| *e == HitEvent::Click)
            .count();
        assert_eq!(clicks, 2);
    }

    #[test]
    fn hiding_drops_a_press_in_progress() {
        let mut p = PointerState::default();
        p.entered();
        p.down();
        p.reset();
        assert_eq!(p.up(true), [], "no click and no hover after the hide");
    }
}
