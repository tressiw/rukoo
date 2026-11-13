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
//! Hover comes from a tracking area that is active while Voice isn't, which a plain window of an
//! inactive app wouldn't get. The page draws hover and pressed from `pill://pointer`
//! (`pointer.rs` decides what it hears).

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSPanel, NSTrackingArea, NSTrackingAreaOptions, NSView,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use voice_engine::UiOptions;

pub use super::pointer::HitEvent;
use super::pointer::PointerState;
use super::window;
use crate::pill::place::Frame;

pub type HitHandler = Box<dyn Fn(HitEvent)>;

pub struct PanelIvars {
    handler: HitHandler,
    pointer: RefCell<PointerState>,
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
        fn mouse_down(&self, _event: &NSEvent) {
            let events = self.ivars().pointer.borrow_mut().down();
            self.report(events);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let point = event.locationInWindow();
            let size = self.frame().size;
            let inside = point.x >= 0.0 && point.x < size.width && point.y >= 0.0 && point.y < size.height;
            let events = self.ivars().pointer.borrow_mut().up(inside);
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
    }
);

/// Owned by the main thread, like every window.
pub struct HitPanel {
    panel: Retained<ButtonPanel>,
}

impl HitPanel {
    /// A hidden, invisible button panel. `title` is what screen readers announce.
    pub fn create(title: &str, handler: HitHandler) -> Result<HitPanel, String> {
        let mtm =
            MainThreadMarker::new().ok_or("the button panel has to be made on the main thread")?;
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        let this = ButtonPanel::alloc(mtm).set_ivars(PanelIvars {
            handler,
            pointer: RefCell::new(PointerState::default()),
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
        // SAFETY: a fresh tracking area owned by the panel, which outlives the view it is added to.
        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                frame,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect
                    | NSTrackingAreaOptions::EnabledDuringMouseDrag,
                Some(&panel),
                None,
            )
        };
        view.addTrackingArea(&tracking);
        panel.setContentView(Some(&view));

        // Not `setFloatingPanel(true)`: that moves the panel to the floating level, below the pill.
        panel.setLevel(window::LEVEL);
        panel.setCollectionBehavior(window::collection_behavior());
        panel.setBecomesKeyOnlyIfNeeded(true);
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
        Ok(HitPanel { panel })
    }

    pub fn set_title(&self, title: &str) {
        self.panel.setTitle(&NSString::from_str(title));
    }

    /// Over the button and above the pill: called right after every raise of the pill, so it sits
    /// directly above it at the same level.
    pub fn show_at(&self, frame: Frame) {
        if frame.w <= 0.0 || frame.h <= 0.0 {
            return;
        }
        let rect = NSRect::new(
            NSPoint::new(frame.x, frame.y),
            NSSize::new(frame.w, frame.h),
        );
        self.panel.setFrame_display(rect, false);
        self.panel.setLevel(window::LEVEL);
        self.panel.orderFrontRegardless();
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
