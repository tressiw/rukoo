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
//! Polling state heals itself: whatever happened between two reads, a key
//! that reads up on the next read is released then, also after the process was
//! suspended or the Mac slept. What a read can't see is a key-up the system
//! never reports, such as a key that still reads down after a wake; dictation's
//! watchdog and its longest recording cover that.

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{InputError, Key};

/// How often key state is sampled. 25 polls a second cost about 0.1% of one
/// core; 20 ms cost 0.65% (measured for Ownkey).
pub const POLL_INTERVAL: Duration = Duration::from_millis(40);

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
        let shared = Arc::new(Shared::default());
        let send = move |event| tx.send(event).is_ok();
        let poller = start_polling(keys, key_state, interval, send, &shared)?;
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
        let poller = match start_polling(keys, key_state, interval, send, &shared) {
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

fn start_polling<S, T>(
    keys: Vec<Key>,
    key_state: S,
    interval: Duration,
    send: T,
    shared: &Arc<Shared>,
) -> Result<JoinHandle<()>, InputError>
where
    S: FnMut(Key) -> bool + Send + 'static,
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
        .spawn(move || poll(&unique, key_state, interval, send, &shared))
        .map_err(thread_error)
}

/// Reads every key each `interval` and sends each change; `send` returns
/// `false` once nobody listens.
fn poll<S, T>(keys: &[Key], mut key_state: S, interval: Duration, mut send: T, shared: &Shared)
where
    S: FnMut(Key) -> bool,
    T: FnMut(HotkeyEvent) -> bool,
{
    // Starts with nothing held, so a key that is already down counts as a
    // press, as key repeat did for Ownkey's old hook.
    let mut held = vec![false; keys.len()];
    loop {
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
