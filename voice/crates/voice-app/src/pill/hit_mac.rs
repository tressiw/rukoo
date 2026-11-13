//! macOS: the stop and screen buttons' input, each a small native panel over the button the page draws,
//! as `hit.rs` does with layered windows on Windows (pill-settings-design.md 3.7).
//!
//! The pill window lets every click through, so WKWebView never gets pointer input and has nothing to
//! take focus with. A panel is borderless and fully transparent; it still takes clicks because
//! `ignoresMouseEvents` is set to false explicitly (left at its default, AppKit lets clicks through
//! the transparent pixels). It is an `NSPanel` with `NSWindowStyleMaskNonactivatingPanel` that answers
//! no to `canBecomeKeyWindow` and `canBecomeMainWindow`, so a click never makes Voice the active app
//! and never moves the keyboard focus out of the user's app (spike S4). Its content view accepts the
//! first mouse, so the click that reaches a window of an inactive app is a click and not only an
//! activation attempt.
//!
//! Each button panel reaches a few points past its drawn button (`place::hit_areas`), so a near miss
//! still clicks instead of landing on the wallpaper, where macOS's "click wallpaper to reveal desktop"
//! moves every window aside. Hover comes from a tracking area over the drawn button alone, active
//! while Voice isn't, so only the button itself looks hovered or pressed. The page draws those looks
//! from `pill://pointer` (`pointer.rs` decides what it hears).
//!
//! A third, inert panel of the same kind lies under the two, over the drawn pill and a margin around
//! it: a click on the pill's text or edge is swallowed there, never reaching the app or the wallpaper
//! behind, and does nothing else. It is as non-activating as the button panels.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{
    define_class, msg_send, AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSAccessibility, NSAccessibilityButtonRole, NSBackingStoreType, NSColor, NSEvent,
    NSEventModifierFlags, NSEventType, NSPanel, NSTrackingArea, NSTrackingAreaOptions, NSView,
    NSWindowStyleMask,
};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use voice_engine::UiOptions;

pub use super::pointer::HitEvent;
use super::pointer::PointerState;
use super::window;
use crate::pill::place::Frame;

pub type HitHandler = Box<dyn Fn(HitEvent)>;

pub struct PanelIvars {
    handler: HitHandler,
    pointer: RefCell<PointerState>,
    /// The drawn button in the panel's own coordinates; `None` for the inert body panel.
    button: Cell<Option<NSRect>>,
    /// The body panel: takes clicks and does nothing with them.
    inert: bool,
}

/// Half-open, as AppKit hit-tests.
fn contains(r: NSRect, p: NSPoint) -> bool {
    p.x >= r.origin.x
        && p.x < r.origin.x + r.size.width
        && p.y >= r.origin.y
        && p.y < r.origin.y + r.size.height
}

/// Where a panel goes, in AppKit screen points: the area it takes clicks in and the drawn button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelFrame {
    pub hit: Frame,
    pub button: Option<Frame>,
}

define_class!(
    // SAFETY: NSPanel has no subclassing requirements, and this type implements no Drop.
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "RukooVoicePillButtonPanel"]
    #[ivars = PanelIvars]
    struct ButtonPanel;

    impl ButtonPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if self.ivars().inert {
                return;
            }
            let point = event.locationInWindow();
            let on_button = self.ivars().button.get().is_some_and(|b| contains(b, point));
            let events = self.ivars().pointer.borrow_mut().down(on_button);
            self.report(events);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if self.ivars().inert {
                return;
            }
            let size = self.frame().size;
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), size);
            let on_panel = contains(bounds, event.locationInWindow());
            let events = self.ivars().pointer.borrow_mut().up(on_panel);
            self.report(events);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            let events = self.ivars().pointer.borrow_mut().entered();
            self.report(events);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            let events = self.ivars().pointer.borrow_mut().exited();
            self.report(events);
        }
    }
);

impl ButtonPanel {
    /// Calls the handler after the pointer state's borrow ended, so a handler that leads back here
    /// can't find it borrowed.
    fn report(&self, events: Vec<HitEvent>) {
        for event in events {
            (self.ivars().handler)(event);
        }
    }
}

define_class!(
    // SAFETY: NSView has no subclassing requirements, and this type implements no Drop.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "RukooVoicePillButtonView"]
    struct ButtonView;

    impl ButtonView {
        /// The first click on a window of an inactive app is a click.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// VoiceOver's press is a click on the button.
        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> Bool {
            let Some(window) = self.window() else {
                return Bool::NO;
            };
            if !window.isKindOfClass(ButtonPanel::class()) {
                return Bool::NO;
            }
            // SAFETY: checked just above that the window is a `ButtonPanel`.
            let panel: Retained<ButtonPanel> = unsafe { Retained::cast_unchecked(window) };
            panel.report(vec![HitEvent::Click]);
            Bool::YES
        }
    }
);

/// Owned by the main thread, like every window.
pub struct HitPanel {
    panel: Retained<ButtonPanel>,
    view: Retained<ButtonView>,
    /// The tracking area over the drawn button, replaced when the button moves.
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}

impl HitPanel {
    /// A hidden, invisible button panel. `title` is what screen readers announce.
    pub fn create(title: &str, handler: HitHandler) -> Result<HitPanel, String> {
        Self::build(title, handler, false)
    }

    /// The hidden body panel, which swallows clicks on the pill around its buttons.
    pub fn create_inert() -> Result<HitPanel, String> {
        Self::build("", Box::new(|_| {}), true)
    }

    fn build(title: &str, handler: HitHandler, inert: bool) -> Result<HitPanel, String> {
        let mtm =
            MainThreadMarker::new().ok_or("the button panel has to be made on the main thread")?;
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        let this = ButtonPanel::alloc(mtm).set_ivars(PanelIvars {
            handler,
            pointer: RefCell::new(PointerState::default()),
            button: Cell::new(None),
            inert,
        });
        let mask = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        // SAFETY: NSPanel's designated initializer, on a freshly allocated instance of a subclass that
        // adds only Rust ivars.
        let panel: Retained<ButtonPanel> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: frame,
                styleMask: mask,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        // SAFETY: NSView's designated initializer on a fresh instance of a subclass without ivars.
        let view: Retained<ButtonView> =
            unsafe { msg_send![ButtonView::alloc(mtm), initWithFrame: frame] };
        // VoiceOver announces a button with the stop or screen label, and can press it. The body
        // panel isn't anything to VoiceOver.
        view.setAccessibilityElement(!inert);
        if !inert {
            // SAFETY: AppKit's role constants are immutable strings that last as long as the process.
            view.setAccessibilityRole(Some(unsafe { NSAccessibilityButtonRole }));
            view.setAccessibilityLabel(Some(&NSString::from_str(title)));
        }
        panel.setContentView(Some(&view));

        // Not `setFloatingPanel(true)`: that moves the panel to the floating level, below the pill.
        panel.setLevel(window::LEVEL);
        panel.setCollectionBehavior(window::collection_behavior());
        panel.setBecomesKeyOnlyIfNeeded(true);
        // Settings' folder picker runs modally; the stop button still has to stop the meeting.
        panel.setWorksWhenModal(true);
        panel.setHidesOnDeactivate(false);
        // Hide in Settings or Hide Others elsewhere leaves the buttons with the pill.
        panel.setCanHide(false);
        panel.setOpaque(false);
        panel.setHasShadow(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setIgnoresMouseEvents(false);
        panel.setTitle(&NSString::from_str(title));
        // SAFETY: nobody else holds a claim on this window's release; Rust owns it.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.orderOut(None);
        Ok(HitPanel {
            panel,
            view,
            tracking: RefCell::new(None),
        })
    }

    pub fn set_title(&self, title: &str) {
        let title = NSString::from_str(title);
        self.panel.setTitle(&title);
        self.view.setAccessibilityLabel(Some(&title));
    }

    /// Over the button and above the pill: called right after every raise of the pill, so it sits
    /// directly above it at the same level.
    pub fn show_at(&self, place: PanelFrame) {
        let hit = place.hit;
        if hit.w <= 0.0 || hit.h <= 0.0 {
            return;
        }
        let rect = NSRect::new(NSPoint::new(hit.x, hit.y), NSSize::new(hit.w, hit.h));
        self.panel.setFrame_display(rect, false);
        let button = place.button.map(|b| {
            NSRect::new(
                NSPoint::new(b.x - hit.x, b.y - hit.y),
                NSSize::new(b.w, b.h),
            )
        });
        if !self.panel.ivars().inert && self.panel.ivars().button.replace(button) != button {
            self.track(button);
        }
        self.panel.setLevel(window::LEVEL);
        self.panel.orderFrontRegardless();
    }

    /// Hover over the drawn button only: a tracking area of that size, active while Voice isn't.
    fn track(&self, button: Option<NSRect>) {
        if let Some(old) = self.tracking.borrow_mut().take() {
            self.view.removeTrackingArea(&old);
        }
        let Some(button) = button else { return };
        // SAFETY: a fresh tracking area whose owner is the panel; AppKit holds the owner weakly, and
        // the panel outlives the view it is added to.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                button,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::EnabledDuringMouseDrag,
                Some(&self.panel),
                None,
            )
        };
        self.view.addTrackingArea(&area);
        *self.tracking.borrow_mut() = Some(area);
    }

    pub fn hide(&self) {
        if self.panel.isVisible() {
            self.panel.orderOut(None);
        }
        // A hidden panel gets no `mouseExited:`; the next show starts plain.
        self.panel.ivars().pointer.borrow_mut().reset();
    }

    pub fn is_visible(&self) -> bool {
        self.panel.isVisible()
    }

    /// For the self-test: a press in the panel's middle, released there or just right of it, made as
    /// `NSEvent`s and given to the panel's `sendEvent:`, the way AppKit dispatches a real click: through
    /// the content view (first mouse) to the panel's own `mouseDown:` and `mouseUp:`, with whatever
    /// AppKit does about key windows and activation on the way. Nothing is posted to the window server,
    /// so no other app sees it and the pointer doesn't move. Main thread only.
    pub fn click_for_test(&self, release_inside: bool) -> Result<(), String> {
        let size = self.panel.frame().size;
        let middle = NSPoint::new(size.width / 2.0, size.height / 2.0);
        let off = NSPoint::new(size.width + 10.0, size.height / 2.0);
        let event = |kind, point| {
            NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                point,
                NSEventModifierFlags::empty(),
                0.0,
                self.panel.windowNumber(),
                None,
                0,
                1,
                1.0,
            )
            .ok_or("AppKit made no mouse event")
        };
        let down = event(NSEventType::LeftMouseDown, middle)?;
        let up = event(
            NSEventType::LeftMouseUp,
            if release_inside { middle } else { off },
        )?;
        self.panel.sendEvent(&down);
        self.panel.sendEvent(&up);
        Ok(())
    }

    /// "Hide pill from capture": the panel draws nothing, but it is a window over the pill and follows
    /// it.
    pub fn set_capture_excluded(&self, excluded: bool) {
        self.panel.setSharingType(window::sharing_type(excluded));
    }

    /// The pill's theme, so anything AppKit draws for the panel matches the page under it.
    pub fn set_appearance(&self, ui: &UiOptions) {
        crate::appearance::apply_to_ns_window(&self.panel, ui);
    }

    pub fn recipe(&self) -> window::Recipe {
        window::recipe_of(&self.panel)
    }

    pub fn styles(&self) -> window::Styles {
        window::styles_of(&self.panel)
    }
}

impl Drop for HitPanel {
    fn drop(&mut self) {
        self.panel.orderOut(None);
        self.panel.close();
    }
}
