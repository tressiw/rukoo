//! Notifications on macOS, through Notification Center, in place of the Windows tray balloon.
//!
//! `UNUserNotificationCenter` only works inside an app bundle: an unbundled binary has no bundle id, and
//! the framework raises an Objective-C exception when it is asked for the shared center there. macOS may
//! also refuse notifications to an ad-hoc signed bundle, as #105 reports for Rukoo Mail. In both cases the
//! text stays in the log, which `tray.rs` writes before it calls in here, and nothing fails.
//!
//! Before the first notification, Voice reads what macOS already decided (`getNotificationSettings`, which
//! never prompts). A granted or denied answer from an earlier run applies at once. Only when macOS has
//! never been asked does Voice send the authorization request, once, with alert and sound. It sends it
//! only while someone is at the console with the screen unlocked: asked during a lock, macOS refuses
//! without a prompt and never lists the app under System Settings > Notifications, so the user has
//! nothing to switch on. Voice can start locked (Start at login after an automatic login, a restart after
//! a crash). Until the screen is unlocked, notifications wait in the queue below and the next one after
//! the unlock sends the request. Notifications that arrive while macOS hasn't answered wait for the
//! answer; a denial turns them into log lines and is reported once. Nothing here waits for a completion
//! handler.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotificationRequest, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
};

use crate::session_mac::at_the_console;

/// What macOS has said about our right to post, and what waits for that answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Permission {
    /// Nobody has looked yet.
    #[default]
    Unknown,
    /// The settings are being read; that never prompts.
    Checking,
    /// macOS has never been asked, and nobody is at the console to see the prompt.
    Deferred,
    /// The authorization request is out and hasn't been answered.
    Asking,
    /// Allowed.
    Granted,
    /// Refused: the text stays in the log.
    Denied,
}

/// What macOS already decided, as `getNotificationSettings` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Authorization {
    NotDetermined,
    Denied,
    /// Authorized, provisional or ephemeral: all three let a notification through.
    Granted,
}

impl Authorization {
    fn from_status(status: UNAuthorizationStatus) -> Authorization {
        match status {
            UNAuthorizationStatus::Denied => Authorization::Denied,
            UNAuthorizationStatus::Authorized
            | UNAuthorizationStatus::Provisional
            | UNAuthorizationStatus::Ephemeral => Authorization::Granted,
            _ => Authorization::NotDetermined,
        }
    }
}

/// What happens to one notification when it is handed over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Post it now.
    Post,
    /// Hold it and read what macOS decided so far.
    Check,
    /// Hold it until macOS answers, and ask now.
    Ask,
    /// Hold it; the question is out, or the screen is locked.
    Wait,
    /// Only the log gets it.
    LogOnly,
    /// Voice isn't in an app bundle: only the log gets it, and the state stays untouched.
    Unbundled,
}

/// What the settings read means for the notifications that wait.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Allowed already: post these.
    Post(Vec<(String, String)>),
    /// Refused already: drop these.
    Drop(Vec<(String, String)>),
    /// Never asked and someone is at the console: send the request.
    Ask,
    /// Never asked and the screen is locked: keep them until a notification arrives after the unlock.
    Hold,
}

struct State {
    permission: Permission,
    /// Notifications that arrived before macOS answered the authorization request.
    pending: Vec<(String, String)>,
}

/// A prompt nobody answers must not let notifications pile up for the life of the process.
const PENDING_MAX: usize = 8;

impl State {
    const fn new() -> State {
        State {
            permission: Permission::Unknown,
            pending: Vec::new(),
        }
    }

    /// Decides what happens to one notification, keeping it while the question is out. `at_console` is
    /// whether the session is on the console with the screen unlocked.
    fn accept(&mut self, title: &str, body: &str, at_console: bool) -> Step {
        match self.permission {
            Permission::Granted => Step::Post,
            Permission::Denied => Step::LogOnly,
            Permission::Unknown => {
                self.permission = Permission::Checking;
                self.hold(title, body);
                Step::Check
            }
            Permission::Checking | Permission::Asking => {
                self.hold(title, body);
                Step::Wait
            }
            Permission::Deferred => {
                self.hold(title, body);
                if at_console {
                    self.permission = Permission::Asking;
                    Step::Ask
                } else {
                    Step::Wait
                }
            }
        }
    }

    /// Keeps a notification for the answer, dropping the oldest past the cap.
    fn hold(&mut self, title: &str, body: &str) {
        if self.pending.len() >= PENDING_MAX {
            self.pending.remove(0);
        }
        self.pending.push((title.to_string(), body.to_string()));
    }

    /// The settings came back: an earlier answer applies, and only a macOS that was never asked gets the
    /// request, and only from someone at the console.
    fn settings_read(&mut self, authorization: Authorization, at_console: bool) -> Verdict {
        match authorization {
            Authorization::Granted => Verdict::Post(self.answered(true)),
            Authorization::Denied => Verdict::Drop(self.answered(false)),
            Authorization::NotDetermined if at_console => {
                self.permission = Permission::Asking;
                Verdict::Ask
            }
            Authorization::NotDetermined => {
                self.permission = Permission::Deferred;
                Verdict::Hold
            }
        }
    }

    /// macOS answered the authorization request: everything that waited goes out, or is dropped when the
    /// answer was no.
    fn answered(&mut self, granted: bool) -> Vec<(String, String)> {
        self.permission = if granted {
            Permission::Granted
        } else {
            Permission::Denied
        };
        std::mem::take(&mut self.pending)
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());
/// The reason macOS notifications stay in the log is worth saying once, not on every notification.
static UNBUNDLED_LOGGED: AtomicBool = AtomicBool::new(false);
static DENIED_LOGGED: AtomicBool = AtomicBool::new(false);
static POST_FAILED_LOGGED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shows `title` and `body` in Notification Center, or says once why they stay in the log. Never blocks:
/// the settings read, the authorization request and the post have completion handlers, not results.
pub fn notify(title: &str, body: &str) {
    // The lock goes before anything that can call back into macOS, so a completion handler that runs at
    // once can't meet a held lock.
    let step = route(&mut state(), bundled(), at_the_console(), title, body);
    match step {
        Step::Post => post(title, body),
        Step::Check => check_settings(),
        Step::Ask => request_authorization(),
        Step::Wait => {}
        // tray.rs already wrote the text to the log.
        Step::LogOnly => {}
        Step::Unbundled => {
            if !UNBUNDLED_LOGGED.swap(true, Ordering::Relaxed) {
                log::info!(
                    "notifications: Voice isn't in an app bundle, so macOS notifications stay in the log"
                );
            }
        }
    }
}

/// The decision for one notification with the outside world passed in.
fn route(state: &mut State, bundled: bool, at_console: bool, title: &str, body: &str) -> Step {
    if !bundled {
        return Step::Unbundled;
    }
    state.accept(title, body, at_console)
}

/// An unbundled binary has no bundle id, and asking for the notification center there throws.
fn bundled() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Reads what macOS decided in an earlier run. This never prompts, so it is safe on a locked screen.
fn check_settings() {
    let block = RcBlock::new(|settings: NonNull<UNNotificationSettings>| {
        // SAFETY: macOS hands the completion handler a live settings object for the length of the call.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        let authorization = Authorization::from_status(status);
        let at_console = at_the_console();
        let verdict = state().settings_read(authorization, at_console);
        match verdict {
            Verdict::Post(pending) => {
                log::info!("notifications: macOS allowed them in an earlier run");
                for (title, body) in pending {
                    post(&title, &body);
                }
            }
            Verdict::Drop(_) => log_denied(None),
            Verdict::Ask => request_authorization(),
            Verdict::Hold => log::info!(
                "notifications: the screen is locked, so Voice asks macOS for permission after the unlock"
            ),
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&block);
}

fn request_authorization() {
    log::info!("notifications: asking macOS for permission to show alerts");
    let block = RcBlock::new(|granted: Bool, error: *mut NSError| {
        let granted = granted.as_bool();
        let pending = state().answered(granted);
        if granted {
            log::info!("notifications: macOS allows them");
            for (title, body) in pending {
                post(&title, &body);
            }
            return;
        }
        let reason = unsafe { error.as_ref() }.map(|error| error.localizedDescription());
        log_denied(reason.as_deref());
    });
    let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(options, &block);
}

/// Says once that notifications stay in the log, with macOS's reason when it gave one.
fn log_denied(reason: Option<&NSString>) {
    if DENIED_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    match reason {
        Some(reason) => log::warn!(
            "notifications: macOS didn't allow notifications for Voice ({reason}), so they stay in the log"
        ),
        None => log::warn!(
            "notifications: macOS didn't allow notifications for Voice, so they stay in the log"
        ),
    }
}

fn post(title: &str, body: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let identifier = NSString::from_str(&format!("rukoo-voice-{}-{id}", std::process::id()));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
    let block = RcBlock::new(|error: *mut NSError| {
        let Some(error) = (unsafe { error.as_ref() }) else {
            return;
        };
        if !POST_FAILED_LOGGED.swap(true, Ordering::Relaxed) {
            let description = error.localizedDescription();
            let description: &NSString = &description;
            log::warn!(
                "notifications: Notification Center didn't take it ({description}), so they stay in the log"
            );
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&block));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state past the settings read: macOS never decided, and `at_console` says who is there.
    fn undecided(at_console: bool) -> (State, Verdict) {
        let mut state = State::new();
        assert_eq!(
            state.accept("Rukoo Voice", "Welcome", at_console),
            Step::Check
        );
        let verdict = state.settings_read(Authorization::NotDetermined, at_console);
        (state, verdict)
    }

    #[test]
    fn the_first_notification_reads_the_settings_and_waits() {
        let mut state = State::new();
        assert_eq!(state.accept("Rukoo Voice", "Welcome", true), Step::Check);
        // A second one before the answer neither reads nor asks again.
        assert_eq!(state.accept("Rukoo Voice", "Started", true), Step::Wait);
        assert_eq!(state.pending.len(), 2);
    }

    #[test]
    fn an_unlocked_console_asks_when_macos_never_decided() {
        let (state, verdict) = undecided(true);
        assert_eq!(verdict, Verdict::Ask);
        assert_eq!(state.permission, Permission::Asking);
        assert_eq!(state.pending.len(), 1);
    }

    #[test]
    fn a_locked_screen_does_not_ask_and_keeps_the_notification() {
        let (mut state, verdict) = undecided(false);
        assert_eq!(verdict, Verdict::Hold);
        assert_eq!(state.permission, Permission::Deferred);
        assert_eq!(state.pending.len(), 1);
        // Still locked: later ones queue and nothing asks.
        assert_eq!(state.accept("Rukoo Voice", "Later", false), Step::Wait);
        assert_eq!(state.permission, Permission::Deferred);
        assert_eq!(state.pending.len(), 2);
    }

    #[test]
    fn the_first_notification_after_the_unlock_asks_with_everything_that_waited() {
        let (mut state, _) = undecided(false);
        state.accept("Rukoo Voice", "Locked", false);
        assert_eq!(state.accept("Rukoo Voice", "Unlocked", true), Step::Ask);
        assert_eq!(state.permission, Permission::Asking);
        assert_eq!(state.pending.len(), 3);
        let waiting = state.answered(true);
        assert_eq!(waiting.len(), 3);
        assert_eq!(state.accept("Rukoo Voice", "After", false), Step::Post);
    }

    #[test]
    fn an_earlier_grant_posts_even_on_a_locked_screen() {
        let mut state = State::new();
        state.accept("Rukoo Voice", "Welcome", false);
        let verdict = state.settings_read(Authorization::Granted, false);
        let Verdict::Post(waiting) = verdict else {
            panic!("expected a post, got {verdict:?}");
        };
        assert_eq!(waiting.len(), 1);
        assert_eq!(state.permission, Permission::Granted);
        assert_eq!(state.accept("Rukoo Voice", "Later", false), Step::Post);
    }

    #[test]
    fn an_earlier_denial_asks_nothing_and_keeps_the_log() {
        for at_console in [true, false] {
            let mut state = State::new();
            state.accept("Rukoo Voice", "Welcome", at_console);
            let verdict = state.settings_read(Authorization::Denied, at_console);
            assert!(matches!(verdict, Verdict::Drop(ref dropped) if dropped.len() == 1));
            assert_eq!(state.permission, Permission::Denied);
            assert_eq!(
                state.accept("Rukoo Voice", "Later", at_console),
                Step::LogOnly
            );
            assert!(state.pending.is_empty());
        }
    }

    #[test]
    fn the_settings_map_to_what_they_allow() {
        assert_eq!(
            Authorization::from_status(UNAuthorizationStatus::NotDetermined),
            Authorization::NotDetermined
        );
        assert_eq!(
            Authorization::from_status(UNAuthorizationStatus::Denied),
            Authorization::Denied
        );
        for status in [
            UNAuthorizationStatus::Authorized,
            UNAuthorizationStatus::Provisional,
            UNAuthorizationStatus::Ephemeral,
        ] {
            assert_eq!(Authorization::from_status(status), Authorization::Granted);
        }
    }

    #[test]
    fn a_prompt_nobody_answers_does_not_pile_up() {
        let mut state = State::new();
        state.accept("Rukoo Voice", "Welcome", true);
        for i in 0..PENDING_MAX {
            assert_eq!(
                state.accept("Rukoo Voice", &format!("later {i}"), true),
                Step::Wait
            );
        }
        assert_eq!(state.pending.len(), PENDING_MAX);
        assert_eq!(state.pending[0].1, "later 0");
        assert_eq!(state.pending[PENDING_MAX - 1].1, "later 7");
    }

    #[test]
    fn a_locked_screen_keeps_the_same_cap() {
        let (mut state, _) = undecided(false);
        for i in 0..20 {
            assert_eq!(
                state.accept("Rukoo Voice", &format!("locked {i}"), false),
                Step::Wait
            );
        }
        assert_eq!(state.pending.len(), PENDING_MAX);
        assert_eq!(state.pending[PENDING_MAX - 1].1, "locked 19");
    }

    #[test]
    fn a_grant_posts_what_waited_and_everything_after() {
        let mut state = State::new();
        state.accept("Rukoo Voice", "Welcome", true);
        state.settings_read(Authorization::NotDetermined, true);
        let waiting = state.answered(true);
        assert_eq!(state.permission, Permission::Granted);
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].1, "Welcome");
        assert_eq!(state.accept("Rukoo Voice", "Later", true), Step::Post);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn a_denial_keeps_the_log_and_nothing_fails() {
        let mut state = State::new();
        state.accept("Rukoo Voice", "Welcome", true);
        state.settings_read(Authorization::NotDetermined, true);
        let dropped = state.answered(false);
        assert_eq!(state.permission, Permission::Denied);
        // The caller logs the refusal once; the text of every notification is already in the log.
        assert_eq!(dropped.len(), 1);
        assert_eq!(state.accept("Rukoo Voice", "Later", true), Step::LogOnly);
        assert_eq!(state.accept("Rukoo Voice", "Again", true), Step::LogOnly);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn an_unbundled_binary_logs_only_and_touches_nothing() {
        let mut state = State::new();
        for at_console in [true, false] {
            let step = route(&mut state, false, at_console, "Rukoo Voice", "Welcome");
            assert_eq!(step, Step::Unbundled);
        }
        assert_eq!(state.permission, Permission::Unknown);
        assert!(state.pending.is_empty());
    }
}
