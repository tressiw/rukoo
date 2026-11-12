//! Notifications on macOS, through Notification Center, in place of the Windows tray balloon.
//!
//! `UNUserNotificationCenter` only works inside an app bundle: an unbundled binary has no bundle id, and
//! the framework raises an Objective-C exception when it is asked for the shared center there. macOS may
//! also refuse notifications to an ad-hoc signed bundle, as #105 reports for Rukoo Mail. In both cases the
//! text stays in the log, which `tray.rs` writes before it calls in here, and nothing fails.
//!
//! The authorization request goes out once, with alert and sound, the first time a notification is
//! needed. Notifications that arrive while macOS hasn't answered wait for the answer; a denial turns them
//! into log lines and is reported once. Nothing here waits for a completion handler.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
    UNNotificationSound, UNUserNotificationCenter,
};

/// What macOS has said about our right to post, and what waits for that answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Permission {
    /// Nobody has asked yet.
    #[default]
    Unknown,
    /// The authorization request is out and hasn't been answered.
    Asking,
    /// Allowed.
    Granted,
    /// Refused: the text stays in the log.
    Denied,
}

/// What happens to one notification when it is handed over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Post it now.
    Post,
    /// Hold it until macOS answers, and ask now.
    Ask,
    /// Hold it; the question is out already.
    Wait,
    /// Only the log gets it.
    LogOnly,
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

    /// Decides what happens to one notification, keeping it while the question is out.
    fn accept(&mut self, title: &str, body: &str) -> Step {
        match self.permission {
            Permission::Granted => Step::Post,
            Permission::Unknown => {
                self.permission = Permission::Asking;
                self.pending.push((title.to_string(), body.to_string()));
                Step::Ask
            }
            Permission::Asking => {
                if self.pending.len() >= PENDING_MAX {
                    self.pending.remove(0);
                }
                self.pending.push((title.to_string(), body.to_string()));
                Step::Wait
            }
            Permission::Denied => Step::LogOnly,
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
/// the authorization request and the post have completion handlers, not results.
pub fn notify(title: &str, body: &str) {
    if !bundled() {
        if !UNBUNDLED_LOGGED.swap(true, Ordering::Relaxed) {
            log::info!(
                "notifications: Voice isn't in an app bundle, so macOS notifications stay in the log"
            );
        }
        return;
    }
    // The lock goes before anything that can call back into macOS, so a completion handler that runs at
    // once can't meet a held lock.
    let step = state().accept(title, body);
    match step {
        Step::Post => post(title, body),
        Step::Ask => request_authorization(),
        Step::Wait => {}
        // tray.rs already wrote the text to the log.
        Step::LogOnly => {}
    }
}

/// An unbundled binary has no bundle id, and asking for the notification center there throws.
fn bundled() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
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
        if !DENIED_LOGGED.swap(true, Ordering::Relaxed) {
            let reason = unsafe { error.as_ref() }.map(|error| error.localizedDescription());
            match reason {
                Some(reason) => {
                    let reason: &NSString = &reason;
                    log::warn!(
                        "notifications: macOS didn't allow notifications for Voice ({reason}), so they stay in the log"
                    );
                }
                None => log::warn!(
                    "notifications: macOS didn't allow notifications for Voice, so they stay in the log"
                ),
            }
        }
    });
    let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
    UNUserNotificationCenter::currentNotificationCenter()
        .requestAuthorizationWithOptions_completionHandler(options, &block);
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

    #[test]
    fn the_first_notification_asks_and_waits() {
        let mut state = State::new();
        assert_eq!(state.accept("Rukoo Voice", "Welcome"), Step::Ask);
        // A second one before the answer doesn't ask again.
        assert_eq!(state.accept("Rukoo Voice", "Started"), Step::Wait);
        assert_eq!(state.pending.len(), 2);
    }

    #[test]
    fn a_prompt_nobody_answers_does_not_pile_up() {
        let mut state = State::new();
        state.accept("Rukoo Voice", "Welcome");
        for i in 0..PENDING_MAX {
            assert_eq!(
                state.accept("Rukoo Voice", &format!("later {i}")),
                Step::Wait
            );
        }
        assert_eq!(state.pending.len(), PENDING_MAX);
        assert_eq!(state.pending[0].1, "later 0");
        assert_eq!(state.pending[PENDING_MAX - 1].1, "later 7");
    }

    #[test]
    fn a_grant_posts_what_waited_and_everything_after() {
        let mut state = State::new();
        state.accept("Rukoo Voice", "Welcome");
        let waiting = state.answered(true);
        assert_eq!(state.permission, Permission::Granted);
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].1, "Welcome");
        assert_eq!(state.accept("Rukoo Voice", "Later"), Step::Post);
        assert!(state.pending.is_empty());
    }

    #[test]
    fn a_denial_keeps_the_log_and_nothing_fails() {
        let mut state = State::new();
        state.accept("Rukoo Voice", "Welcome");
        let dropped = state.answered(false);
        assert_eq!(state.permission, Permission::Denied);
        // The caller logs the refusal once; the text of every notification is already in the log.
        assert_eq!(dropped.len(), 1);
        assert_eq!(state.accept("Rukoo Voice", "Later"), Step::LogOnly);
        assert_eq!(state.accept("Rukoo Voice", "Again"), Step::LogOnly);
        assert!(state.pending.is_empty());
    }
}
