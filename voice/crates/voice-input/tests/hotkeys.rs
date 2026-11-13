//! The polling listener, ported from Ownkey's `tests/test_windows_hotkeys.py`
//! (fix #11). Every test drives an injected key-state function; none reads or
//! sends real keys.

mod common;

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use common::{wait_for, Keyboard, Latch, INTERVAL, PATIENCE};
use voice_input::HotkeyEvent::{Pressed, Released};
use voice_input::{HotkeyEvent, HotkeyPoller, Key, Stopper};

type Events = Arc<Mutex<Vec<HotkeyEvent>>>;

fn recorder() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

fn is(events: &Events, expected: &[HotkeyEvent]) -> bool {
    *events.lock().unwrap() == expected
}

fn listen(
    keyboard: &Arc<Keyboard>,
    handler: impl FnMut(HotkeyEvent) + Send + 'static,
) -> HotkeyPoller {
    HotkeyPoller::spawn_handler_with(vec![Key::RightAlt], keyboard.state(), INTERVAL, handler)
        .unwrap()
}

const HOLD: [HotkeyEvent; 2] = [Pressed(Key::RightAlt), Released(Key::RightAlt)];
const TWO_HOLDS: [HotkeyEvent; 4] = [
    Pressed(Key::RightAlt),
    Released(Key::RightAlt),
    Pressed(Key::RightAlt),
    Released(Key::RightAlt),
];

/// Ownkey #11 test 1 (`:53`).
#[test]
fn reports_one_press_and_one_release_per_hold() {
    let (keyboard, events) = (Keyboard::new(1, &[]), recorder());
    let poller = listen(&keyboard, {
        let events = Arc::clone(&events);
        move |event| events.lock().unwrap().push(event)
    });
    keyboard.set(Key::RightAlt, true);
    assert!(wait_for(|| is(&events, &HOLD[..1])));
    thread::sleep(Duration::from_millis(50));
    assert!(is(&events, &HOLD[..1]), "no repeats while held");
    keyboard.set(Key::RightAlt, false);
    assert!(wait_for(|| is(&events, &HOLD)));
    poller.join();
}

/// Ownkey #11 test 2 (`:63`): the microphone is slow to open while the user
/// lets go, holds again and lets go.
#[test]
fn changes_during_a_slow_press_handler_arrive_in_order() {
    let (keyboard, events, opening) = (
        Keyboard::new(1, &[]),
        recorder(),
        Arc::new(Latch::new(false)),
    );
    let poller = listen(&keyboard, {
        let (events, opening) = (Arc::clone(&events), Arc::clone(&opening));
        move |event| {
            let first = {
                let mut events = events.lock().unwrap();
                events.push(event);
                events.len() == 1
            };
            if first {
                opening.wait();
            }
        }
    });
    keyboard.set(Key::RightAlt, true);
    assert!(wait_for(|| events.lock().unwrap().len() == 1));
    for down in [false, true, false] {
        keyboard.set(Key::RightAlt, down);
    }
    opening.open();
    assert!(wait_for(|| is(&events, &TWO_HOLDS)));
    poller.join();
}

/// Ownkey #11 test 3 (`:79`): a hold while the microphone is slow to close.
#[test]
fn a_hold_during_a_slow_release_handler_is_not_lost() {
    let (keyboard, events, closing) = (
        Keyboard::new(1, &[]),
        recorder(),
        Arc::new(Latch::new(false)),
    );
    let poller = listen(&keyboard, {
        let (events, closing) = (Arc::clone(&events), Arc::clone(&closing));
        move |event| {
            let first_release = {
                let mut events = events.lock().unwrap();
                events.push(event);
                events.len() == 2
            };
            if first_release {
                closing.wait();
            }
        }
    });
    keyboard.set(Key::RightAlt, true);
    keyboard.set(Key::RightAlt, false);
    assert!(wait_for(|| events.lock().unwrap().len() == 2));
    keyboard.set(Key::RightAlt, true);
    keyboard.set(Key::RightAlt, false);
    closing.open();
    assert!(wait_for(|| is(&events, &TWO_HOLDS)));
    poller.join();
}

/// Ownkey's `test_key_already_held_counts_as_a_press` (`:96`).
#[test]
fn a_key_already_held_counts_as_a_press() {
    let (keyboard, events) = (Keyboard::new(1, &[Key::RightAlt]), recorder());
    let poller = listen(&keyboard, {
        let events = Arc::clone(&events);
        move |event| events.lock().unwrap().push(event)
    });
    assert!(wait_for(|| is(&events, &HOLD[..1])));
    keyboard.set(Key::RightAlt, false);
    assert!(wait_for(|| is(&events, &HOLD)));
    poller.join();
}

/// Ownkey's `test_callback_can_stop_its_own_listener` (`:103`).
#[test]
fn a_handler_can_stop_its_own_poller() {
    let (keyboard, events) = (Keyboard::new(1, &[]), recorder());
    let stopper: Arc<OnceLock<Stopper>> = Arc::new(OnceLock::new());
    let poller = listen(&keyboard, {
        let (events, stopper) = (Arc::clone(&events), Arc::clone(&stopper));
        move |event| {
            if let Released(_) = event {
                events.lock().unwrap().push(event);
                stopper.get().unwrap().stop();
            }
        }
    });
    stopper.set(poller.stopper()).unwrap();
    keyboard.set(Key::RightAlt, true);
    // The release stops polling, so don't wait for samples.
    keyboard.held.lock().unwrap().clear();
    assert!(wait_for(|| !poller.is_running()));
    keyboard.held.lock().unwrap().insert(Key::RightAlt);
    keyboard.held.lock().unwrap().clear();
    thread::sleep(Duration::from_millis(50));
    assert!(is(&events, &HOLD[1..]));
    poller.join();
}

/// Ownkey's `test_failing_callback_does_not_end_push_to_talk` (`:121`).
#[test]
fn a_panicking_handler_does_not_end_push_to_talk() {
    let (keyboard, events) = (Keyboard::new(1, &[]), recorder());
    let poller = listen(&keyboard, {
        let events = Arc::clone(&events);
        move |event| {
            events.lock().unwrap().push(event);
            if let Pressed(_) = event {
                panic!("microphone vanished");
            }
        }
    });
    for count in [1, 2] {
        keyboard.set(Key::RightAlt, true);
        assert!(wait_for(|| events.lock().unwrap().len() == 2 * count - 1));
        keyboard.set(Key::RightAlt, false);
        assert!(wait_for(|| events.lock().unwrap().len() == 2 * count));
    }
    assert!(is(&events, &TWO_HOLDS));
    poller.join();
}

/// Ownkey #11 test 9 (`:279`) checked that Windows polls every offered key
/// instead of installing a keyboard hook. There is no hook here at all; this
/// checks that every key is sampled on every tick.
#[test]
fn samples_every_offered_key_on_every_tick() {
    let sampled = Arc::new(Mutex::new(Vec::new()));
    let (tx, _rx) = mpsc::channel();
    let mut keys = Key::ALL.to_vec();
    keys.push(Key::RightAlt); // listed twice, polled once
    let poller = HotkeyPoller::spawn_with(
        keys,
        tx,
        {
            let sampled = Arc::clone(&sampled);
            move |key| {
                sampled.lock().unwrap().push(key);
                false
            }
        },
        INTERVAL,
    )
    .unwrap();
    assert!(wait_for(
        || sampled.lock().unwrap().len() >= 3 * Key::ALL.len()
    ));
    poller.join();
    let sampled = sampled.lock().unwrap();
    for tick in sampled.chunks(Key::ALL.len()) {
        assert_eq!(tick, &Key::ALL[..tick.len()]);
    }
}

#[test]
fn a_slow_channel_reader_still_gets_every_change_in_order() {
    let keyboard = Keyboard::new(2, &[]);
    let (tx, rx) = mpsc::channel();
    let poller =
        HotkeyPoller::spawn_with(vec![Key::F13, Key::F14], tx, keyboard.state(), INTERVAL).unwrap();
    // Nobody reads while these happen.
    keyboard.set(Key::F13, true);
    keyboard.set(Key::F14, true);
    keyboard.set(Key::F13, false);
    keyboard.set(Key::F13, true);
    keyboard.set(Key::F14, false);
    keyboard.set(Key::F13, false);
    let events: Vec<HotkeyEvent> = rx.try_iter().collect();
    assert_eq!(
        events,
        [
            Pressed(Key::F13),
            Pressed(Key::F14),
            Released(Key::F13),
            Pressed(Key::F13),
            Released(Key::F14),
            Released(Key::F13),
        ]
    );
    poller.join();
}

/// Work from another thread, such as the app releasing a held key when the screen locks, runs on
/// the handler's thread after the events queued before it: a release that arrives while a slow
/// press is still handled can't overtake that press.
#[test]
fn work_from_another_thread_runs_after_the_events_queued_before_it() {
    let (keyboard, opening) = (Keyboard::new(1, &[]), Arc::new(Latch::new(false)));
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let poller = listen(&keyboard, {
        let (log, opening) = (Arc::clone(&log), Arc::clone(&opening));
        move |event| {
            log.lock().unwrap().push(format!("{event:?}"));
            if event == Pressed(Key::RightAlt) {
                opening.wait();
            }
        }
    });
    keyboard.set(Key::RightAlt, true);
    assert!(wait_for(|| log.lock().unwrap().len() == 1));
    keyboard.set(Key::RightAlt, false);
    // The release is queued while the press is still handled.
    thread::sleep(INTERVAL * 4);
    let done = poller
        .in_order()
        .expect("a handler poller runs work in order")
        .run({
            let log = Arc::clone(&log);
            move || log.lock().unwrap().push("work".into())
        })
        .expect("the poller runs");
    assert_eq!(
        done.recv_timeout(Duration::from_millis(100)),
        Err(RecvTimeoutError::Timeout),
        "the work waits for the press"
    );
    opening.open();
    assert_eq!(done.recv_timeout(PATIENCE), Ok(()));
    assert_eq!(
        *log.lock().unwrap(),
        ["Pressed(RightAlt)", "Released(RightAlt)", "work"]
    );
    poller.join();
}

/// Only a poller with a handler thread runs work, and only until it stops. A handle that is still
/// around doesn't keep the handler thread alive: `join` returns.
#[test]
fn work_runs_only_on_a_running_handler_poller() {
    let keyboard = Keyboard::new(1, &[]);
    let (tx, _rx) = mpsc::channel();
    let channel = HotkeyPoller::spawn_with(vec![Key::F13], tx, keyboard.state(), INTERVAL).unwrap();
    assert!(channel.in_order().is_none(), "no handler thread to run on");
    channel.join();
    let poller = listen(&keyboard, |_| {});
    let in_order = poller.in_order().unwrap();
    let done = in_order.run(|| {}).unwrap();
    assert_eq!(done.recv_timeout(PATIENCE), Ok(()));
    poller.join();
    assert!(in_order.run(|| {}).is_none(), "stopped: nothing runs");
}

#[test]
fn the_poller_ends_when_nobody_listens() {
    let keyboard = Keyboard::new(1, &[]);
    let (tx, rx) = mpsc::channel();
    let poller = HotkeyPoller::spawn_with(vec![Key::F13], tx, keyboard.state(), INTERVAL).unwrap();
    drop(rx);
    keyboard.held.lock().unwrap().insert(Key::F13);
    assert!(wait_for(|| !poller.is_running()));
}

#[test]
fn stop_wakes_a_waiting_poller_at_once() {
    let keyboard = Keyboard::new(1, &[]);
    let (tx, _rx) = mpsc::channel();
    let poller = HotkeyPoller::spawn_with(
        vec![Key::F13],
        tx,
        keyboard.state(),
        Duration::from_secs(3600),
    )
    .unwrap();
    poller.stop();
    assert!(wait_for(|| !poller.is_running()));
}

#[test]
fn dropping_the_poller_stops_it() {
    let keyboard = Keyboard::new(1, &[]);
    let (tx, rx) = mpsc::channel();
    let poller = HotkeyPoller::spawn_with(vec![Key::F13], tx, keyboard.state(), INTERVAL).unwrap();
    drop(poller);
    // The polling thread drops its sender when it ends.
    assert_eq!(
        rx.recv_timeout(PATIENCE),
        Err(RecvTimeoutError::Disconnected)
    );
}
