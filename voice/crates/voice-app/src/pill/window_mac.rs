//! macOS: the pill as a window above other apps, a full-screen app in its own Space included, that
//! never takes focus (spike S4). Everything here runs on the main thread, which owns the windows; a
//! call from another thread finds no `MainThreadMarker` and does nothing.
//!
//! tao's `set_visible(true)` and so tauri's `show` are `makeKeyAndOrderFront`, which would activate
//! Voice. This module shows the window with `orderFrontRegardless` instead and never calls `show`. The
//! window class stays tao's, which answers `canBecomeKeyWindow` and `canBecomeMainWindow` from the
//! builder's `.focusable(false)`, so the window can't take the keyboard even if something orders it in.
//!
//! The recipe, reapplied on every show and raise because tao rewrites some of it from its own flags:
//! - `NSStatusWindowLevel` (25), above normal and floating windows and below the menu bar's menus;
//! - `canJoinAllSpaces | fullScreenAuxiliary | stationary | ignoresCycle`, so the pill follows the user
//!   into a full-screen Space instead of pulling them out of it, stays put in Mission Control and is
//!   never part of Cmd+`;
//! - `ignoresMouseEvents`, so the web view never gets pointer input: the stop and screen buttons are
//!   panels of their own (`hit_mac.rs`), as they are separate windows on Windows;
//! - `hidesOnDeactivate = false`; Voice is an accessory app and never active anyway;
//! - `canHide = false`, so Hide (Cmd+H in Settings) and another app's Hide Others leave it on screen
//!   during a meeting.
//!
//! "Hide pill from capture" is `sharingType = .none`, which tauri's `set_content_protected` sets on
//! macOS (`.readOnly` when off); `recipe` reports what the window has.

use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSEvent, NSRunningApplication, NSScreen,
    NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowOcclusionState,
    NSWindowSharingType, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};
use objc2_foundation::{
    NSArray, NSDictionary, NSDistributedNotificationCenter, NSNotification, NSNotificationCenter,
    NSNumber, NSPoint, NSRect, NSSize, NSString,
};

use super::*;
use crate::pill::place::{self, Frame, Rect, Screen};

/// The level both the pill and its button panels use.
pub const LEVEL: isize = NSStatusWindowLevel;

thread_local! {
    /// The display the pill was last placed on, so a display change keeps it there while that display
    /// is still connected, wherever the rearrangement moved it.
    static LAST_DISPLAY: Cell<Option<u32>> = const { Cell::new(None) };
}

/// Posted on the distributed notification center when the screen unlocks.
const SCREEN_UNLOCKED: &str = "com.apple.screenIsUnlocked";

/// Whether another app coming forward raises the pill: on while it shows (`foreground_hook`).
static FOREGROUND: AtomicBool = AtomicBool::new(false);

/// `canJoinAllSpaces | fullScreenAuxiliary | stationary | ignoresCycle`, for the pill and its panels.
pub fn collection_behavior() -> NSWindowCollectionBehavior {
    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle
}

/// "Hide pill from capture" as AppKit has it.
pub fn sharing_type(excluded: bool) -> NSWindowSharingType {
    if excluded {
        NSWindowSharingType::None
    } else {
        NSWindowSharingType::ReadOnly
    }
}

fn sharing_name(sharing: NSWindowSharingType) -> &'static str {
    match sharing {
        NSWindowSharingType::None => "none",
        NSWindowSharingType::ReadOnly => "readOnly",
        _ => "readWrite",
    }
}

/// The names of the collection behaviour's flags the recipe is about, for `status`.
fn behavior_names(behavior: NSWindowCollectionBehavior) -> Vec<&'static str> {
    [
        (
            NSWindowCollectionBehavior::CanJoinAllSpaces,
            "canJoinAllSpaces",
        ),
        (
            NSWindowCollectionBehavior::MoveToActiveSpace,
            "moveToActiveSpace",
        ),
        (NSWindowCollectionBehavior::Managed, "managed"),
        (NSWindowCollectionBehavior::Transient, "transient"),
        (NSWindowCollectionBehavior::Stationary, "stationary"),
        (
            NSWindowCollectionBehavior::ParticipatesInCycle,
            "participatesInCycle",
        ),
        (NSWindowCollectionBehavior::IgnoresCycle, "ignoresCycle"),
        (
            NSWindowCollectionBehavior::FullScreenPrimary,
            "fullScreenPrimary",
        ),
        (
            NSWindowCollectionBehavior::FullScreenAuxiliary,
            "fullScreenAuxiliary",
        ),
        (NSWindowCollectionBehavior::FullScreenNone, "fullScreenNone"),
    ]
    .into_iter()
    .filter(|(flag, _)| behavior.contains(*flag))
    .map(|(_, name)| name)
    .collect()
}

/// How a native window of the pill stands, for the `status` dev hook and the self-test: a script
/// checks the recipe reached AppKit without looking at the screen.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub level: isize,
    pub collection_behavior: Vec<&'static str>,
    pub sharing_type: &'static str,
    pub ignores_mouse_events: bool,
    pub hides_on_deactivate: bool,
    /// `NSApp hide:` takes the window away with the app.
    pub can_hide: bool,
    pub can_become_key: bool,
    pub can_become_main: bool,
    pub is_key: bool,
    pub visible: bool,
    /// `occlusionState` has `.visible`: some of the window is on screen and WebKit paints it.
    pub occlusion_visible: bool,
    pub frame: Frame,
    /// `NSWindow.appearance`'s name, `None` while it follows the system.
    pub appearance: Option<String>,
}

pub fn recipe_of(ns: &NSWindow) -> Recipe {
    use objc2_app_kit::NSAppearanceCustomization;
    Recipe {
        level: ns.level(),
        collection_behavior: behavior_names(ns.collectionBehavior()),
        sharing_type: sharing_name(ns.sharingType()),
        ignores_mouse_events: ns.ignoresMouseEvents(),
        hides_on_deactivate: ns.hidesOnDeactivate(),
        can_hide: ns.canHide(),
        can_become_key: ns.canBecomeKeyWindow(),
        can_become_main: ns.canBecomeMainWindow(),
        is_key: ns.isKeyWindow(),
        visible: ns.isVisible(),
        occlusion_visible: ns
            .occlusionState()
            .contains(NSWindowOcclusionState::Visible),
        frame: to_frame(ns.frame()),
        appearance: ns.appearance().map(|a| a.name().to_string()),
    }
}

/// The pill's own `NSWindow`, on the main thread only.
fn with_ns<T>(
    window: &WebviewWindow,
    f: impl FnOnce(&NSWindow, MainThreadMarker) -> T,
) -> Option<T> {
    let mtm = MainThreadMarker::new()?;
    let ptr = NonNull::new(window.ns_window().ok()?.cast::<NSWindow>())?;
    // SAFETY: tauri hands out the live window's NSWindow, and this runs on the main thread that owns
    // it; the reference doesn't outlive this call.
    let ns = unsafe { ptr.as_ref() };
    Some(f(ns, mtm))
}

pub fn recipe(window: &WebviewWindow) -> Option<Recipe> {
    with_ns(window, |ns, _| recipe_of(ns))
}

fn to_frame(r: NSRect) -> Frame {
    Frame::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

fn to_rect(f: Frame) -> NSRect {
    NSRect::new(NSPoint::new(f.x, f.y), NSSize::new(f.w, f.h))
}

/// Once after the window is built: the recipe, and the observers for display changes and app
/// activations, which outlive any rebuilt window.
pub fn prepare(window: &WebviewWindow) {
    apply_styles(window);
    if let Some(r) = recipe(window) {
        log::info!(
            "pill: mac recipe level={} behaviour={} ignoresMouseEvents={} canBecomeKey={} canBecomeMain={} sharing={}",
            r.level,
            r.collection_behavior.join("|"),
            r.ignores_mouse_events,
            r.can_become_key,
            r.can_become_main,
            r.sharing_type
        );
    }
    static OBSERVE: Once = Once::new();
    OBSERVE.call_once(observe);
}

/// Display changes and app activations, until Voice quits. Both centers deliver on the main thread.
fn observe() {
    // A display connected, removed, rearranged or rescaled, and the Dock or the menu bar moved or
    // resized: the pill repairs itself once the burst settles (`Pill::display_changed`).
    let screens = RcBlock::new(|_: NonNull<NSNotification>| {
        if let Some(callbacks) = CALLBACKS.get() {
            (callbacks.display_changed)();
        }
    });
    // SAFETY: the block takes the notification pointer AppKit hands it and only calls the pill's
    // callback, which schedules work. The observer is leaked on purpose: the center keeps the block,
    // and Voice listens until it quits.
    let observer = unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(NSApplicationDidChangeScreenParametersNotification),
            None,
            None,
            &screens,
        )
    };
    std::mem::forget(observer);

    // The session coming back: the Mac or its screens woke, a switched-out session returned, the
    // screen unlocked. Windows repairs on the same moments (#13); the repair places, raises and puts
    // the panels back, and does nothing to a pill that isn't shown.
    let back = RcBlock::new(|_: NonNull<NSNotification>| {
        if let Some(callbacks) = CALLBACKS.get() {
            (callbacks.display_changed)();
        }
    });
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: AppKit's notification names are immutable strings that last as long as the process.
    let names = unsafe {
        [
            NSWorkspaceDidWakeNotification,
            NSWorkspaceScreensDidWakeNotification,
            NSWorkspaceSessionDidBecomeActiveNotification,
        ]
    };
    for name in names {
        // SAFETY: as above.
        let observer = unsafe {
            workspace.addObserverForName_object_queue_usingBlock(Some(name), None, None, &back)
        };
        std::mem::forget(observer);
    }
    // SAFETY: as above; the distributed center delivers on the main run loop too.
    let observer = unsafe {
        NSDistributedNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(&NSString::from_str(SCREEN_UNLOCKED)),
            None,
            None,
            &back,
        )
    };
    std::mem::forget(observer);

    // Another app came forward while the pill shows: put the pill back at the front of its level,
    // above a call app's own floating or status-level window (Ownkey #14).
    let activated = RcBlock::new(|note: NonNull<NSNotification>| {
        if !FOREGROUND.load(Ordering::Relaxed) || activated_is_us(note) {
            return;
        }
        if let Some(callbacks) = CALLBACKS.get() {
            (callbacks.foreground_changed)();
        }
    });
    // SAFETY: as above.
    let observer = unsafe {
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
                None,
                &activated,
            )
    };
    std::mem::forget(observer);
}

/// Whether an activation notification names Voice itself.
fn activated_is_us(note: NonNull<NSNotification>) -> bool {
    // SAFETY: AppKit hands the block a live notification for the length of the call.
    let note = unsafe { note.as_ref() };
    let Some(info) = note.userInfo() else {
        return false;
    };
    // SAFETY: the key is an immutable AppKit string that lasts as long as the process.
    let key = unsafe { NSWorkspaceApplicationKey };
    info.objectForKey(key)
        .and_then(|app| app.downcast::<NSRunningApplication>().ok())
        .is_some_and(|app| app.processIdentifier() == std::process::id() as i32)
}

/// Level, Spaces behaviour, click-through body and no deactivation hiding.
pub fn apply_styles(window: &WebviewWindow) {
    with_ns(window, |ns, _| set_recipe(ns));
}

fn set_recipe(ns: &NSWindow) {
    ns.setLevel(LEVEL);
    ns.setCollectionBehavior(collection_behavior());
    ns.setIgnoresMouseEvents(true);
    ns.setHidesOnDeactivate(false);
    ns.setCanHide(false);
}

/// Shows without activating: `orderFrontRegardless`, never `makeKeyAndOrderFront`.
pub fn show(window: &WebviewWindow) {
    raise(window);
}

pub fn hide(window: &WebviewWindow) {
    with_ns(window, |ns, _| ns.orderOut(None));
}

/// Ownkey #14: back to the front of its level without activating. The button panels follow it in
/// `update_hit`, so they stay above it.
pub fn raise(window: &WebviewWindow) -> bool {
    with_ns(window, |ns, _| {
        set_recipe(ns);
        ns.orderFrontRegardless();
    })
    .is_some()
}

fn display_id(screen: &NSScreen) -> Option<u32> {
    let description = screen.deviceDescription();
    let number = description.objectForKey(&NSString::from_str("NSScreenNumber"))?;
    number
        .downcast_ref::<NSNumber>()
        .map(|number| number.unsignedIntValue())
}

/// Every display, the primary one (with the menu bar) first, as `NSScreen.screens` orders them.
fn screens(mtm: MainThreadMarker) -> Vec<Screen> {
    NSScreen::screens(mtm)
        .iter()
        .enumerate()
        .map(|(index, screen)| {
            let insets = screen.safeAreaInsets();
            Screen {
                // A display without a number (never seen) still gets an id of its own.
                id: display_id(&screen).unwrap_or(u32::MAX - index as u32),
                frame: to_frame(screen.frame()),
                visible: to_frame(screen.visibleFrame()),
                safe_top: insets.top,
                safe_bottom: insets.bottom,
            }
        })
        .collect()
}

/// The front app's front window, in AppKit's space.
///
/// `CGWindowListCopyWindowInfo` is window information, not pixels: bounds, owner, layer and alpha
/// need no Screen Recording permission (only window names do). The list runs front to back; the
/// frontmost app's first window on layer 0 that is neither transparent nor tiny is its front window.
/// The Finder's desktop sits on a lower layer, so an app with no windows falls back to the pointer.
fn front_window(screens: &[Screen]) -> Option<Frame> {
    let front = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = front.processIdentifier();
    let primary_height = screens.first()?.frame.h;
    let list = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    )?;
    // SAFETY: a CFArray of CFDictionaries with CFString keys is toll-free bridged to the NSArray of
    // NSDictionaries this reads it as, and `list` outlives the borrow.
    let windows: &NSArray<NSDictionary<NSString, AnyObject>> =
        unsafe { &*CFRetained::as_ptr(&list).as_ptr().cast() };
    let number = |info: &NSDictionary<NSString, AnyObject>, key: &str| -> Option<f64> {
        info.objectForKey(&NSString::from_str(key))?
            .downcast_ref::<NSNumber>()
            .map(NSNumber::as_f64)
    };
    for info in windows.iter() {
        if number(&info, "kCGWindowOwnerPID") != Some(f64::from(pid))
            || number(&info, "kCGWindowLayer") != Some(0.0)
        {
            continue;
        }
        let bounds = info
            .objectForKey(&NSString::from_str("kCGWindowBounds"))?
            .downcast::<NSDictionary>()
            .ok()?;
        // SAFETY: `kCGWindowBounds` is a dictionary with string keys and number values.
        let bounds: Retained<NSDictionary<NSString, AnyObject>> =
            unsafe { Retained::cast_unchecked(bounds) };
        let frame = place::cg_to_appkit(
            primary_height,
            number(&bounds, "X")?,
            number(&bounds, "Y")?,
            number(&bounds, "Width")?,
            number(&bounds, "Height")?,
        );
        if place::is_user_window(number(&info, "kCGWindowAlpha"), &frame) {
            return Some(frame);
        }
    }
    None
}

/// The display of the front window, else the pointer's, else the primary one.
pub fn target_monitor() -> Monitor {
    let Some(mtm) = MainThreadMarker::new() else {
        return 0;
    };
    let all = screens(mtm);
    let mouse = NSEvent::mouseLocation();
    place::choose_screen(&all, front_window(&all), Some((mouse.x, mouse.y)))
        .map_or(0, |i| all[i].id as Monitor)
}

/// The display the pill was placed on while it is connected, else the one nearest the window.
pub fn current_monitor(window: &WebviewWindow) -> Option<Monitor> {
    with_ns(window, |ns, mtm| {
        let all = screens(mtm);
        let last = LAST_DISPLAY.with(Cell::get);
        place::screen_after_change(&all, last, &to_frame(ns.frame())).map(|i| all[i].id as Monitor)
    })
    .flatten()
}

/// Settings' work area under the pointer: nothing on macOS yet; Settings opens where AppKit puts it.
pub fn cursor_work_area() -> Option<(Rect, f64)> {
    None
}

/// Puts the window on `monitor` (a display id): centred, above the Dock or under the menu bar and the
/// notch. A display that went away since the id was taken falls back to the nearest one.
pub fn place(window: &WebviewWindow, monitor: Monitor, edge: Edge) -> Option<Frame> {
    with_ns(window, |ns, mtm| {
        let all = screens(mtm);
        let index = all
            .iter()
            .position(|s| s.id as Monitor == monitor)
            .or_else(|| place::screen_for_frame(&all, &to_frame(ns.frame())))?;
        let screen = all[index];
        let frame = place::mac_pill_frame(&screen, edge);
        ns.setFrame_display(to_rect(frame), true);
        LAST_DISPLAY.with(|last| last.set(Some(screen.id)));
        log::info!(
            "pill: mac placement display {} at ({:.0}, {:.0}) {:.0}x{:.0} pt; frame ({:.0}, {:.0}) {:.0}x{:.0}, visible ({:.0}, {:.0}) {:.0}x{:.0}, safe area top {:.0} bottom {:.0}, edge {edge:?}",
            screen.id,
            frame.x,
            frame.y,
            frame.w,
            frame.h,
            screen.frame.x,
            screen.frame.y,
            screen.frame.w,
            screen.frame.h,
            screen.visible.x,
            screen.visible.y,
            screen.visible.w,
            screen.visible.h,
            screen.safe_top,
            screen.safe_bottom,
        );
        Some(frame)
    })
    .flatten()
}

/// The recipe in the shape Windows reports its extended styles in, so the self-test reads both the
/// same way: `exStyle` is the level, `noactivate` that the window can be neither key nor main,
/// `topmost` that it sits at the pill's level, `transparent` that clicks go through, `toolwindow` that
/// it stays out of Cmd+` and every Space's window cycle, `layered` that it isn't opaque.
pub fn styles_of(ns: &NSWindow) -> Styles {
    let behavior = ns.collectionBehavior();
    Styles {
        ex_style: u32::try_from(ns.level()).unwrap_or(0),
        toolwindow: behavior.contains(collection_behavior()),
        noactivate: !ns.canBecomeKeyWindow() && !ns.canBecomeMainWindow(),
        topmost: ns.level() >= LEVEL,
        transparent: ns.ignoresMouseEvents(),
        layered: !ns.isOpaque(),
        appwindow: false,
        visible: ns.isVisible(),
    }
}

pub fn styles(window: &WebviewWindow) -> Option<Styles> {
    with_ns(window, |ns, _| styles_of(ns))
}

/// While the pill shows, another app's activation raises it.
pub fn foreground_hook(on: bool) {
    FOREGROUND.store(on, Ordering::Relaxed);
}

/// Where a button of the pill's page is on screen, in AppKit points.
pub fn button_frame(window: &WebviewWindow, css: &place::CssRect) -> Option<Frame> {
    with_ns(window, |ns, _| {
        place::css_to_points(&to_frame(ns.frame()), css)
    })
    .flatten()
}

/// Whether WebKit can paint the pill now. It runs no frames for a window that is wholly covered or
/// off screen, so the page's paint acknowledgement can't come then, and the paint watchdog waits
/// instead of rebuilding a window that is fine.
///
/// The screen lock counts, but not whether the session is the console's: a session someone uses over
/// Screen Sharing paints without being on the console. A switched-out session nobody shows is told
/// apart by the window's occlusion, which AppKit reports per session.
pub fn paint_expected(window: &WebviewWindow) -> bool {
    let (_, screen_locked) = crate::session_mac::session_flags();
    let visible = with_ns(window, |ns, _| {
        ns.occlusionState()
            .contains(NSWindowOcclusionState::Visible)
    });
    paint_expected_from(screen_locked, visible)
}

/// The rule `paint_expected` applies: the screen isn't locked and the window isn't occluded. A window
/// that can't be asked (gone, or not the main thread) counts as painting, so the watchdog still
/// repairs it.
pub fn paint_expected_from(screen_locked: Option<bool>, occlusion_visible: Option<bool>) -> bool {
    screen_locked != Some(true) && occlusion_visible != Some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_from_capture_maps_to_sharing_none_and_back_to_read_only() {
        assert_eq!(sharing_type(true), NSWindowSharingType::None);
        assert_eq!(sharing_type(false), NSWindowSharingType::ReadOnly);
        assert_eq!(sharing_name(sharing_type(true)), "none");
        assert_eq!(sharing_name(sharing_type(false)), "readOnly");
    }

    #[test]
    fn the_recipe_names_its_four_flags_and_the_status_level() {
        assert_eq!(
            behavior_names(collection_behavior()),
            [
                "canJoinAllSpaces",
                "stationary",
                "ignoresCycle",
                "fullScreenAuxiliary"
            ]
        );
        assert_eq!(LEVEL, 25);
    }

    #[test]
    fn the_watchdog_waits_while_nobody_can_see_the_pill() {
        assert!(paint_expected_from(Some(false), Some(true)));
        assert!(paint_expected_from(None, Some(true)), "no session flag");
        assert!(
            paint_expected_from(Some(false), None),
            "unknown counts as painting"
        );
        // Wholly covered, or a switched-out session nobody shows: WebKit paints nothing.
        assert!(!paint_expected_from(Some(false), Some(false)));
        // Locked.
        assert!(!paint_expected_from(Some(true), Some(true)));
        assert!(!paint_expected_from(Some(true), None));
    }
}
