//! Push-to-talk dictation: Ownkey's state machine (ownkey-dictation.md 4-8) on voice-input's
//! hotkey poller.
//!
//! A press opens the microphone (the recorder's tap during a meeting), a release queues the
//! recording, and one worker processes recordings strictly in order: transcribe, clean up,
//! optionally rewrite, then insert. The newest recording owns the pill.

pub mod capture;
pub mod dump;
pub mod errors;
pub mod level;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use voice_core::cleanup::SENTENCE_END;
use voice_core::config::Config;
use voice_core::providers::provider_requires_key;
use voice_core::{cleanup, prompts, RewriteRequest};
use voice_input::{
    HotkeyConfig, HotkeyEvent, HotkeyPoller, InputError, InsertMode, Key, Mode, PushToTalk,
    TextTarget,
};
use voice_recorder::meeting::RecorderFile;

use self::errors::Step;
use self::level::LevelMeter;
use crate::api::{Activity, PillPhase};
use crate::engine::Core;
use crate::meetings::tap::{self, TapEnd};
use crate::permissions::{Permission, PermissionState};
use crate::pill::{self, notices};
use crate::services::{AudioSink, MicLost, MicOpen, MicStream};

/// Ownkey's micro-tap gates (`ownkey.py:3189-3196`): under 800 WAV bytes (378 samples after the
/// 44-byte header), or under 0.10 s without anything heard, nothing is sent anywhere.
const MIN_SAMPLES: usize = (800 - 44) / 2;
const MIN_SAMPLES_WITHOUT_ACTIVITY: usize = 1600;
/// Ownkey waits for the selection captured at the press for this long.
const PRESS_CAPTURE_WAIT: Duration = Duration::from_millis(1500);
const CAPTURE_TIMEOUT: Duration = Duration::from_millis(500);
/// Ownkey's settle times before inserting (`ownkey.py:3235`, `3310`).
const DICTATION_SETTLE: Duration = Duration::from_millis(100);
const REWRITE_SETTLE: Duration = Duration::from_millis(50);
/// The longest an insertion waits for the user to let go of a hotkey.
const DEFER_LIMIT: Duration = Duration::from_secs(300);
const DEFER_POLL: Duration = Duration::from_millis(20);
/// How often a held dictation checks that the meeting's current recorder knows about it.
const MARK_RETRY: Duration = Duration::from_millis(100);
/// How long a session release waits to be handled. Closing a microphone takes milliseconds, but
/// a press whose microphone is still opening is handled first.
const RELEASE_WAIT: Duration = Duration::from_secs(2);

/// A key-down this soon before the release was seen may have come right after it: the poll
/// sees a release up to one interval late.
const TYPED_MARGIN: Duration = voice_input::POLL_INTERVAL;

pub(crate) struct Dictation {
    ptt: Mutex<PushToTalk>,
    stop_key: Mutex<Option<Key>>,
    poller: Mutex<Option<HotkeyPoller>>,
    recording: Mutex<Option<Recording>>,
    next_id: AtomicU64,
    queue: Mutex<Option<Sender<Job>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    meter: Mutex<LevelMeter>,
    mic_error: Mutex<Option<String>>,
    /// Whether this run already asked macOS for the Accessibility grant, so a dialog the user
    /// dismissed doesn't come back at every key press.
    accessibility_asked: AtomicBool,
}

struct Recording {
    id: u64,
    /// The key that started it, and when the press was seen.
    key: Key,
    pressed: Instant,
    /// Voice posted Cmd+C or Ctrl+C during the hold, for the selection.
    posted_keys: bool,
    mode: Mode,
    cfg: Arc<Config>,
    stream: Option<Box<dyn MicStream>>,
    captured: Arc<Mutex<Captured>>,
    selection: Arc<Selection>,
    /// The meeting recorder that was told about this dictation; `end` goes there.
    marked: Option<RecorderFile>,
}

struct Captured {
    pcm: Vec<i16>,
    heard: bool,
    armed_at: Option<Instant>,
    no_audio: bool,
    meter: LevelMeter,
    last_push: Option<Instant>,
    closed: bool,
    /// How often Voice's own microphone was opened again after it stopped mid-dictation.
    reopens: u32,
    /// Which open of this dictation's capture this is: 1 from the press, so a capture opened
    /// after a lost stream or an ended tap logs as reopen 1. Only the capture's log line reads it.
    mic_opens: u32,
}

struct Job {
    id: u64,
    mode: Mode,
    /// How long the key was held, for the outcome line.
    hold: Duration,
    cfg: Arc<Config>,
    pcm: Vec<i16>,
    heard: bool,
    selection: Arc<Selection>,
}

/// The selection copied at the press, for keys that can copy while held. A copy that failed keeps
/// its error, so a missing Accessibility grant is reported as itself rather than as an empty
/// selection.
struct Selection {
    result: Mutex<Option<Result<Option<String>, InputError>>>,
    done: Condvar,
}

impl Selection {
    fn ready(result: Result<Option<String>, InputError>) -> Arc<Selection> {
        Arc::new(Selection {
            result: Mutex::new(Some(result)),
            done: Condvar::new(),
        })
    }

    fn pending() -> Arc<Selection> {
        Arc::new(Selection {
            result: Mutex::new(None),
            done: Condvar::new(),
        })
    }

    fn set(&self, result: Result<Option<String>, InputError>) {
        *lock(&self.result) = Some(result);
        self.done.notify_all();
    }

    /// The copy's answer, or `None` when it hadn't finished in time.
    fn wait(&self, timeout: Duration) -> Option<Result<Option<String>, InputError>> {
        let guard = lock(&self.result);
        let (guard, _) = self
            .done
            .wait_timeout_while(guard, timeout, |r| r.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    }
}

/// The keys from Settings. An unknown dictation key falls back to the default.
pub fn hotkeys(cfg: &Config, dev_mode: bool) -> (HotkeyConfig, Option<Key>) {
    let fallback = if dev_mode && !cfg!(target_os = "macos") {
        Key::F13
    } else {
        Key::RightAlt
    };
    let dictate = Key::parse(&cfg.hotkey).unwrap_or(fallback);
    let rewrite = Key::parse(&cfg.rewrite_hotkey).filter(|k| *k != dictate);
    let stop = Key::parse(&cfg.meetings_stop_key).filter(|k| *k != dictate && Some(*k) != rewrite);
    (HotkeyConfig { dictate, rewrite }, stop)
}

/// What a finished recording shows.
enum Outcome {
    Inserted {
        words: u32,
        chars: usize,
        method: InsertMode,
        no_text_box: bool,
    },
    Rewrote {
        chars: usize,
        method: InsertMode,
    },
    /// Nothing to insert. The pill says nothing was heard for all of these; which of them it was
    /// only reaches the log, so a missing insertion can be told from a silent one.
    NothingHeard(Nothing),
    Failed {
        notice: &'static str,
        title: &'static str,
        body: String,
    },
    /// Voice is quitting: nothing more happens, nothing is inserted.
    Quiet,
}

/// Why a recording had no text to insert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nothing {
    /// No audio arrived, or none with sound in it in a clip too short to send.
    NoAudio,
    /// An audible clip under the micro-tap gate: under 0.10 s of audio.
    TooShort,
    /// The transcript was empty after cleanup.
    EmptyTranscript,
}

impl Nothing {
    fn ending(self) -> Ending {
        match self {
            Nothing::NoAudio => Ending::NothingHeard,
            Nothing::TooShort => Ending::Dropped(Drop::TooShort),
            Nothing::EmptyTranscript => Ending::EmptyTranscript,
        }
    }
}

/// Why a recording was dropped rather than sent anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drop {
    /// A key that isn't the hotkey went down during the hold: the user typed.
    AnotherKey,
    /// The session went away under the held key: lock, user switch, sleep or session resign.
    Session,
    /// There was less than 0.10 s of audio to send.
    TooShort,
    /// Voice is quitting.
    Cancelled,
}

impl Drop {
    fn description(self) -> &'static str {
        match self {
            Drop::AnotherKey => "another key went down during the hold",
            Drop::Session => "the session went away",
            Drop::TooShort => "too short",
            Drop::Cancelled => "Voice is quitting",
        }
    }
}

/// How a dictation or a rewrite ended, for the one line the log gets for it.
enum Ending {
    Inserted {
        method: InsertMode,
        chars: usize,
    },
    NothingHeard,
    EmptyTranscript,
    Dropped(Drop),
    /// The error code the pill showed, such as `pill.error.auth`.
    Failed(&'static str),
    /// The press was refused before the microphone opened, because this platform's permission to
    /// type into other apps is off: nothing recorded could have been inserted.
    Refused(&'static str),
}

impl Dictation {
    pub fn new(cfg: &Config, dev_mode: bool) -> Dictation {
        let (keys, stop) = hotkeys(cfg, dev_mode);
        Dictation {
            ptt: Mutex::new(PushToTalk::new(keys)),
            stop_key: Mutex::new(stop),
            poller: Mutex::new(None),
            recording: Mutex::new(None),
            next_id: AtomicU64::new(1),
            queue: Mutex::new(None),
            worker: Mutex::new(None),
            meter: Mutex::new(LevelMeter::default()),
            mic_error: Mutex::new(None),
            accessibility_asked: AtomicBool::new(false),
        }
    }

    /// Starts the worker and the hotkey poller. The poller polls every offered key, so a
    /// Settings change never needs a new one (ownkey-dictation.md 3.4).
    pub fn start(core: &Arc<Core>) -> Result<(), String> {
        let this = &core.dictation;
        let (tx, rx) = mpsc::channel();
        *lock(&this.queue) = Some(tx);
        let weak = Arc::downgrade(core);
        let worker = thread::Builder::new()
            .name("dictation".into())
            .spawn(move || work(weak, rx))
            .map_err(|e| e.to_string())?;
        *lock(&this.worker) = Some(worker);

        let weak = Arc::downgrade(core);
        let handler = move |event: HotkeyEvent| {
            if let Some(core) = weak.upgrade() {
                on_hotkey(&core, event);
            }
        };
        let poller = match core.svc.key_state.clone() {
            Some(state) => HotkeyPoller::spawn_handler_with(
                Key::ALL.to_vec(),
                move |key| state(key),
                core.svc.hotkey_interval,
                handler,
            ),
            None => HotkeyPoller::spawn_handler(Key::ALL.to_vec(), handler),
        }
        .map_err(|e| e.to_string())?;
        *lock(&this.poller) = Some(poller);
        Ok(())
    }

    /// New keys from Settings apply from the next event. A recording keeps the keys it
    /// started with.
    pub fn apply(&self, cfg: &Config, dev_mode: bool) {
        let (keys, stop) = hotkeys(cfg, dev_mode);
        lock(&self.ptt).set_config(keys);
        *lock(&self.stop_key) = stop;
    }

    pub fn mic_error(&self) -> Option<String> {
        lock(&self.mic_error).clone()
    }

    /// Stops listening for keys and closes the microphone. Queued recordings are dropped and
    /// nothing is inserted from here on (ownkey-dictation.md 4.8).
    pub fn shutdown(&self) {
        if let Some(poller) = lock(&self.poller).take() {
            poller.stop();
        }
        if let Some(mut recording) = lock(&self.recording).take() {
            recording.stream.take();
            let (pcm, _, _) = take_audio(&mut recording);
            report(
                recording.id,
                recording.mode,
                recording.pressed.elapsed(),
                audio_of(&pcm),
                level::peak_dbfs(&pcm),
                Ending::Dropped(Drop::Cancelled),
            );
        }
        // The worker ends after the job it is on; a cloud request can't be cancelled, so it
        // isn't waited for. It inserts nothing once Voice is quitting.
        lock(&self.queue).take();
        lock(&self.worker).take();
    }
}

fn on_hotkey(core: &Arc<Core>, event: HotkeyEvent) {
    if core.shutting_down() {
        return;
    }
    let this = &core.dictation;
    if let HotkeyEvent::Pressed(key) = event {
        // A locked screen or a session switched out: whoever pressed the key isn't the user at
        // this session, and nothing they say should be recorded, sent or inserted.
        //
        // The poller reads every known key, so only a configured hotkey is worth a line here:
        // at the lock screen every key of a password would otherwise be logged with its time.
        let hotkey = configured_hotkey(lock(&this.ptt).config(), key);
        if !(core.svc.at_console)() {
            if hotkey {
                log::info!("dictation: {key} pressed away from the console; ignored");
            }
            return;
        }
        if *lock(&this.stop_key) == Some(key) {
            core.stop_clicked();
            return;
        }
    }
    // One line per press and release of a configured hotkey, so a check at the
    // keyboard can see which key Voice read. The poller reads every known key,
    // but only these are worth a log line; a right-hand modifier that macOS
    // folds onto the left-hand code shows up here as the right-hand key.
    if lock(&this.ptt).config().mode_for(event.key()).is_some() {
        match event {
            HotkeyEvent::Pressed(key) => log::info!("dictation: {key} pressed"),
            HotkeyEvent::Released(key) => log::info!("dictation: {key} released"),
        }
    }
    let action = lock(&this.ptt).handle(event);
    match action {
        Some(voice_input::Action::Start(mode)) => start_recording(core, mode, event.key()),
        Some(voice_input::Action::Stop) => stop_recording(core),
        None => {}
    }
}

/// Whether a key press is one of the configured hotkeys: the only presses worth a line while
/// nobody is at the console. The poller reads every key it knows, so logging the others would
/// record, for example, each key of a password typed at the lock screen.
fn configured_hotkey(keys: HotkeyConfig, key: Key) -> bool {
    keys.mode_for(key).is_some()
}

/// The key-up can be lost when the session goes away: a fast user switch, the
/// lock screen, or the Mac sleeping. The app hears those from the system and
/// calls this, which drops the held recording: the person who held the key isn't
/// at this console any more, so nothing they said is transcribed, sent to a
/// provider or inserted, and the clipboard isn't touched. A key-up that arrives
/// later changes nothing.
///
/// The release runs on the hotkey handler's thread, after the presses and
/// releases already queued there, so push-to-talk changes on one thread only.
/// A release that came in while a press was still opening the microphone
/// therefore drops the recording that press stored, instead of finding nothing
/// to stop and leaving the microphone on. Returns once the release was
/// handled, or after [`RELEASE_WAIT`].
pub(crate) fn release_all(core: &Arc<Core>) {
    if core.shutting_down() {
        return;
    }
    // The memory behind a held right-hand modifier has to let go with the key:
    // that is what makes a wipe of the flags word survivable, and after sleep
    // or a session switch a key that was released then must not read down for
    // as long as its pair's shared code does.
    voice_input::forget_held_modifiers();
    let in_order = lock(&core.dictation.poller)
        .as_ref()
        .and_then(HotkeyPoller::in_order);
    let Some(in_order) = in_order else {
        // No poller, so no press can be in flight.
        release_now(core);
        return;
    };
    let weak = Arc::downgrade(core);
    let queued = in_order.run(move || {
        if let Some(core) = weak.upgrade() {
            release_now(&core);
        }
    });
    if let Some(done) = queued {
        let _ = done.recv_timeout(RELEASE_WAIT);
    }
}

/// [`release_all`] on the hotkey handler's thread. Unlike a key-up, this ends the recording
/// without a word: the session went away, so there is nobody at this console to insert it for.
fn release_now(core: &Arc<Core>) {
    if core.shutting_down() {
        return;
    }
    // Push-to-talk lets go of its key; whether a recording was running is what matters here.
    if !lock(&core.dictation.ptt).release_all() {
        return;
    }
    let Some(mut recording) = lock(&core.dictation.recording).take() else {
        return;
    };
    let (pcm, _) = close_recording(core, &mut recording);
    let held = recording.pressed.elapsed();
    drop_recording(
        core,
        recording.id,
        recording.mode,
        held,
        &pcm,
        Drop::Session,
    );
}

fn start_recording(core: &Arc<Core>, mode: Mode, key: Key) {
    let this = &core.dictation;
    let cfg = core.config();
    let id = this.next_id.fetch_add(1, Ordering::SeqCst);
    let pressed = Instant::now();
    // A dictation or a rewrite that couldn't be inserted is not worth recording, and a user who
    // talks into one only learns about the missing grant after the transcription. This is macOS's
    // Accessibility grant; Windows and Linux report no such permission.
    if accessibility_missing(core) {
        refuse_accessibility(core, id, mode, pressed);
        return;
    }
    // The local model loads while the user speaks (ownkey-dictation.md 4.2, step 3).
    if cfg.audio_provider == "orukeet" {
        core.svc.speech.local_preload();
    }
    let posted_keys = mode == Mode::Rewrite && key.can_copy_while_held();
    let selection = if posted_keys {
        let pending = Selection::pending();
        let (input, slot) = (Arc::clone(&core.svc.input), Arc::clone(&pending));
        let _ = thread::Builder::new()
            .name("selection".into())
            .spawn(move || {
                // The rewrite key is down on purpose here; it doesn't change the copy chord.
                let copied = input.capture_selection(CAPTURE_TIMEOUT, &mut || Ok(()));
                slot.set(copied)
            });
        pending
    } else {
        // A key that would add a modifier to the copy, such as Right Alt or, on a Mac, Right
        // Control: copy after the release.
        Selection::ready(Ok(None))
    };
    let activity = activity_of(mode);
    // Whether the focused app takes text comes after the microphone opens: on a Mac it is two
    // Accessibility requests, and a busy app would hold up the first words.
    core.pill.update(|p, _| p.press(id, activity, false));
    core.meetings.set_dictating(true);

    let captured = Arc::new(Mutex::new(Captured {
        pcm: Vec::new(),
        heard: false,
        armed_at: None,
        no_audio: false,
        meter: lock(&this.meter).restart(),
        last_push: None,
        closed: false,
        reopens: 0,
        mic_opens: 1,
    }));
    let make_sink = || audio_sink(Arc::downgrade(core), id, Arc::clone(&captured));
    let on_end = tap_ended(core, id, Arc::clone(&captured));
    let opened = match core.meetings.open_tap(make_sink(), on_end) {
        Some(Ok(stream)) => {
            log::info!("{}", tap_line(id));
            Ok(stream)
        }
        // The recorder's tap failed: dictation still works on its own capture.
        Some(Err(_)) | None => core.svc.microphone.open_watched(
            make_sink(),
            mic_lost(core, id, Arc::clone(&captured)),
            MicOpen::first(id),
        ),
    };
    match opened {
        Ok(stream) => {
            let recovered = lock(&this.mic_error).take().is_some();
            *lock(&this.recording) = Some(Recording {
                id,
                key,
                pressed,
                posted_keys,
                mode,
                cfg,
                stream: Some(stream),
                captured,
                selection,
                marked: None,
            });
            mark(core, id);
            watch(core, id, key);
            read_target(core, id);
            if recovered {
                // Settings shows "no microphone" until it hears otherwise.
                core.settings_changed();
            }
        }
        Err(message) => {
            let first_failure = lock(&this.mic_error).replace(message.clone()).is_none();
            lock(&this.ptt).recording_ended();
            core.meetings.set_dictating(false);
            core.pill.update(|p, now| {
                p.finished(
                    id,
                    PillPhase::Error,
                    Some(pill::notice(errors::MIC)),
                    None,
                    pill::ERROR_HOLD,
                    now,
                )
            });
            core.notify(
                errors::TITLE_DICTATION,
                format!("Microphone error: {message}"),
            );
            if first_failure {
                core.settings_changed();
            }
            report(
                id,
                mode,
                pressed.elapsed(),
                Duration::ZERO,
                level::SILENT_DBFS,
                Ending::Failed(errors::MIC),
            );
        }
    }
}

/// Whether this computer has a permission to type into other apps and it is off. macOS reports
/// the Accessibility grant as denied until the user switches it on; Windows and Linux have no
/// such permission and answer `None`, so nothing changes there.
fn accessibility_missing(core: &Core) -> bool {
    core.svc.permissions.state(Permission::Accessibility) == Some(PermissionState::Denied)
}

/// A press whose text could never be inserted: macOS's Accessibility grant is off, so Voice
/// doesn't open the microphone at all. The pill says what to switch on straight away, the first
/// such press of this run asks macOS for the grant (its own dialog, once), and the log gets the
/// one line every recording gets.
fn refuse_accessibility(core: &Arc<Core>, id: u64, mode: Mode, pressed: Instant) {
    if !core
        .dictation
        .accessibility_asked
        .swap(true, Ordering::SeqCst)
    {
        core.svc.permissions.request(Permission::Accessibility);
    }
    let activity = activity_of(mode);
    core.pill.update(|p, now| {
        p.press(id, activity, false);
        p.finished(
            id,
            PillPhase::Error,
            Some(pill::notice(errors::ACCESSIBILITY)),
            None,
            pill::ERROR_HOLD,
            now,
        );
    });
    // Nothing started, so the key-up has nothing to stop; the debounce still lasts until it comes.
    lock(&core.dictation.ptt).recording_ended();
    report(
        id,
        mode,
        pressed.elapsed(),
        Duration::ZERO,
        level::SILENT_DBFS,
        Ending::Refused(errors::ACCESSIBILITY),
    );
}

/// Tells the meeting that records the microphone that this dictation is on it (decision 9), so
/// Rukoo's agent knows these words weren't said in the call. That holds whatever the dictation
/// reads from: the recorder's tap, or Voice's own capture when the tap failed or the recorder
/// restarts. Until the key is up, a thread checks which recorder the dictation told and tells
/// each one that takes over after a crash: the new session ends the dead one's dictation itself
/// (RECORDING.md), and on Voice's own capture no tap ends to say the recorder changed.
fn mark(core: &Arc<Core>, id: u64) {
    if try_mark(core, id) != Marking::Watch {
        return;
    }
    let weak = Arc::downgrade(core);
    let _ = thread::Builder::new()
        .name("dictation-mark".into())
        .spawn(move || loop {
            thread::sleep(MARK_RETRY);
            let Some(core) = weak.upgrade() else {
                return;
            };
            if core.shutting_down() || try_mark(&core, id) != Marking::Watch {
                return;
            }
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Marking {
    /// Nothing more to mark: no meeting records the microphone, or the key is up.
    Done,
    /// A meeting records the microphone: its recorder may not answer yet, or may restart before
    /// the key is up.
    Watch,
}

fn try_mark(core: &Core, id: u64) -> Marking {
    let this = &core.dictation;
    let marked = match lock(&this.recording).as_ref().filter(|r| r.id == id) {
        Some(r) => r.marked.clone(),
        None => return Marking::Done,
    };
    let watch = || {
        if core.meetings.records_mic() {
            Marking::Watch
        } else {
            Marking::Done
        }
    };
    // `None` while the recorder restarts or stops, and without a meeting that records the mic.
    let Some(file) = core.meetings.controller.tap() else {
        return watch();
    };
    if marked.as_ref() == Some(&file) {
        return Marking::Watch;
    }
    // Not told yet, or told a recorder that has died since.
    if tap::dictation(&file, "start").is_err() {
        // It died a moment ago; the watchdog restarts it or ends the meeting.
        return watch();
    }
    let mut recording = lock(&this.recording);
    match recording.as_mut().filter(|r| r.id == id) {
        Some(r) => {
            r.marked = Some(file);
            Marking::Watch
        }
        None => {
            // Let go while the recorder answered: the stretch ends right away.
            drop(recording);
            let _ = tap::dictation(&file, "end");
            Marking::Done
        }
    }
}

/// Runs when the recorder ends the tap while the key may still be held: the meeting stopped or
/// its recorder died. The rest of the dictation comes from Voice's own capture into the same
/// buffer, so nothing said after that is lost, and a restarted recorder is told about it again
/// by the thread [`mark`] started at the press.
fn tap_ended(core: &Arc<Core>, id: u64, captured: Arc<Mutex<Captured>>) -> TapEnd {
    let weak = Arc::downgrade(core);
    Box::new(move || {
        let Some(core) = weak.upgrade() else {
            return;
        };
        if core.shutting_down() {
            return;
        }
        let this = &core.dictation;
        match lock(&this.recording).as_mut().filter(|r| r.id == id) {
            // The stretch ended with that recorder.
            Some(r) => r.marked = None,
            None => return,
        }
        // The tap is closed from its own reader thread.
        match reopen(&core, id, captured) {
            Ok(()) => {
                // A recorder that lost its device but lives on is told at once.
                try_mark(&core, id);
            }
            Err(message) => {
                // Nothing arrives any more: "No audio detected" until the key is up.
                core.pill.update(|p, _| p.no_audio(id, true));
                core.notify(
                    errors::TITLE_DICTATION,
                    format!(
                        "The meeting's microphone ended and Voice's own didn't open: {message}"
                    ),
                );
            }
        }
    })
}

/// Opens Voice's own microphone into recording `id`'s buffer and puts it in place of the stream
/// that ended, which is dropped here.
fn reopen(core: &Arc<Core>, id: u64, captured: Arc<Mutex<Captured>>) -> Result<(), String> {
    // The opens number themselves over the whole dictation, whether the tap or a lost stream
    // ended the one before, so the capture's lines read in the order they happened.
    let about = {
        let mut c = lock(&captured);
        c.mic_opens += 1;
        MicOpen::reopen(id, c.mic_opens - 1)
    };
    let sink = audio_sink(Arc::downgrade(core), id, Arc::clone(&captured));
    let stream = core
        .svc
        .microphone
        .open_watched(sink, mic_lost(core, id, captured), about)?;
    let old = match lock(&core.dictation.recording)
        .as_mut()
        .filter(|r| r.id == id)
    {
        Some(r) => r.stream.replace(stream),
        // Let go meanwhile: the new capture closes again.
        None => Some(stream),
    };
    drop(old);
    Ok(())
}

/// How often one dictation opens Voice's own microphone again before it gives up.
const MIC_REOPENS: u32 = 3;

/// Runs when Voice's own microphone stops while the key may still be held: the device went away,
/// or changed its format, as a Bluetooth headset or a new default microphone does. The rest of
/// the dictation comes from the default microphone, opened again into the same buffer, as
/// [`tap_ended`] does for the meeting's. When that fails, the pill says no audio is coming in.
fn mic_lost(core: &Arc<Core>, id: u64, captured: Arc<Mutex<Captured>>) -> MicLost {
    let weak = Arc::downgrade(core);
    Box::new(move || {
        let Some(core) = weak.upgrade() else {
            return;
        };
        if core.shutting_down() || !holds(&core, id) {
            return;
        }
        let attempt = {
            let mut c = lock(&captured);
            c.reopens += 1;
            c.reopens
        };
        let result = if attempt > MIC_REOPENS {
            Err("it stopped again".to_string())
        } else {
            reopen(&core, id, captured)
        };
        if let Err(message) = result {
            core.pill.update(|p, _| p.no_audio(id, true));
            core.notify(
                errors::TITLE_DICTATION,
                format!(
                    "The microphone stopped during the dictation and didn't open again: {message}"
                ),
            );
        }
    })
}

/// What the microphone calls with every block.
fn audio_sink(core: Weak<Core>, id: u64, captured: Arc<Mutex<Captured>>) -> AudioSink {
    Box::new(move |block: &[i16]| {
        let Some(core) = core.upgrade() else {
            return;
        };
        let now = Instant::now();
        let mut c = lock(&captured);
        if c.closed {
            return;
        }
        let first = c.armed_at.is_none();
        let armed_at = *c.armed_at.get_or_insert(now);
        c.pcm.extend_from_slice(block);
        let (level, heard) = c.meter.feed(block);
        let mut silent = None;
        if heard && !c.heard {
            c.heard = true;
            if c.no_audio {
                c.no_audio = false;
                silent = Some(false);
            }
        }
        if !c.heard
            && !c.no_audio
            && level < level::ACTIVITY_LEVEL
            && now.saturating_duration_since(armed_at) >= level::NO_AUDIO_AFTER
        {
            c.no_audio = true;
            silent = Some(true);
        }
        let push = c
            .last_push
            .is_none_or(|at| now.saturating_duration_since(at) >= level::PUSH_INTERVAL);
        if push {
            c.last_push = Some(now);
        }
        drop(c);
        if first {
            core.pill.update(|p, _| p.listening(id));
            core.play_chime();
        }
        if let Some(silent) = silent {
            core.pill.update(|p, _| p.no_audio(id, silent));
        }
        if push && core.pill.read(|p| p.listening_to(id)) {
            core.sink.level(level);
        }
    })
}

/// Reads whether the focused app takes text on a thread of its own, so a slow app holds up
/// neither the recording nor its release, and shows the warning while the key is held.
fn read_target(core: &Arc<Core>, id: u64) {
    let weak = Arc::downgrade(core);
    let _ = thread::Builder::new()
        .name("dictation-target".into())
        .spawn(move || {
            let Some(core) = weak.upgrade() else {
                return;
            };
            let warning = core.svc.input.text_target() == TextTarget::NotSelected;
            core.pill.update(|p, _| p.target_warning(id, warning));
        });
}

/// Why the watchdog stopped a recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overrun {
    /// The key read up twice in a row, a watchdog interval apart, and no release stopped it.
    KeyUp,
    /// It ran for [`crate::services::DictationTiming::longest`].
    TooLong,
}

/// The last line of defense for "a missed key-up must never leave recording on" (Ownkey #11).
/// While the recording exists, a thread reads its key every watchdog interval and stops the
/// recording when the key reads up twice in a row, or when it has run for the longest time a
/// recording may take. A release seen by the poller stops it within one poll, so the watchdog
/// acts only when that release was lost or its handler is stuck, or when a key reads down for
/// ten minutes, such as a key-down another app posted without its key-up.
fn watch(core: &Arc<Core>, id: u64, key: Key) {
    let weak = Arc::downgrade(core);
    let timing = core.svc.dictation_timing;
    let started = Instant::now();
    let _ = thread::Builder::new()
        .name("dictation-watchdog".into())
        .spawn(move || {
            let mut up = 0;
            loop {
                thread::sleep(timing.watchdog);
                let Some(core) = weak.upgrade() else {
                    return;
                };
                if core.shutting_down() || !holds(&core, id) {
                    return;
                }
                up = if core.key_down(key) { 0 } else { up + 1 };
                let overrun = if up >= 2 {
                    Some(Overrun::KeyUp)
                } else if started.elapsed() >= timing.longest {
                    Some(Overrun::TooLong)
                } else {
                    None
                };
                if let Some(overrun) = overrun {
                    stop_overrun(&core, id, overrun);
                    return;
                }
            }
        });
}

/// Whether recording `id` is still the one running.
fn holds(core: &Core, id: u64) -> bool {
    lock(&core.dictation.recording)
        .as_ref()
        .is_some_and(|r| r.id == id)
}

/// Stops recording `id` from the watchdog, as a release would. Push-to-talk lets go of its key
/// under the recording's lock, so a release the handler thread works on at the same time finds
/// nothing more to stop, and a newer recording is never stopped instead.
fn stop_overrun(core: &Arc<Core>, id: u64, overrun: Overrun) {
    let this = &core.dictation;
    let recording = {
        let mut slot = lock(&this.recording);
        if slot.as_ref().map(|r| r.id) != Some(id) {
            return;
        }
        // Lock order: `recording`, then `ptt`; nothing takes them the other way round.
        lock(&this.ptt).release_all();
        slot.take()
    };
    let Some(recording) = recording else {
        return;
    };
    match overrun {
        Overrun::KeyUp => log::warn!("dictation: the key read up but no release came; stopped"),
        Overrun::TooLong => {
            log::warn!("dictation: stopped at the longest recording");
            core.notify(
                errors::TITLE_DICTATION,
                format!(
                    "Dictation stopped after {} minutes, the longest one recording can be.",
                    core.svc.dictation_timing.longest.as_secs().div_ceil(60)
                ),
            );
        }
    }
    end_recording(core, recording);
}

fn stop_recording(core: &Arc<Core>) {
    let Some(recording) = lock(&core.dictation.recording).take() else {
        return;
    };
    end_recording(core, recording);
}

/// Closes a recording taken out of its slot and queues it for the worker.
fn end_recording(core: &Arc<Core>, mut recording: Recording) {
    let this = &core.dictation;
    let (pcm, heard) = close_recording(core, &mut recording);
    let held = recording.pressed.elapsed();
    if !recording.posted_keys && typed_during(recording.key, held, (core.svc.key_activity)()) {
        // Right Option with a letter types é or @, and Right Command with one is a shortcut:
        // the key was held to type, not to talk.
        drop_recording(
            core,
            recording.id,
            recording.mode,
            held,
            &pcm,
            Drop::AnotherKey,
        );
        return;
    }
    let phase = match recording.mode {
        Mode::Dictate => PillPhase::Transcribing,
        Mode::Rewrite => PillPhase::Rewriting,
    };
    let activity = activity_of(recording.mode);
    core.pill
        .update(|p, _| p.working(recording.id, phase, activity, None));
    let audio = audio_of(&pcm);
    let peak = level::peak_dbfs(&pcm);
    let job = Job {
        id: recording.id,
        mode: recording.mode,
        hold: held,
        cfg: recording.cfg,
        pcm,
        heard,
        selection: recording.selection,
    };
    let sent = lock(&this.queue).as_ref().map(|queue| queue.send(job));
    if sent.is_none_or(|sent| sent.is_err()) {
        // Voice is quitting: the worker is gone and nothing will process this recording.
        report(
            recording.id,
            recording.mode,
            held,
            audio,
            peak,
            Ending::Dropped(Drop::Cancelled),
        );
    }
}

/// Closes the microphone and the meeting's stretch of a recording taken out of its slot, and
/// takes its audio. Closing the stream first means no audio arrives after the release is handled.
fn close_recording(core: &Core, recording: &mut Recording) -> (Vec<i16>, bool) {
    drop(recording.stream.take());
    if let Some(file) = recording.marked.take() {
        let _ = tap::dictation(&file, "end");
    }
    core.meetings.set_dictating(false);
    let (pcm, heard, meter) = take_audio(recording);
    // Keeps the meter calibrated to this microphone for the next press.
    *lock(&core.dictation.meter) = meter;
    (pcm, heard)
}

/// Takes a recording's audio, closing its capture against further blocks.
fn take_audio(recording: &mut Recording) -> (Vec<i16>, bool, LevelMeter) {
    let mut c = lock(&recording.captured);
    c.closed = true;
    (std::mem::take(&mut c.pcm), c.heard, c.meter.clone())
}

/// Drops a recording without sending it anywhere: the pill goes and the log says why. Used when
/// another key went down during the hold, and when the session went away under it.
fn drop_recording(core: &Core, id: u64, mode: Mode, held: Duration, pcm: &[i16], why: Drop) {
    core.pill.update(|p, _| p.dropped(id));
    report(
        id,
        mode,
        held,
        audio_of(pcm),
        level::peak_dbfs(pcm),
        Ending::Dropped(why),
    );
}

/// The one info line a dictation or a rewrite leaves in the log when it ends, so "nothing heard",
/// an empty transcript, a dropped hold and a failed insert can be told apart. It holds counts,
/// durations, the clip's peak level and error codes only: never the text, the selection or any
/// part of them.
fn report(id: u64, mode: Mode, hold: Duration, audio: Duration, peak: f32, ending: Ending) {
    log::info!(
        "dictation: {}",
        outcome_line(id, mode, hold, audio, peak, &ending)
    );
}

fn outcome_line(
    id: u64,
    mode: Mode,
    hold: Duration,
    audio: Duration,
    peak: f32,
    ending: &Ending,
) -> String {
    let what = match mode {
        Mode::Dictate => "dictation",
        Mode::Rewrite => "rewrite",
    };
    let how = match ending {
        Ending::Inserted { method, chars } => {
            let method = match method {
                InsertMode::Paste => "pasting",
                InsertMode::Type => "typing",
            };
            format!("inserted {chars} characters by {method}")
        }
        Ending::NothingHeard => "nothing heard".to_string(),
        Ending::EmptyTranscript => "empty transcript".to_string(),
        Ending::Dropped(why) => format!("dropped ({})", why.description()),
        Ending::Failed(code) => format!("failed ({code})"),
        Ending::Refused(code) => format!("not recorded ({code})"),
    };
    format!(
        "{what} {id} ended: {how} ({:.1} s held, {:.1} s of audio, peak {peak:.0} dBFS)",
        hold.as_secs_f64(),
        audio.as_secs_f64()
    )
}

/// How much audio a recording holds. Every capture delivers 16 kHz mono samples.
fn audio_of(pcm: &[i16]) -> Duration {
    Duration::from_secs_f64(pcm.len() as f64 / f64::from(voice_audio::SAMPLE_RATE))
}

/// The one line a dictation logs when it takes the meeting recorder's tap instead of opening a
/// device of its own, so a reader can tell that path from a capture that opened a headset. The
/// tap delivers 16 kHz mono already, so there is nothing to convert.
fn tap_line(id: u64) -> String {
    format!(
        "dictation {id} capture open (meeting tap): the recorder's microphone tap at {} Hz",
        voice_audio::SAMPLE_RATE
    )
}

/// Whether a key that isn't a modifier went down while `key` was held for `held`, from how long
/// ago the last one went down. Only a modifier hotkey can be part of a chord that types or runs a
/// shortcut; an F key's own press and its repeats are key-downs themselves. A key-down within
/// [`TYPED_MARGIN`] of the release may have followed it, so it doesn't count.
fn typed_during(key: Key, held: Duration, since_key_down: Option<Duration>) -> bool {
    let modifier = matches!(
        key,
        Key::RightAlt | Key::RightCommand | Key::RightCtrl | Key::RightShift
    );
    modifier && since_key_down.is_some_and(|since| since > TYPED_MARGIN && since < held)
}

fn work(core: Weak<Core>, jobs: Receiver<Job>) {
    for job in jobs {
        let Some(core) = core.upgrade() else {
            return;
        };
        // A job queued just before Voice began quitting still gets its outcome line, so no
        // recording ends without a word in the log.
        let outcome = if core.shutting_down() {
            Outcome::Quiet
        } else {
            process(&core, &job)
        };
        finish(&core, &job, outcome);
    }
}

fn finish(core: &Core, job: &Job, outcome: Outcome) {
    let id = job.id;
    let ending = ending_of(&outcome);
    let nothing = matches!(outcome, Outcome::NothingHeard(_));
    match outcome {
        Outcome::Inserted {
            words, no_text_box, ..
        } => core.pill.update(|p, now| {
            if no_text_box {
                p.finished(
                    id,
                    PillPhase::Warning,
                    Some(pill::notice(notices::SELECT_TEXT_BOX)),
                    None,
                    pill::WARNING_HOLD,
                    now,
                );
            } else {
                p.finished(
                    id,
                    PillPhase::Inserted,
                    None,
                    Some(words),
                    pill::INSERTED_HOLD,
                    now,
                );
            }
        }),
        Outcome::Rewrote { .. } => core.pill.update(|p, now| {
            p.finished(id, PillPhase::Rewrote, None, None, pill::INSERTED_HOLD, now)
        }),
        // Differs from Ownkey on purpose: an empty capture showed the green "Done" check.
        Outcome::NothingHeard(_) => core.pill.update(|p, now| {
            p.finished(
                id,
                PillPhase::NothingHeard,
                None,
                None,
                pill::NOTHING_HEARD_HOLD,
                now,
            )
        }),
        Outcome::Failed {
            notice,
            title,
            body,
        } => {
            core.pill.update(|p, now| {
                p.finished(
                    id,
                    PillPhase::Error,
                    Some(pill::notice(notice)),
                    None,
                    pill::ERROR_HOLD,
                    now,
                )
            });
            core.notify(title, body);
        }
        Outcome::Quiet => {}
    }
    if nothing {
        keep_audio(core, job);
    }
    report(
        id,
        job.mode,
        job.hold,
        audio_of(&job.pcm),
        level::peak_dbfs(&job.pcm),
        ending,
    );
}

/// With the development switch on, keeps the audio of a recording that inserted nothing, so a
/// miss can be looked into. Off in every normal run, and compiled out of a release build; see
/// [`dump`] for what the file holds and how private it is.
fn keep_audio(core: &Core, job: &Job) {
    if !core.opts.dump_empty_dictation {
        return;
    }
    match dump::save(&core.opts.data_dir, job.id, &job.pcm) {
        Ok(path) => log::warn!(
            "dictation: kept the audio of {} in {}",
            job.id,
            path.display()
        ),
        Err(error) => log::warn!("dictation: couldn't keep the audio of {}: {error}", job.id),
    }
}

/// How an outcome reads in the log, which is the only place some of them differ.
fn ending_of(outcome: &Outcome) -> Ending {
    match outcome {
        Outcome::Inserted { chars, method, .. } => Ending::Inserted {
            method: *method,
            chars: *chars,
        },
        Outcome::Rewrote { chars, method } => Ending::Inserted {
            method: *method,
            chars: *chars,
        },
        Outcome::NothingHeard(nothing) => nothing.ending(),
        Outcome::Failed { notice, .. } => Ending::Failed(notice),
        Outcome::Quiet => Ending::Dropped(Drop::Cancelled),
    }
}

fn failed(notice: &'static str, title: &'static str, body: impl Into<String>) -> Outcome {
    Outcome::Failed {
        notice,
        title,
        body: body.into(),
    }
}

fn provider_failed(error: &voice_core::ProviderError, step: Step) -> Outcome {
    let title = match step {
        Step::Transcribe => errors::TITLE_DICTATION,
        Step::Rewrite => errors::TITLE_REWRITE,
    };
    failed(
        errors::provider_notice(error),
        title,
        errors::provider_body(error, step),
    )
}

fn process(core: &Core, job: &Job) -> Outcome {
    let samples = job.pcm.len();
    if samples < MIN_SAMPLES {
        // No audio at all is "nothing heard"; an audible blip under the micro-tap gate is a
        // hold too short to send anywhere.
        return Outcome::NothingHeard(if samples == 0 {
            Nothing::NoAudio
        } else {
            Nothing::TooShort
        });
    }
    if !job.heard && samples < MIN_SAMPLES_WITHOUT_ACTIVITY {
        return Outcome::NothingHeard(Nothing::NoAudio);
    }
    let audio = job.cfg.dictation_audio();
    if provider_requires_key(&audio.provider, &audio.endpoint) && audio.api_key.trim().is_empty() {
        return failed(
            errors::NO_KEY,
            errors::TITLE_DICTATION,
            "No audio API key set. Open Settings > Transcription and add a key.",
        );
    }
    match job.mode {
        Mode::Dictate => dictate(core, job),
        Mode::Rewrite => rewrite_selection(core, job),
    }
}

/// Transcribes the recording, showing "Loading model" while Orukeet still loads.
fn transcribe(core: &Core, job: &Job) -> Result<String, Outcome> {
    let audio = job.cfg.dictation_audio();
    let recognizer = core
        .svc
        .speech
        .recognizer(&audio)
        .map_err(|e| provider_failed(&e, Step::Transcribe))?;
    if audio.provider == "orukeet"
        && core.svc.speech.local_status().state != voice_core::local::LocalState::Loaded
    {
        let (phase, activity) = match job.mode {
            Mode::Dictate => (PillPhase::Transcribing, Activity::Dictate),
            Mode::Rewrite => (PillPhase::Rewriting, Activity::Rewrite),
        };
        core.pill.update(|p, _| {
            p.working(
                job.id,
                phase,
                activity,
                Some(pill::notice(notices::LOADING_MODEL)),
            )
        });
    }
    let recognized = recognizer
        .transcribe(&job.pcm, &job.cfg.dictation_hints())
        .map_err(|e| provider_failed(&e, Step::Transcribe))?;
    Ok(recognized.text.trim().to_string())
}

fn dictate(core: &Core, job: &Job) -> Outcome {
    let cfg = &job.cfg;
    let raw = match transcribe(core, job) {
        Ok(text) => text,
        Err(outcome) => return outcome,
    };
    if core.shutting_down() {
        return Outcome::Quiet;
    }
    let mut text = cleanup::clean(&raw, &cfg.cleanup());
    if !text.is_empty() && cfg.auto_rewrite {
        core.pill
            .update(|p, _| p.working(job.id, PillPhase::Rewriting, Activity::Rewrite, None));
        match auto_rewrite(core, cfg, &text) {
            Ok(rewritten) if !rewritten.trim().is_empty() => text = rewritten.trim().to_string(),
            // An empty reply keeps the cleaned transcript (`ownkey.py:1469-1472`).
            Ok(_) => {}
            Err(error) => {
                // Auto-rewrite never blocks insertion: the cleaned transcript goes in.
                let body = match error.kind {
                    voice_core::ErrorKind::Auth | voice_core::ErrorKind::Provider => {
                        format!("{} Inserted the raw transcript.", error.message)
                    }
                    _ => "Rewrite failed. Inserted the raw transcript instead.".to_string(),
                };
                core.notify(errors::TITLE_REWRITE, body);
            }
        }
    }
    if text.is_empty() {
        return Outcome::NothingHeard(Nothing::EmptyTranscript);
    }
    thread::sleep(DICTATION_SETTLE);
    if core.shutting_down() {
        return Outcome::Quiet;
    }
    let no_text_box = core.svc.input.text_target() == TextTarget::NotSelected;
    if no_text_box {
        // Ownkey inserts anyway: many apps' text fields can't be detected.
        core.notify(
            errors::TITLE_NO_TEXT_BOX,
            "No text box selected. Click a text field and try again.".to_string(),
        );
    }
    let words = text.split_whitespace().count() as u32;
    let inserted = inserted_text(&text);
    let chars = inserted.chars().count();
    if let Err(outcome) = insert(core, cfg, &inserted) {
        return outcome;
    }
    Outcome::Inserted {
        words,
        chars,
        method: insertion_mode(cfg),
        no_text_box,
    }
}

fn auto_rewrite(
    core: &Core,
    cfg: &Config,
    text: &str,
) -> Result<String, voice_core::ProviderError> {
    let rewriter = core.svc.speech.rewriter(&cfg.rewrite())?;
    rewriter.rewrite(&RewriteRequest {
        system: prompts::dictation_rewrite_prompt_for(cfg),
        user: text.to_string(),
        max_tokens: None,
    })
}

fn rewrite_selection(core: &Core, job: &Job) -> Outcome {
    let cfg = &job.cfg;
    // The copy taken at the press, or a second try once the key is up. A copy that failed keeps its
    // error: with macOS's Accessibility grant missing Voice can neither read the selection nor copy
    // it, and "nothing selected" would send the user looking for a selection that is there.
    let capture = match job.selection.wait(PRESS_CAPTURE_WAIT) {
        Some(Ok(selection)) if !selection.as_deref().is_none_or(|s| s.trim().is_empty()) => {
            Ok(selection)
        }
        Some(Err(error)) => Err(error),
        _ => {
            // This recording's key is up, but the next one may be down by the time Ctrl+C goes out.
            let held = hotkeys_of(core, cfg);
            let mut released = || wait_for_release(core, &held);
            core.svc
                .input
                .capture_selection(CAPTURE_TIMEOUT, &mut released)
        }
    };
    if core.shutting_down() {
        return Outcome::Quiet;
    }
    let selection = match capture {
        Ok(Some(selection)) if !selection.trim().is_empty() => selection,
        Ok(_) => {
            return failed(
                errors::SELECT_TEXT,
                errors::TITLE_REWRITE,
                "Nothing selected. Highlight text, then hold the rewrite key and say what should change.",
            )
        }
        Err(error) => {
            return failed(
                errors::input_notice(&error),
                errors::TITLE_REWRITE,
                error.to_string(),
            )
        }
    };
    if prompts::selection_too_long(&selection) {
        return failed(
            errors::SELECTION_TOO_LONG,
            errors::TITLE_REWRITE,
            "The selection is too long to rewrite.",
        );
    }
    let instruction = match transcribe(core, job) {
        Ok(text) => text,
        Err(outcome) => return outcome,
    };
    if core.shutting_down() {
        return Outcome::Quiet;
    }
    if instruction.is_empty() {
        return Outcome::NothingHeard(Nothing::EmptyTranscript);
    }
    core.pill
        .update(|p, _| p.working(job.id, PillPhase::Rewriting, Activity::Rewrite, None));
    let rewrite = cfg.rewrite();
    if provider_requires_key(&rewrite.provider, &rewrite.endpoint)
        && rewrite.api_key.trim().is_empty()
    {
        return failed(
            errors::NO_KEY,
            errors::TITLE_REWRITE,
            "No rewrite API key set. Open Settings > Rewriting and add a key.",
        );
    }
    let result = core.svc.speech.rewriter(&rewrite).and_then(|rewriter| {
        rewriter.rewrite(&RewriteRequest {
            system: prompts::REWRITE_SELECTION_PROMPT.to_string(),
            user: prompts::selection_user_message(&instruction, &selection),
            max_tokens: None,
        })
    });
    let result = match result {
        Ok(text) => text,
        Err(e) => return provider_failed(&e, Step::Rewrite),
    };
    if result.trim().is_empty() {
        return failed(
            errors::GENERIC,
            errors::TITLE_REWRITE,
            "The rewrite came back empty, so the selection is unchanged.",
        );
    }
    thread::sleep(REWRITE_SETTLE);
    if core.shutting_down() {
        return Outcome::Quiet;
    }
    let written = result.trim();
    let chars = written.chars().count();
    match insert(core, cfg, written) {
        Ok(()) => Outcome::Rewrote {
            chars,
            method: insertion_mode(cfg),
        },
        Err(outcome) => outcome,
    }
}

/// The dictation and rewrite keys of the recording's settings and of the live ones.
fn hotkeys_of(core: &Core, cfg: &Config) -> Vec<Key> {
    let (keys, _) = hotkeys(cfg, core.opts.dev_mode);
    let live = core.config();
    let (live_keys, _) = hotkeys(&live, core.opts.dev_mode);
    [
        Some(keys.dictate),
        keys.rewrite,
        Some(live_keys.dictate),
        live_keys.rewrite,
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Waits until none of `held` is down, for at most [`DEFER_LIMIT`]. Fails once Voice quits.
fn wait_for_release(core: &Core, held: &[Key]) -> Result<(), InputError> {
    let quitting = || InputError::System("Voice is quitting.".into());
    let deadline = Instant::now() + DEFER_LIMIT;
    while held.iter().any(|&key| core.key_down(key)) && Instant::now() < deadline {
        if core.shutting_down() {
            return Err(quitting());
        }
        thread::sleep(DEFER_POLL);
    }
    if core.shutting_down() {
        return Err(quitting());
    }
    Ok(())
}

/// The closing quotes and brackets a sentence's full stop can be followed by.
const SENTENCE_CLOSERS: [char; 7] = ['"', '\'', '\u{201d}', '\u{2019}', ')', ']', '\u{bb}'];

/// The text a dictation inserts: the transcript, plus one trailing space when it ends a
/// sentence. Two dictations in a row then read "One sentence. The next one." instead of running
/// together. A fragment gets none, and neither does a text that ends in a newline or already in
/// whitespace, so no second space is ever added. A CJK full stop gets none either: those scripts
/// put no space after a sentence. The caller's text is already trimmed, and every reader of the
/// transcript before this point, the pill's word count included, keeps the text without the
/// space. A rewrite doesn't come here: it replaces a selection and fits what stood there.
fn inserted_text(text: &str) -> String {
    let ends_sentence = text
        .trim_end_matches(SENTENCE_CLOSERS)
        .chars()
        .next_back()
        .is_some_and(|last| SENTENCE_END.contains(&last));
    if !ends_sentence {
        return text.to_string();
    }
    let mut inserted = String::with_capacity(text.len() + 1);
    inserted.push_str(text);
    inserted.push(' ');
    inserted
}

/// Inserts with no hotkey held. Keys go to whatever has focus, with the keys the user holds:
/// pasting while Right Alt is held for the next dictation turns Ctrl+V into AltGr+V, which Ownkey
/// did (ownkey-dictation.md 17.7). Voice waits for the release instead, on purpose, right before
/// every batch of keys: a paste can wait for the previous paste's clipboard first, and a key can go
/// down during that wait or halfway through typing.
fn insert(core: &Core, cfg: &Config, text: &str) -> Result<(), Outcome> {
    let held = hotkeys_of(core, cfg);
    let mut released = || wait_for_release(core, &held);
    let mode = insertion_mode(cfg);
    core.svc
        .input
        .insert(text, mode, &mut released)
        .map_err(|e| {
            if core.shutting_down() {
                return Outcome::Quiet;
            }
            failed(
                errors::input_notice(&e),
                errors::TITLE_DICTATION,
                e.to_string(),
            )
        })
}

fn activity_of(mode: Mode) -> Activity {
    match mode {
        Mode::Dictate => Activity::Dictate,
        Mode::Rewrite => Activity::Rewrite,
    }
}

/// How this recording puts text in, which the outcome line names.
fn insertion_mode(cfg: &Config) -> InsertMode {
    if cfg.paste_mode {
        InsertMode::Paste
    } else {
        InsertMode::Type
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_come_from_settings_with_ownkeys_fallbacks() {
        let mut cfg = Config::defaults(true);
        // Pinned, so the test reads the same on a Mac, whose development copy has other defaults.
        cfg.hotkey = "f13".into();
        cfg.rewrite_hotkey = "f14".into();
        let (keys, stop) = hotkeys(&cfg, true);
        assert_eq!(
            (keys.dictate, keys.rewrite, stop),
            (Key::F13, Some(Key::F14), None)
        );
        cfg.hotkey = "nonsense".into();
        cfg.rewrite_hotkey = "off".into();
        cfg.meetings_stop_key = "f15".into();
        let (keys, stop) = hotkeys(&cfg, true);
        // An unknown key falls back to F13, or to Right Option on a Mac.
        let fallback = if cfg!(target_os = "macos") {
            Key::RightAlt
        } else {
            Key::F13
        };
        assert_eq!(
            (keys.dictate, keys.rewrite, stop),
            (fallback, None, Some(Key::F15))
        );
        let (keys, _) = hotkeys(&cfg, false);
        assert_eq!(keys.dictate, Key::RightAlt);
    }

    #[test]
    fn a_key_typed_during_a_modifier_hold_drops_the_recording() {
        let held = Duration::from_secs(2);
        let ago = |ms| Some(Duration::from_millis(ms));
        // Right Option and a letter, a second before the release.
        assert!(typed_during(Key::RightAlt, held, ago(1000)));
        assert!(typed_during(Key::RightCommand, held, ago(1000)));
        assert!(typed_during(Key::RightShift, held, ago(500)));
        // Before the press, or right at the release, it doesn't count.
        assert!(!typed_during(Key::RightAlt, held, ago(2500)));
        assert!(!typed_during(Key::RightAlt, held, ago(20)));
        // An F key's own press is a key-down; a platform that can't say drops nothing.
        assert!(!typed_during(Key::F13, held, ago(1000)));
        assert!(!typed_during(Key::RightAlt, held, None));
    }

    #[test]
    fn only_configured_hotkeys_count_away_from_the_console() {
        let keys = HotkeyConfig {
            dictate: Key::RightAlt,
            rewrite: Some(Key::RightCommand),
        };
        assert!(configured_hotkey(keys, Key::RightAlt));
        assert!(configured_hotkey(keys, Key::RightCommand));
        // Right Shift is what typed a password at the lock screen: it isn't a hotkey, so the
        // "away from the console" line must not name it.
        assert!(!configured_hotkey(keys, Key::RightShift));
        assert!(!configured_hotkey(keys, Key::F13));
        let rewrite_off = HotkeyConfig {
            dictate: Key::F13,
            rewrite: None,
        };
        assert!(configured_hotkey(rewrite_off, Key::F13));
        assert!(!configured_hotkey(rewrite_off, Key::RightCommand));
    }

    #[test]
    fn the_micro_tap_gate_matches_ownkey() {
        assert_eq!(MIN_SAMPLES, 378);
        assert_eq!(MIN_SAMPLES_WITHOUT_ACTIVITY, 1600);
    }

    /// A dictation that ends a sentence is inserted with one space after it, so the next one
    /// doesn't run into it. Closing quotes and brackets can follow the full stop.
    #[test]
    fn a_dictation_that_ends_a_sentence_gets_one_trailing_space() {
        for text in [
            "Done.",
            "Done!",
            "Done?",
            "Done\u{2026}",
            "Really?!",
            "He said \u{201c}done.\u{201d}",
            "She said 'done.'",
            "He said \"done.\"",
            "(Done.)",
            "[Done.]",
            "\u{ab}Done.\u{bb}",
        ] {
            let inserted = inserted_text(text);
            assert_eq!(inserted, format!("{text} "), "{text}");
            assert_eq!(
                inserted.matches(' ').count(),
                text.matches(' ').count() + 1,
                "exactly one space for {text}"
            );
        }
    }

    /// Nothing else gets one: a word or a fragment, a sentence that ends in a comma or a colon,
    /// a text that ends in a newline or already in whitespace, and a CJK full stop.
    #[test]
    fn a_fragment_a_newline_and_cjk_get_no_trailing_space() {
        for text in [
            "",
            "word",
            "a fragment",
            "ends in a comma,",
            "ends in a colon:",
            "ends in a semicolon;",
            "Done.\n",
            "Done. ",
            // Chinese and Japanese end a sentence without a space, so a full stop there gets
            // none.
            "\u{4f60}\u{597d}\u{3002}",
            "\u{597d}\u{7684}\u{ff01}",
            "\u{307b}\u{3093}\u{3068}\u{3046}\u{ff1f}",
        ] {
            assert_eq!(inserted_text(text), text, "{text:?}");
        }
    }

    /// Every kind of ending has its own words, so a check at the keyboard can tell a missing
    /// insertion from a silent one, and a rewrite from a dictation.
    #[test]
    fn every_ending_gets_one_line() {
        let hold = Duration::from_millis(2400);
        let audio = Duration::from_millis(2300);
        let peak = -31.0;
        let line = |ending: Ending| outcome_line(4, Mode::Dictate, hold, audio, peak, &ending);
        assert_eq!(
            line(Ending::Inserted {
                method: InsertMode::Paste,
                chars: 24
            }),
            "dictation 4 ended: inserted 24 characters by pasting (2.4 s held, 2.3 s of audio, peak -31 dBFS)"
        );
        assert_eq!(
            line(Ending::Inserted {
                method: InsertMode::Type,
                chars: 8
            }),
            "dictation 4 ended: inserted 8 characters by typing (2.4 s held, 2.3 s of audio, peak -31 dBFS)"
        );
        assert_eq!(
            line(Ending::NothingHeard),
            "dictation 4 ended: nothing heard (2.4 s held, 2.3 s of audio, peak -31 dBFS)"
        );
        assert_eq!(
            line(Ending::EmptyTranscript),
            "dictation 4 ended: empty transcript (2.4 s held, 2.3 s of audio, peak -31 dBFS)"
        );
        for (nothing, why) in [
            (Nothing::NoAudio, "nothing heard"),
            (Nothing::TooShort, "dropped (too short)"),
            (Nothing::EmptyTranscript, "empty transcript"),
        ] {
            assert_eq!(
                line(nothing.ending()),
                format!("dictation 4 ended: {why} (2.4 s held, 2.3 s of audio, peak -31 dBFS)")
            );
        }
        for (why, text) in [
            (Drop::AnotherKey, "another key went down during the hold"),
            (Drop::Session, "the session went away"),
            (Drop::TooShort, "too short"),
            (Drop::Cancelled, "Voice is quitting"),
        ] {
            assert_eq!(
                line(Ending::Dropped(why)),
                format!(
                    "dictation 4 ended: dropped ({text}) (2.4 s held, 2.3 s of audio, peak -31 dBFS)"
                )
            );
        }
        assert_eq!(
            line(Ending::Failed(errors::AUTH)),
            "dictation 4 ended: failed (pill.error.auth) (2.4 s held, 2.3 s of audio, peak -31 dBFS)"
        );
        // A press refused before the microphone opened has no audio at all.
        assert_eq!(
            outcome_line(
                6,
                Mode::Dictate,
                Duration::ZERO,
                Duration::ZERO,
                level::SILENT_DBFS,
                &Ending::Refused(errors::ACCESSIBILITY)
            ),
            "dictation 6 ended: not recorded (pill.error.accessibility) (0.0 s held, 0.0 s of audio, peak -96 dBFS)"
        );
        assert_eq!(
            outcome_line(
                7,
                Mode::Rewrite,
                Duration::ZERO,
                Duration::ZERO,
                level::SILENT_DBFS,
                &Ending::Refused(errors::ACCESSIBILITY)
            ),
            "rewrite 7 ended: not recorded (pill.error.accessibility) (0.0 s held, 0.0 s of audio, peak -96 dBFS)"
        );
        assert_eq!(
            outcome_line(
                9,
                Mode::Rewrite,
                hold,
                audio,
                peak,
                &Ending::EmptyTranscript
            ),
            "rewrite 9 ended: empty transcript (2.4 s held, 2.3 s of audio, peak -31 dBFS)"
        );
    }

    /// A capture that delivered only digital silence says so in the line, so it reads as the
    /// microphone's fault rather than as a recognizer that heard words and returned none.
    #[test]
    fn an_empty_line_says_when_nothing_but_silence_arrived() {
        let silence = vec![0i16; 16_000 * 10];
        let line = outcome_line(
            5,
            Mode::Dictate,
            Duration::from_millis(10_400),
            audio_of(&silence),
            level::peak_dbfs(&silence),
            &Ending::EmptyTranscript,
        );
        assert_eq!(
            line,
            "dictation 5 ended: empty transcript (10.4 s held, 10.0 s of audio, peak -96 dBFS)"
        );
    }

    /// A dictation during a meeting says that it takes the recorder's tap, and at which rate.
    #[test]
    fn a_tap_open_says_the_tap_is_used() {
        assert_eq!(
            tap_line(7),
            "dictation 7 capture open (meeting tap): the recorder's microphone tap at 16000 Hz"
        );
    }

    /// The line carries counts, durations and a level only: the text and the selection never
    /// reach it.
    #[test]
    fn the_outcome_line_holds_no_text() {
        let text = "a secret sentence";
        let line = outcome_line(
            3,
            Mode::Dictate,
            Duration::from_secs(1),
            Duration::from_secs(1),
            -42.0,
            &Ending::Inserted {
                method: InsertMode::Paste,
                chars: text.chars().count(),
            },
        );
        assert!(!line.contains("secret"), "{line}");
        assert!(line.contains("17 characters"), "{line}");
    }
}
