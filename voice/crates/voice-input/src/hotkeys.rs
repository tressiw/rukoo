//! Hotkey presses and releases read from physical key state (port of ownkey-windows #11).
//!
//! Windows stops waiting for a low-level keyboard hook after
//! `LowLevelHooksTimeout`, and the hook never sees the events it missed. In
//! Ownkey a slow microphone open on the hook thread lost the hotkey release
//! and left push-to-talk stuck. So there is no hook: one thread samples key
//! state every 40 ms and queues each change, and the consumer handles them on
//! another thread. A slow handler delays later presses and releases but can't
//! hide them, and other apps never wait for Voice's input handling.
//!
//! A poll that wakes up more than [`STALL_LIMIT`] after its last read was
//! stopped: the Mac slept, the process was suspended or starved. A release can
//! have happened in that gap, so every key the poller held is released then,
//! unless it still reads down.

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{InputError, Key};

/// How often key state is sampled. 25 polls a second cost about 0.1% of one
/// core; 20 ms cost 0.65% (measured for Ownkey).
pub const POLL_INTERVAL: Duration = Duration::from_millis(40);

/// A gap this long between two reads means the poll thread itself was stopped,
/// not that the machine was busy. The spike for macOS measured 40 ms sleeps
/// waking at most 45 to 50 ms late, so this is 20 ordinary intervals. The
/// thread treats a longer gap as untrustworthy and releases a held key that no
/// longer reads down.
pub const STALL_LIMIT: Duration = Duration::from_secs(1);

/// A change in a hotkey's physical state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HotkeyEvent {
    Pressed(Key),
    Released(Key),
}

impl HotkeyEvent {
    pub fn key(self) -> Key {
        match self {
            HotkeyEvent::Pressed(key) | HotkeyEvent::Released(key) => key,
        }
    }
}

/// What the `hotkey-callbacks` thread handles, one at a time, in the order it arrived.
enum Dispatch {
    Event(HotkeyEvent),
    Run(Box<dyn FnOnce() + Send>),
}

#[derive(Debug, Default)]
struct Shared {
    stopped: Mutex<bool>,
    wake: Condvar,
    /// The dispatcher's queue for [`InOrder`]. Stopping drops it, so the
    /// dispatcher ends once the polling thread has dropped its sender too.
    queue: Mutex<Option<Sender<Dispatch>>>,
}

impl Shared {
    fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.wake.notify_all();
    }

    fn is_stopped(&self) -> bool {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sleeps for `timeout` or until stopped. Returns whether it was stopped.
    fn wait(&self, timeout: Duration) -> bool {
        let stopped = self.stopped.lock().unwrap_or_else(PoisonError::into_inner);
        let (stopped, _) = self
            .wake
            .wait_timeout_while(stopped, timeout, |stopped| !*stopped)
            .unwrap_or_else(PoisonError::into_inner);
        *stopped
    }
}

/// Stops a [`HotkeyPoller`] from anywhere, including from its own handler.
#[derive(Clone, Debug)]
pub struct Stopper(Arc<Shared>);

impl Stopper {
    pub fn stop(&self) {
        self.0.stop();
    }

    pub fn is_stopped(&self) -> bool {
        self.0.is_stopped()
    }
}

/// Runs work on a [`HotkeyPoller`]'s `hotkey-callbacks` thread, after the
/// presses and releases already queued there. Whatever the handler keeps,
/// such as push-to-talk's state, then changes on that one thread only, so work
/// from another thread can't land between a press and the start it causes.
#[derive(Clone, Debug)]
pub struct InOrder(Arc<Shared>);

impl InOrder {
    /// Queues `work` behind the events already waiting. The receiver yields
    /// once it has run, and disconnects without yielding when the poller stops
    /// first or the work panics. `None` once the poller has stopped. Don't wait
    /// on the receiver from the handler itself: the work runs after it.
    pub fn run(&self, work: impl FnOnce() + Send + 'static) -> Option<Receiver<()>> {
        let (done, finished) = mpsc::channel();
        let queue = self.0.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let work = Box::new(move || {
            work();
            let _ = done.send(());
        });
        queue.as_ref()?.send(Dispatch::Run(work)).ok()?;
        Some(finished)
    }
}

/// Polls hotkeys on a background thread named `hotkeys`.
///
/// A key that is already held when polling starts counts as a press. A tap
/// shorter than one poll interval is never seen, which is fine for
/// push-to-talk. Dropping the poller stops it without waiting for its threads,
/// because a handler may drop or stop its own poller.
#[derive(Debug)]
pub struct HotkeyPoller {
    shared: Arc<Shared>,
    poller: Option<JoinHandle<()>>,
    dispatcher: Option<JoinHandle<()>>,
}

impl HotkeyPoller {
    /// Polls the physical keyboard and sends every change to `tx` in order.
    /// Fails with [`InputError::Unsupported`] where the platform can't read the
    /// keyboard.
    pub fn spawn(keys: Vec<Key>, tx: Sender<HotkeyEvent>) -> Result<HotkeyPoller, InputError> {
        HotkeyPoller::spawn_with(keys, tx, crate::sys::key_state()?, POLL_INTERVAL)
    }

    /// Polls the physical keyboard and runs `handler` for every change, in
    /// order, on a thread named `hotkey-callbacks`. A panicking handler is
    /// reported by the panic hook and the next event still runs: one failed
    /// recording must not end push-to-talk.
    pub fn spawn_handler<H>(keys: Vec<Key>, handler: H) -> Result<HotkeyPoller, InputError>
    where
        H: FnMut(HotkeyEvent) + Send + 'static,
    {
        HotkeyPoller::spawn_handler_with(keys, crate::sys::key_state()?, POLL_INTERVAL, handler)
    }

    /// [`HotkeyPoller::spawn`] with an injected key-state function, for tests
    /// and for platforms that read keys another way.
    pub fn spawn_with<S>(
        keys: Vec<Key>,
        tx: Sender<HotkeyEvent>,
        key_state: S,
        interval: Duration,
    ) -> Result<HotkeyPoller, InputError>
    where
        S: FnMut(Key) -> bool + Send + 'static,
    {
        HotkeyPoller::spawn_with_clock(keys, tx, key_state, interval, Instant::now)
    }

    /// [`HotkeyPoller::spawn_with`] whose clock is injected too: the poller
    /// compares the time of each read with the last one to notice a stalled
    /// poll, and tests move that clock by hand.
    pub fn spawn_with_clock<S, C>(
        keys: Vec<Key>,
        tx: Sender<HotkeyEvent>,
        key_state: S,
        interval: Duration,
        clock: C,
    ) -> Result<HotkeyPoller, InputError>
    where
        S: FnMut(Key) -> bool + Send + 'static,
        C: FnMut() -> Instant + Send + 'static,
    {
        let shared = Arc::new(Shared::default());
        let send = move |event| tx.send(event).is_ok();
        let poller = start_polling(keys, key_state, clock, interval, send, &shared)?;
        Ok(HotkeyPoller {
            shared,
            poller: Some(poller),
            dispatcher: None,
        })
    }

    /// [`HotkeyPoller::spawn_handler`] with an injected key-state function.
    pub fn spawn_handler_with<S, H>(
        keys: Vec<Key>,
        key_state: S,
        interval: Duration,
        handler: H,
    ) -> Result<HotkeyPoller, InputError>
    where
        S: FnMut(Key) -> bool + Send + 'static,
        H: FnMut(HotkeyEvent) + Send + 'static,
    {
        HotkeyPoller::spawn_handler_with_clock(keys, key_state, interval, Instant::now, handler)
    }

    /// [`HotkeyPoller::spawn_handler_with`] with an injected clock, as
    /// [`HotkeyPoller::spawn_with_clock`].
    pub fn spawn_handler_with_clock<S, C, H>(
        keys: Vec<Key>,
        key_state: S,
        interval: Duration,
        clock: C,
        handler: H,
    ) -> Result<HotkeyPoller, InputError>
    where
        S: FnMut(Key) -> bool + Send + 'static,
        C: FnMut() -> Instant + Send + 'static,
        H: FnMut(HotkeyEvent) + Send + 'static,
    {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::channel();
        *shared.queue.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx.clone());
        let dispatcher = thread::Builder::new()
            .name("hotkey-callbacks".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || dispatch(rx, &shared, handler)
            })
            .map_err(thread_error)?;
        // If polling can't start, its sender is dropped, the queue's goes with
        // the stop and the dispatcher ends.
        let send = move |event| tx.send(Dispatch::Event(event)).is_ok();
        let poller = match start_polling(keys, key_state, clock, interval, send, &shared) {
            Ok(poller) => poller,
            Err(err) => {
                shared.stop();
                return Err(err);
            }
        };
        Ok(HotkeyPoller {
            shared,
            poller: Some(poller),
            dispatcher: Some(dispatcher),
        })
    }

    /// Stops polling. Never waits, so a handler can call it. Changes that were
    /// queued but not yet handled are dropped, as in Ownkey.
    pub fn stop(&self) {
        self.shared.stop();
    }

    /// A handle that runs work on the handler's thread, in order with the
    /// events. `None` for a poller that sends its events to a channel.
    pub fn in_order(&self) -> Option<InOrder> {
        self.dispatcher
            .as_ref()
            .map(|_| InOrder(Arc::clone(&self.shared)))
    }

    /// A handle that stops this poller, for handlers and other threads.
    pub fn stopper(&self) -> Stopper {
        Stopper(Arc::clone(&self.shared))
    }

    /// Whether the polling thread is still running.
    pub fn is_running(&self) -> bool {
        self.poller
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }

    /// Stops polling and waits for both threads to end. A handler that is
    /// still running is waited for. Called from the handler itself, it skips
    /// waiting for its own thread.
    pub fn join(mut self) {
        self.stop();
        for thread in [self.poller.take(), self.dispatcher.take()]
            .into_iter()
            .flatten()
        {
            if thread.thread().id() != thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for HotkeyPoller {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_polling<S, C, T>(
    keys: Vec<Key>,
    key_state: S,
    clock: C,
    interval: Duration,
    send: T,
    shared: &Arc<Shared>,
) -> Result<JoinHandle<()>, InputError>
where
    S: FnMut(Key) -> bool + Send + 'static,
    C: FnMut() -> Instant + Send + 'static,
    T: FnMut(HotkeyEvent) -> bool + Send + 'static,
{
    let mut unique = Vec::with_capacity(keys.len());
    for key in keys {
        if !unique.contains(&key) {
            unique.push(key);
        }
    }
    let shared = Arc::clone(shared);
    thread::Builder::new()
        .name("hotkeys".into())
        .spawn(move || poll(&unique, key_state, clock, interval, send, &shared))
        .map_err(thread_error)
}

/// Reads every key each `interval` and sends each change; `send` returns
/// `false` once nobody listens.
fn poll<S, C, T>(
    keys: &[Key],
    mut key_state: S,
    mut clock: C,
    interval: Duration,
    mut send: T,
    shared: &Shared,
) where
    S: FnMut(Key) -> bool,
    C: FnMut() -> Instant,
    T: FnMut(HotkeyEvent) -> bool,
{
    // Starts with nothing held, so a key that is already down counts as a
    // press, as key repeat did for Ownkey's old hook.
    let mut held = vec![false; keys.len()];
    let mut last_read = clock();
    loop {
        let now = clock();
        if now.saturating_duration_since(last_read) > STALL_LIMIT
            && !release_after_stall(keys, &mut held, &mut key_state, &mut send, shared)
        {
            return;
        }
        last_read = now;
        for (&key, was_down) in keys.iter().zip(held.iter_mut()) {
            let down = key_state(key);
            if down == *was_down {
                continue;
            }
            *was_down = down;
            if shared.is_stopped() {
                return;
            }
            let event = if down {
                HotkeyEvent::Pressed(key)
            } else {
                HotkeyEvent::Released(key)
            };
            if !send(event) {
                return; // nobody is listening any more
            }
        }
        if shared.wait(interval) {
            return;
        }
    }
}

/// The rule for a poll that woke up after a stall: a held key counts as
/// released unless it still reads down.
///
/// The fresh read is what decides, because nothing else can: if the Mac slept
/// with the key held, the key-up was never delivered to Voice and the key no
/// longer reads down, so the recording stops at wake. A key that does read down
/// keeps its hold instead of being released and pressed again: a release and a
/// new press inside the gap can't be told apart from one long hold, and cutting
/// a live recording in two is worse than treating the hold as continuing. (The
/// app's session observer already releases the key before a sleep, so this is
/// the second line of defense, not the first.)
///
/// Returns `false` when polling has to end.
fn release_after_stall<S>(
    keys: &[Key],
    held: &mut [bool],
    key_state: &mut S,
    send: &mut dyn FnMut(HotkeyEvent) -> bool,
    shared: &Shared,
) -> bool
where
    S: FnMut(Key) -> bool,
{
    for (&key, was_down) in keys.iter().zip(held.iter_mut()) {
        if !*was_down {
            continue;
        }
        if key_state(key) {
            continue;
        }
        *was_down = false;
        if shared.is_stopped() {
            return false;
        }
        if !send(HotkeyEvent::Released(key)) {
            return false;
        }
    }
    true
}

fn dispatch<H>(rx: Receiver<Dispatch>, shared: &Shared, mut handler: H)
where
    H: FnMut(HotkeyEvent),
{
    // Ends when the polling thread exits and the stop dropped the queue.
    for item in rx {
        if shared.is_stopped() {
            return;
        }
        // One failed recording must not end push-to-talk. The panic hook has
        // already reported the panic.
        let _ = panic::catch_unwind(AssertUnwindSafe(|| match item {
            Dispatch::Event(event) => handler(event),
            Dispatch::Run(work) => work(),
        }));
    }
}

fn thread_error(err: std::io::Error) -> InputError {
    InputError::System(format!("Could not start the hotkey listener: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A read after a stall releases a held key that no longer reads down, and
    /// keeps one that does.
    #[test]
    fn a_stalled_poll_releases_only_keys_that_no_longer_read_down() {
        let (tx, rx) = mpsc::channel();
        let mut held = [true, true];
        let mut state = |key: Key| key == Key::F14;
        assert!(release_after_stall(
            &[Key::F13, Key::F14],
            &mut held,
            &mut state,
            &mut |event| tx.send(event).is_ok(),
            &Shared::default()
        ));
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [HotkeyEvent::Released(Key::F13)]
        );
        assert_eq!(held, [false, true], "F14 kept its hold");
        // The next ordinary poll sees no second release for F13.
        let mut state = |_: Key| false;
        assert!(release_after_stall(
            &[Key::F13, Key::F14],
            &mut held,
            &mut state,
            &mut |event| tx.send(event).is_ok(),
            &Shared::default()
        ));
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [HotkeyEvent::Released(Key::F14)]
        );
        assert_eq!(held, [false, false]);
    }

    #[test]
    fn a_stalled_poll_ends_when_nobody_listens() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let mut held = [true];
        let mut state = |_: Key| false;
        assert!(!release_after_stall(
            &[Key::F13],
            &mut held,
            &mut state,
            &mut |event| tx.send(event).is_ok(),
            &Shared::default()
        ));
    }

    #[test]
    fn the_stall_limit_is_many_poll_intervals() {
        assert_eq!(STALL_LIMIT.as_millis() / POLL_INTERVAL.as_millis(), 25);
    }
}
