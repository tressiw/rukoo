//! The pill's state machine (pill-settings-design.md 3.4): one complete snapshot per change,
//! never a sequence of patches, with every hold, the meeting clock, the compact timer, the
//! two-step stop, the 60 s resend and the ack watchdog.
//!
//! [`Pill`] is pure: every call takes the time, so the tests run on a made-up clock.
//! [`PillDriver`] owns one behind a lock, sends what changes to the [`UiSink`] and wakes up for
//! the next deadline on its own thread.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::Map;

use crate::api::{
    Activity, Edge, Locale, MeetingPill, Notice, PillPhase, PillRepair, PillSnapshot, Theme, UiSink,
};

pub const INSERTED_HOLD: Duration = Duration::from_millis(700);
pub const NOTHING_HEARD_HOLD: Duration = Duration::from_millis(900);
pub const WARNING_HOLD: Duration = Duration::from_millis(2000);
/// Ownkey asked for 2800 ms after a microphone error but replaced it with 650 ms by accident
/// (ownkey-dictation.md 17.10); Voice keeps every error up for the intended 2800 ms on purpose.
pub const ERROR_HOLD: Duration = Duration::from_millis(2800);
pub const SAVING_HOLD: Duration = Duration::from_millis(2200);
pub const FAILED_HOLD: Duration = Duration::from_millis(2800);
pub const MIC_BACK_HOLD: Duration = Duration::from_millis(2200);
/// A part of the screen recording stopped: the notice stays this long.
pub const DROP_HOLD: Duration = Duration::from_millis(4000);
/// The screen recording moved to another monitor.
pub const MOVED_HOLD: Duration = Duration::from_millis(2200);
pub const COMPACT_AFTER: Duration = Duration::from_millis(4000);
pub const CONFIRM_STOP: Duration = Duration::from_millis(3000);
/// The page runs the meeting clock itself; a fresh snapshot this often corrects its drift.
pub const RESEND: Duration = Duration::from_secs(60);
pub const ACK_NUDGE: Duration = Duration::from_millis(500);
pub const ACK_REBUILD: Duration = Duration::from_secs(2);

pub mod notices {
    pub const NO_AUDIO: &str = "pill.noAudio";
    pub const LOADING_MODEL: &str = "pill.loadingModel";
    pub const SELECT_TEXT_BOX: &str = "pill.selectTextBox";
    pub const MIC_LOST: &str = "pill.micLost";
    pub const MIC_BACK: &str = "pill.micBack";
    pub const MEETING_FAILED: &str = "pill.meetingFailed";
    pub const VIDEO_DROPPED_DISK: &str = "pill.videoDropped.disk";
    pub const VIDEO_DROPPED_ENCODER: &str = "pill.videoDropped.encoder";
    pub const VIDEO_DROPPED_DEVICE: &str = "pill.videoDropped.device";
    pub const FRAMES_DROPPED_DISK: &str = "pill.framesDropped.disk";
    pub const SCREEN_FAILED: &str = "pill.screenFailed";
    pub const SCREEN_PERMISSION: &str = "pill.screenPermission";
    pub const SCREEN_CHANGE_FAILED: &str = "pill.screenChangeFailed";
    /// `{name}`: the monitor's name.
    pub const SCREEN_MOVED: &str = "pill.screenMoved";
}

pub fn notice(key: &str) -> Notice {
    Notice {
        key: key.to_string(),
        args: Map::new(),
    }
}

/// A meeting as the pill shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeetingPhase {
    Recording,
    Paused,
    /// Stopped; transcription goes on. Shown as "Saving" for a moment.
    Processing,
    Failed,
}

impl MeetingPhase {
    fn as_str(self) -> &'static str {
        match self {
            MeetingPhase::Recording => "recording",
            MeetingPhase::Paused => "paused",
            MeetingPhase::Processing => "processing",
            MeetingPhase::Failed => "failed",
        }
    }
}

/// How the pill looks, from Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Looks {
    pub edge: Edge,
    pub theme: Theme,
    pub locale: Locale,
    pub dev: bool,
}

impl Default for Looks {
    fn default() -> Looks {
        Looks {
            edge: Edge::Bottom,
            theme: Theme::Dark,
            locale: Locale::En,
            dev: false,
        }
    }
}

#[derive(Clone, Debug)]
struct Dictation {
    recording: u64,
    phase: PillPhase,
    activity: Activity,
    notice: Option<Notice>,
    words: Option<u32>,
    /// No text field had focus at the press: the hold shows "Select a text box" (Ownkey).
    target_warning: bool,
    until: Option<Instant>,
}

#[derive(Clone, Debug)]
struct Meeting {
    /// Which meeting this is. A meeting can start while the previous one still transcribes, and
    /// that one's outcome must not touch it.
    id: String,
    state: MeetingPhase,
    at_ms: u64,
    anchor: Instant,
    notice: Option<Notice>,
    notice_until: Option<Instant>,
    mic_lost: bool,
    /// What the meeting records of the screen now, and whether the screen button shows.
    screen: String,
    screen_switch: bool,
    /// The last change that shows the label again; the pill compacts 4 s after it.
    changed_at: Instant,
    confirm_until: Option<Instant>,
    until: Option<Instant>,
}

impl Meeting {
    fn clock(&self, now: Instant) -> u64 {
        match self.state {
            // The meeting clock keeps running while paused (PROTOCOL.md).
            MeetingPhase::Recording | MeetingPhase::Paused => {
                self.at_ms + now.saturating_duration_since(self.anchor).as_millis() as u64
            }
            MeetingPhase::Processing | MeetingPhase::Failed => self.at_ms,
        }
    }

    fn compact_at(&self) -> Option<Instant> {
        (self.state == MeetingPhase::Recording && self.notice.is_none())
            .then(|| self.changed_at + COMPACT_AFTER)
    }

    fn live(&self) -> bool {
        matches!(self.state, MeetingPhase::Recording | MeetingPhase::Paused)
    }
}

#[derive(Clone, Copy, Debug)]
struct Watch {
    seq: u64,
    shown: Instant,
    nudged: bool,
}

/// The pill's whole state.
#[derive(Debug)]
pub struct Pill {
    seq: u64,
    looks: Looks,
    dictation: Option<Dictation>,
    meeting: Option<Meeting>,
    /// The newest recording; older ones never touch the pill (ownkey-dictation.md 4.8).
    latest: u64,
    rukoo: bool,
    sent: PillSnapshot,
    sent_at: Instant,
    watch: Option<Watch>,
}

impl Pill {
    pub fn new(looks: Looks, now: Instant) -> Pill {
        Pill {
            seq: 0,
            looks,
            dictation: None,
            meeting: None,
            latest: 0,
            rukoo: false,
            sent: hidden(0, looks),
            sent_at: now,
            watch: None,
        }
    }

    /// The last snapshot sent, for `pill_snapshot`.
    pub fn current(&self) -> PillSnapshot {
        self.sent.clone()
    }

    pub fn set_looks(&mut self, looks: Looks) {
        self.looks = looks;
    }

    pub fn set_rukoo(&mut self, connected: bool) {
        self.rukoo = connected;
    }

    // Dictation ---------------------------------------------------------------------------

    /// A key went down. A new press always wins: it cancels pending holds.
    pub fn press(&mut self, recording: u64, activity: Activity, target_warning: bool) {
        self.latest = recording;
        self.dictation = Some(Dictation {
            recording,
            phase: PillPhase::Arming,
            activity,
            notice: None,
            words: None,
            target_warning,
            until: None,
        });
    }

    fn current_dictation(&mut self, recording: u64) -> Option<&mut Dictation> {
        if recording != self.latest {
            return None;
        }
        self.dictation.as_mut().filter(|d| d.recording == recording)
    }

    /// The first microphone buffer arrived.
    pub fn listening(&mut self, recording: u64) {
        if let Some(d) = self.current_dictation(recording) {
            if d.phase == PillPhase::Arming {
                d.phase = PillPhase::Listening;
            }
        }
    }

    /// Five seconds without sound (or the first sound after them).
    pub fn no_audio(&mut self, recording: u64, silent: bool) {
        if let Some(d) = self.current_dictation(recording) {
            if d.phase == PillPhase::Listening {
                d.notice = silent.then(|| notice(notices::NO_AUDIO));
            }
        }
    }

    /// Transcribing or rewriting, with an optional notice such as "Loading model".
    pub fn working(
        &mut self,
        recording: u64,
        phase: PillPhase,
        activity: Activity,
        notice: Option<Notice>,
    ) {
        if recording != self.latest {
            return;
        }
        self.dictation = Some(Dictation {
            recording,
            phase,
            activity,
            notice,
            words: None,
            target_warning: false,
            until: None,
        });
    }

    /// The end of a recording: inserted, rewrote, nothing heard, a warning or an error, shown
    /// for `hold`.
    pub fn finished(
        &mut self,
        recording: u64,
        phase: PillPhase,
        notice: Option<Notice>,
        words: Option<u32>,
        hold: Duration,
        now: Instant,
    ) {
        if recording != self.latest {
            return;
        }
        let activity = self
            .dictation
            .as_ref()
            .map_or(Activity::Dictate, |d| d.activity);
        self.dictation = Some(Dictation {
            recording,
            phase,
            activity,
            notice,
            words,
            target_warning: false,
            until: Some(now + hold),
        });
    }

    /// Whether level ticks may go out for this recording now.
    pub fn listening_to(&self, recording: u64) -> bool {
        recording == self.latest && self.sent.phase == PillPhase::Listening
    }

    /// Hides everything, for quitting.
    pub fn clear(&mut self) {
        self.dictation = None;
        self.meeting = None;
    }

    // Meetings ----------------------------------------------------------------------------

    /// Meeting `id` is in `state`. A failure of another meeting never covers a meeting that
    /// records: it shows only when the pill has no live meeting.
    pub fn meeting(&mut self, id: &str, state: MeetingPhase, at_ms: u64, now: Instant) {
        if state == MeetingPhase::Failed
            && self
                .meeting
                .as_ref()
                .is_some_and(|m| m.id != id && m.live())
        {
            return;
        }
        let (notice, notice_until, until) = match state {
            MeetingPhase::Recording | MeetingPhase::Paused => (None, None, None),
            MeetingPhase::Processing => (None, None, Some(now + SAVING_HOLD)),
            MeetingPhase::Failed => (
                Some(notice(notices::MEETING_FAILED)),
                Some(now + FAILED_HOLD),
                Some(now + FAILED_HOLD),
            ),
        };
        let keep_mic = self
            .meeting
            .as_ref()
            .filter(|m| {
                m.id == id && matches!(state, MeetingPhase::Recording | MeetingPhase::Paused)
            })
            .map(|m| (m.mic_lost, m.notice.clone(), m.notice_until));
        let (mic_lost, notice, notice_until) = match keep_mic {
            Some((true, kept, kept_until)) => (true, kept, kept_until),
            _ => (false, notice, notice_until),
        };
        let (screen, screen_switch) = self
            .meeting
            .as_ref()
            .filter(|m| m.id == id)
            .map_or(("off".to_string(), false), |m| {
                (m.screen.clone(), m.screen_switch)
            });
        self.meeting = Some(Meeting {
            id: id.to_string(),
            state,
            at_ms,
            anchor: now,
            notice,
            notice_until,
            mic_lost,
            screen,
            screen_switch,
            changed_at: now,
            confirm_until: None,
            until,
        });
    }

    /// What the meeting records of the screen now (`off`, `frames` or `video`), and whether the
    /// pill offers its screen button. Not a change that shows the label again.
    pub fn meeting_screen(&mut self, mode: &str, can_switch: bool) {
        if let Some(m) = self.meeting.as_mut() {
            m.screen = mode.to_string();
            m.screen_switch = can_switch;
        }
    }

    /// A notice about the meeting, such as a part of the screen recording that stopped, shown
    /// for `hold`. A lost microphone shows again after it.
    pub fn meeting_notice(
        &mut self,
        key: &str,
        args: Map<String, serde_json::Value>,
        hold: Duration,
        now: Instant,
    ) {
        let Some(m) = self.meeting.as_mut() else {
            return;
        };
        if !m.live() {
            return;
        }
        m.notice = Some(Notice {
            key: key.to_string(),
            args,
        });
        m.notice_until = Some(now + hold);
        m.changed_at = now;
    }

    /// Meeting `id` ended without anything to show (`done` after the saving hold). Another
    /// meeting on the pill stays.
    pub fn meeting_gone(&mut self, id: &str) {
        if self.meeting.as_ref().is_some_and(|m| {
            m.id == id && (m.until.is_none() || m.state == MeetingPhase::Recording)
        }) {
            self.meeting = None;
        }
    }

    /// The recorder's clock, which corrects the anchor without counting as a change.
    pub fn meeting_clock(&mut self, at_ms: u64, now: Instant) {
        if let Some(m) = self.meeting.as_mut() {
            if matches!(m.state, MeetingPhase::Recording | MeetingPhase::Paused) {
                m.at_ms = at_ms;
                m.anchor = now;
            }
        }
    }

    /// The meeting's microphone went away or came back.
    pub fn meeting_mic(&mut self, ok: bool, now: Instant) {
        let Some(m) = self.meeting.as_mut() else {
            return;
        };
        if !matches!(m.state, MeetingPhase::Recording | MeetingPhase::Paused) || ok != m.mic_lost {
            return;
        }
        m.mic_lost = !ok;
        if ok {
            m.notice = Some(notice(notices::MIC_BACK));
            m.notice_until = Some(now + MIC_BACK_HOLD);
        } else {
            m.notice = Some(notice(notices::MIC_LOST));
            m.notice_until = None;
        }
        m.changed_at = now;
    }

    /// A click on the stop button (or the stop key). Returns true when it should stop now: the
    /// second click within 3 s of the first. A meeting can't be repeated, so one stray click
    /// over a call's toolbar must not end it (pill-settings-design.md 5.5).
    pub fn stop_click(&mut self, now: Instant) -> bool {
        let Some(m) = self.meeting.as_mut() else {
            return false;
        };
        if !matches!(m.state, MeetingPhase::Recording | MeetingPhase::Paused) {
            return false;
        }
        if m.confirm_until.is_some_and(|until| now < until) {
            m.confirm_until = None;
            return true;
        }
        m.confirm_until = Some(now + CONFIRM_STOP);
        false
    }

    // Snapshots ---------------------------------------------------------------------------

    fn expire(&mut self, now: Instant) {
        if self
            .dictation
            .as_ref()
            .is_some_and(|d| d.until.is_some_and(|until| now >= until))
        {
            self.dictation = None;
        }
        if let Some(m) = self.meeting.as_mut() {
            if m.until.is_some_and(|until| now >= until) {
                self.meeting = None;
            } else {
                if m.notice_until.is_some_and(|until| now >= until) {
                    // The microphone is still lost: its notice comes back.
                    m.notice = m.mic_lost.then(|| notice(notices::MIC_LOST));
                    m.notice_until = None;
                    m.changed_at = now;
                }
                if m.confirm_until.is_some_and(|until| now >= until) {
                    m.confirm_until = None;
                }
            }
        }
    }

    /// What the pill shows now, with `seq` 0.
    fn compute(&self, now: Instant) -> PillSnapshot {
        let meeting = self.meeting.as_ref().map(|m| MeetingPill {
            state: m.state.as_str().to_string(),
            at_ms: m.clock(now),
            screen: m.screen.clone(),
            screen_switch: m.screen_switch && m.live(),
            compact: m.compact_at().is_some_and(|at| now >= at),
            confirm_stop: m.confirm_until.is_some_and(|until| now < until),
            stop_enabled: self.dictation.is_none()
                && matches!(m.state, MeetingPhase::Recording | MeetingPhase::Paused),
            rukoo_connected: self.rukoo,
        });
        let looks = self.looks;
        let base = |phase, activity, notice, words| PillSnapshot {
            seq: 0,
            visible: true,
            phase,
            activity,
            notice,
            words,
            meeting: meeting.clone(),
            edge: looks.edge,
            theme: looks.theme,
            locale: looks.locale,
            dev: looks.dev,
        };
        if let Some(d) = &self.dictation {
            let held = matches!(d.phase, PillPhase::Arming | PillPhase::Listening);
            if held && d.target_warning {
                return base(
                    PillPhase::Warning,
                    d.activity,
                    Some(notice(notices::SELECT_TEXT_BOX)),
                    None,
                );
            }
            return base(d.phase, d.activity, d.notice.clone(), d.words);
        }
        if let Some(m) = &self.meeting {
            return base(
                PillPhase::Meeting,
                Activity::Dictate,
                m.notice.clone(),
                None,
            );
        }
        hidden(0, looks)
    }

    /// The snapshot to send now, if anything the page shows changed or the resend is due.
    pub fn take(&mut self, now: Instant) -> Option<PillSnapshot> {
        self.expire(now);
        let next = self.compute(now);
        let resend = next.visible
            && next.meeting.is_some()
            && now.saturating_duration_since(self.sent_at) >= RESEND;
        if !resend && same_view(&next, &self.sent) {
            return None;
        }
        let shown = next.visible && (!self.sent.visible || next.phase != self.sent.phase);
        self.seq += 1;
        let snapshot = PillSnapshot {
            seq: self.seq,
            ..next
        };
        if shown {
            self.watch = Some(Watch {
                seq: snapshot.seq,
                shown: now,
                nudged: false,
            });
        } else if !snapshot.visible {
            self.watch = None;
        }
        self.sent = snapshot.clone();
        self.sent_at = now;
        Some(snapshot)
    }

    /// The page painted `seq`.
    pub fn ack(&mut self, seq: u64) {
        if self.watch.is_some_and(|w| seq >= w.seq) {
            self.watch = None;
        }
    }

    /// A repair the app should make now, when a shown snapshot wasn't acked in time.
    pub fn repair(&mut self, now: Instant) -> Option<(u64, PillRepair)> {
        let watch = self.watch.as_mut()?;
        let waited = now.saturating_duration_since(watch.shown);
        if waited >= ACK_REBUILD {
            let seq = watch.seq;
            self.watch = None;
            return Some((seq, PillRepair::Rebuild));
        }
        if waited >= ACK_NUDGE && !watch.nudged {
            watch.nudged = true;
            return Some((watch.seq, PillRepair::Nudge));
        }
        None
    }

    /// When something changes on its own next: a hold ends, the pill compacts, a resend or a
    /// watchdog step is due.
    pub fn next_deadline(&self) -> Option<Instant> {
        let mut times: Vec<Instant> = Vec::new();
        if let Some(d) = &self.dictation {
            times.extend(d.until);
        }
        if let Some(m) = &self.meeting {
            times.extend(m.until);
            times.extend(m.notice_until);
            times.extend(m.confirm_until);
            if !self.sent.meeting.as_ref().is_some_and(|p| p.compact) {
                times.extend(m.compact_at());
            }
            if self.sent.visible {
                times.push(self.sent_at + RESEND);
            }
        }
        if let Some(w) = &self.watch {
            times.push(w.shown + if w.nudged { ACK_REBUILD } else { ACK_NUDGE });
        }
        times.into_iter().min()
    }
}

fn hidden(seq: u64, looks: Looks) -> PillSnapshot {
    PillSnapshot {
        seq,
        visible: false,
        phase: PillPhase::Hidden,
        activity: Activity::Dictate,
        notice: None,
        words: None,
        meeting: None,
        edge: looks.edge,
        theme: looks.theme,
        locale: looks.locale,
        dev: looks.dev,
    }
}

/// Equal for the page: the meeting time runs on the page, so it doesn't count.
fn same_view(a: &PillSnapshot, b: &PillSnapshot) -> bool {
    let strip = |s: &PillSnapshot| {
        let mut s = s.clone();
        s.seq = 0;
        if let Some(m) = s.meeting.as_mut() {
            m.at_ms = 0;
        }
        s
    };
    strip(a) == strip(b)
}

/// The pill behind a lock, its sink and the thread that wakes it for its deadlines.
pub struct PillDriver {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Shared {
    pill: Mutex<Pill>,
    wake: Condvar,
    stopped: Mutex<bool>,
    sink: Arc<dyn UiSink>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Pill> {
        self.pill.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stopped(&self) -> MutexGuard<'_, bool> {
        self.stopped.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes the timer thread. Taking its lock first means the wake can't fall between the
    /// thread reading the next deadline and starting to wait for it.
    fn wake(&self) {
        let _stopped = self.stopped();
        self.wake.notify_all();
    }

    /// Sends what changed. Runs under the pill's lock, so snapshots leave in `seq` order.
    fn flush(&self, pill: &mut Pill, now: Instant) {
        if let Some(snapshot) = pill.take(now) {
            self.sink.pill(snapshot);
        }
        if let Some((seq, repair)) = pill.repair(now) {
            self.sink.pill_repair(seq, repair);
        }
    }
}

impl PillDriver {
    pub fn new(looks: Looks, sink: Arc<dyn UiSink>) -> PillDriver {
        let shared = Arc::new(Shared {
            pill: Mutex::new(Pill::new(looks, Instant::now())),
            wake: Condvar::new(),
            stopped: Mutex::new(false),
            sink,
        });
        let thread = thread::Builder::new()
            .name("pill".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared)
            })
            .ok();
        PillDriver {
            shared,
            thread: Mutex::new(thread),
        }
    }

    /// Changes the pill and sends the result.
    pub fn update<T>(&self, change: impl FnOnce(&mut Pill, Instant) -> T) -> T {
        let now = Instant::now();
        let mut pill = self.shared.lock();
        let result = change(&mut pill, now);
        self.shared.flush(&mut pill, now);
        drop(pill);
        self.shared.wake();
        result
    }

    pub fn read<T>(&self, look: impl FnOnce(&Pill) -> T) -> T {
        look(&self.shared.lock())
    }

    pub fn stop(&self) {
        *self.shared.stopped() = true;
        self.shared.wake();
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

fn run(shared: &Shared) {
    loop {
        {
            // The deadline is read under the timer's lock (see `Shared::wake`). Lock order:
            // this lock, then the pill's; `update` never holds the pill's while it takes this one.
            let stopped = shared.stopped();
            if *stopped {
                return;
            }
            let deadline = shared.lock().next_deadline();
            let wait = deadline.map_or(Duration::from_secs(3600), |at| {
                at.saturating_duration_since(Instant::now())
            });
            let (stopped, _) = shared
                .wake
                .wait_timeout(stopped, wait)
                .unwrap_or_else(PoisonError::into_inner);
            if *stopped {
                return;
            }
        }
        let now = Instant::now();
        let mut pill = shared.lock();
        shared.flush(&mut pill, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn start() -> (Pill, Instant) {
        let t0 = Instant::now();
        (Pill::new(Looks::default(), t0), t0)
    }

    #[test]
    fn a_dictation_runs_through_its_phases_and_hides_after_the_hold() {
        let (mut pill, t0) = start();
        assert!(pill.take(t0).is_none(), "nothing changed yet");
        pill.press(1, Activity::Dictate, false);
        let s = pill.take(t0).unwrap();
        assert_eq!((s.seq, s.visible, s.phase), (1, true, PillPhase::Arming));
        pill.listening(1);
        assert_eq!(pill.take(t0).unwrap().phase, PillPhase::Listening);
        assert!(pill.listening_to(1));
        pill.no_audio(1, true);
        assert_eq!(pill.take(t0).unwrap().notice.unwrap().key, "pill.noAudio");
        pill.no_audio(1, false);
        assert_eq!(pill.take(t0).unwrap().notice, None);
        pill.working(1, PillPhase::Transcribing, Activity::Dictate, None);
        assert_eq!(pill.take(t0).unwrap().phase, PillPhase::Transcribing);
        pill.finished(1, PillPhase::Inserted, None, Some(4), INSERTED_HOLD, t0);
        let s = pill.take(t0).unwrap();
        assert_eq!((s.phase, s.words), (PillPhase::Inserted, Some(4)));
        assert_eq!(
            pill.next_deadline(),
            Some(t0 + ACK_NUDGE),
            "the watchdog waits for the paint"
        );
        pill.ack(s.seq);
        assert_eq!(pill.next_deadline(), Some(t0 + INSERTED_HOLD));
        assert!(pill.take(t0 + ms(699)).is_none());
        let s = pill.take(t0 + ms(700)).unwrap();
        assert_eq!(
            (s.visible, s.phase, s.seq),
            (false, PillPhase::Hidden, 7),
            "one seq per change"
        );
    }

    #[test]
    fn holds_match_the_design() {
        for (phase, hold) in [
            (PillPhase::NothingHeard, NOTHING_HEARD_HOLD),
            (PillPhase::Warning, WARNING_HOLD),
            (PillPhase::Error, ERROR_HOLD),
            (PillPhase::Rewrote, INSERTED_HOLD),
        ] {
            let (mut pill, t0) = start();
            pill.press(1, Activity::Rewrite, false);
            pill.finished(1, phase, None, None, hold, t0);
            assert_eq!(pill.take(t0).unwrap().phase, phase);
            assert!(pill.take(t0 + hold - ms(1)).is_none());
            assert!(!pill.take(t0 + hold).unwrap().visible, "{phase:?}");
        }
    }

    #[test]
    fn a_new_press_cancels_a_hold_and_older_recordings_lose_the_pill() {
        let (mut pill, t0) = start();
        pill.press(1, Activity::Dictate, false);
        pill.working(1, PillPhase::Transcribing, Activity::Dictate, None);
        pill.press(2, Activity::Dictate, false);
        pill.take(t0);
        // Recording 1 finishes while 2 is held: the pill stays on recording 2.
        pill.finished(1, PillPhase::Inserted, None, Some(3), INSERTED_HOLD, t0);
        pill.working(1, PillPhase::Rewriting, Activity::Rewrite, None);
        assert!(pill.take(t0).is_none());
        pill.finished(2, PillPhase::Error, None, None, ERROR_HOLD, t0);
        assert_eq!(pill.take(t0).unwrap().phase, PillPhase::Error);
        pill.press(3, Activity::Dictate, false);
        assert_eq!(pill.take(t0 + ms(10)).unwrap().phase, PillPhase::Arming);
        assert!(pill.take(t0 + ERROR_HOLD).is_none(), "the old hold is gone");
    }

    #[test]
    fn no_text_field_shows_a_warning_for_the_whole_hold() {
        let (mut pill, t0) = start();
        pill.press(1, Activity::Dictate, true);
        let s = pill.take(t0).unwrap();
        assert_eq!(s.phase, PillPhase::Warning);
        assert_eq!(s.notice.unwrap().key, "pill.selectTextBox");
        pill.listening(1);
        assert!(pill.take(t0).is_none());
        assert!(!pill.listening_to(1), "no level ticks behind the warning");
        pill.working(1, PillPhase::Transcribing, Activity::Dictate, None);
        assert_eq!(pill.take(t0).unwrap().phase, PillPhase::Transcribing);
    }

    #[test]
    fn the_meeting_pill_compacts_resends_and_pauses() {
        let (mut pill, t0) = start();
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        let s = pill.take(t0).unwrap();
        let m = s.meeting.unwrap();
        assert_eq!(s.phase, PillPhase::Meeting);
        assert!(m.stop_enabled && !m.compact && !m.confirm_stop);
        pill.ack(s.seq);
        assert_eq!(pill.next_deadline(), Some(t0 + COMPACT_AFTER));
        assert!(pill.take(t0 + ms(3999)).is_none());
        let s = pill.take(t0 + COMPACT_AFTER).unwrap();
        assert!(s.meeting.as_ref().unwrap().compact);
        assert_eq!(
            s.meeting.unwrap().at_ms,
            4000,
            "the clock runs from the anchor"
        );
        // Nothing else changes for a minute: the snapshot is sent again to correct drift.
        assert!(pill.take(t0 + ms(30_000)).is_none());
        let s = pill.take(t0 + COMPACT_AFTER + RESEND).unwrap();
        assert_eq!(s.meeting.unwrap().at_ms, 64_000);
        // Paused: never compact, and the clock keeps running.
        let t1 = t0 + ms(70_000);
        pill.meeting("m-a", MeetingPhase::Paused, 70_000, t1);
        let s = pill.take(t1).unwrap();
        assert_eq!(s.meeting.as_ref().unwrap().state, "paused");
        assert!(!s.meeting.unwrap().compact);
        assert!(pill.take(t1 + ms(10_000)).is_none());
        pill.meeting_clock(90_500, t1 + ms(20_000));
        assert!(
            pill.take(t1 + ms(20_000)).is_none(),
            "a clock fix isn't a change"
        );
    }

    #[test]
    fn stopping_takes_two_clicks_within_three_seconds() {
        let (mut pill, t0) = start();
        assert!(!pill.stop_click(t0), "no meeting, nothing to stop");
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        pill.take(t0);
        assert!(!pill.stop_click(t0 + ms(100)));
        assert!(
            pill.take(t0 + ms(100))
                .unwrap()
                .meeting
                .unwrap()
                .confirm_stop
        );
        // The confirmation runs out after 3 s; the next click asks again.
        let s = pill.take(t0 + ms(3100)).unwrap();
        assert!(!s.meeting.unwrap().confirm_stop);
        assert!(!pill.stop_click(t0 + ms(3200)));
        assert!(
            pill.stop_click(t0 + ms(5000)),
            "second click within 3 s stops"
        );
        assert!(
            !pill
                .take(t0 + ms(5000))
                .unwrap()
                .meeting
                .unwrap()
                .confirm_stop
        );
    }

    #[test]
    fn dictation_in_a_meeting_hides_the_stop_button_then_returns_to_the_meeting() {
        let (mut pill, t0) = start();
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        pill.take(t0);
        pill.press(1, Activity::Dictate, false);
        let s = pill.take(t0).unwrap();
        assert_eq!(s.phase, PillPhase::Arming);
        assert!(!s.meeting.unwrap().stop_enabled);
        pill.finished(1, PillPhase::Inserted, None, Some(2), INSERTED_HOLD, t0);
        pill.take(t0);
        let s = pill.take(t0 + INSERTED_HOLD).unwrap();
        assert_eq!(s.phase, PillPhase::Meeting);
        assert!(s.meeting.unwrap().stop_enabled);
    }

    #[test]
    fn saving_shows_then_hides_and_failure_shows_its_notice() {
        let (mut pill, t0) = start();
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        pill.take(t0);
        pill.meeting("m-a", MeetingPhase::Processing, 9000, t0 + ms(9000));
        let s = pill.take(t0 + ms(9000)).unwrap();
        let m = s.meeting.unwrap();
        assert_eq!((m.state.as_str(), m.stop_enabled), ("processing", false));
        assert!(!pill.take(t0 + ms(9000) + SAVING_HOLD).unwrap().visible);

        pill.meeting("m-a", MeetingPhase::Failed, 100, t0 + ms(20_000));
        let s = pill.take(t0 + ms(20_000)).unwrap();
        assert_eq!(s.notice.unwrap().key, "pill.meetingFailed");
        assert!(!pill.take(t0 + ms(20_000) + FAILED_HOLD).unwrap().visible);
    }

    #[test]
    fn an_earlier_meeting_s_outcome_never_touches_the_one_that_records() {
        let (mut pill, t0) = start();
        // Meeting A stopped and still transcribes while B records.
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        pill.meeting("m-a", MeetingPhase::Processing, 5000, t0 + ms(5000));
        pill.meeting("m-b", MeetingPhase::Recording, 0, t0 + ms(6000));
        pill.take(t0 + ms(6000));
        let t1 = t0 + ms(30_000);
        pill.meeting_gone("m-a");
        pill.meeting("m-a", MeetingPhase::Failed, 5000, t1);
        let s = pill.take(t1).unwrap();
        let m = s.meeting.unwrap();
        assert_eq!(
            (s.phase, m.state.as_str()),
            (PillPhase::Meeting, "recording")
        );
        assert!(m.stop_enabled && s.notice.is_none());
        assert!(
            pill.take(t1 + FAILED_HOLD).is_none(),
            "nothing to hide once A's failure would have ended"
        );
        assert_eq!(pill.current().meeting.unwrap().state, "recording");
        assert!(
            !pill.stop_click(t1 + FAILED_HOLD),
            "the stop key still asks"
        );
        assert!(pill.stop_click(t1 + FAILED_HOLD + ms(100)), "and stops B");
        // B's own outcome does reach it.
        pill.meeting("m-b", MeetingPhase::Processing, 9000, t1 + ms(10_000));
        pill.meeting_gone("m-b");
        let s = pill.take(t1 + ms(10_000)).unwrap();
        assert_eq!(s.meeting.unwrap().state, "processing", "saving shows first");
        assert!(!pill.take(t1 + ms(10_000) + SAVING_HOLD).unwrap().visible);
        // With nothing recording, an earlier meeting's failure shows.
        pill.meeting("m-a", MeetingPhase::Failed, 5000, t1 + ms(20_000));
        let s = pill.take(t1 + ms(20_000)).unwrap();
        assert_eq!(s.notice.unwrap().key, "pill.meetingFailed");
    }

    #[test]
    fn a_lost_microphone_shows_until_it_comes_back() {
        let (mut pill, t0) = start();
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        pill.take(t0);
        pill.meeting_mic(false, t0 + ms(5000));
        let s = pill.take(t0 + ms(5000)).unwrap();
        assert_eq!(s.notice.unwrap().key, "pill.micLost");
        assert!(
            !s.meeting.unwrap().compact,
            "never compact while a notice shows"
        );
        assert!(pill.take(t0 + ms(20_000)).is_none());
        // Pausing keeps the notice.
        pill.meeting("m-a", MeetingPhase::Paused, 21_000, t0 + ms(21_000));
        assert_eq!(
            pill.take(t0 + ms(21_000)).unwrap().notice.unwrap().key,
            "pill.micLost"
        );
        pill.meeting_mic(true, t0 + ms(22_000));
        assert_eq!(
            pill.take(t0 + ms(22_000)).unwrap().notice.unwrap().key,
            "pill.micBack"
        );
        let s = pill.take(t0 + ms(22_000) + MIC_BACK_HOLD).unwrap();
        assert_eq!(s.notice, None);
    }

    #[test]
    fn screen_notices_hold_four_seconds_and_the_screen_shows() {
        let (mut pill, t0) = start();
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        pill.meeting_screen("video", true);
        let s = pill.take(t0).unwrap();
        let m = s.meeting.as_ref().unwrap();
        assert_eq!((m.screen.as_str(), m.screen_switch), ("video", true));
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["meeting"]["screen"], "video");
        assert_eq!(json["meeting"]["screenSwitch"], true);

        let t1 = t0 + ms(10_000);
        pill.meeting_notice(notices::VIDEO_DROPPED_DISK, Map::new(), DROP_HOLD, t1);
        pill.meeting_screen("frames", true);
        let s = pill.take(t1).unwrap();
        assert_eq!(s.notice.as_ref().unwrap().key, "pill.videoDropped.disk");
        assert_eq!(s.meeting.as_ref().unwrap().screen, "frames");
        assert!(
            !s.meeting.unwrap().compact,
            "a notice shows the pill in full"
        );
        assert!(pill.take(t1 + ms(3999)).is_none());
        let s = pill.take(t1 + DROP_HOLD).unwrap();
        assert_eq!(s.notice, None);

        // Pausing keeps the screen; moving shows the monitor's name.
        let t2 = t1 + ms(20_000);
        pill.meeting("m-a", MeetingPhase::Paused, 30_000, t2);
        let mut args = Map::new();
        args.insert("name".into(), "DELL U2720Q".into());
        pill.meeting_notice(notices::SCREEN_MOVED, args, MOVED_HOLD, t2);
        let s = pill.take(t2).unwrap();
        assert_eq!(s.meeting.as_ref().unwrap().screen, "frames");
        let notice = s.notice.unwrap();
        assert_eq!(notice.key, "pill.screenMoved");
        assert_eq!(notice.args["name"], "DELL U2720Q");

        // Off keeps the button so capture can be enabled again; a stopped meeting shows none.
        pill.meeting_screen("off", true);
        assert!(
            pill.take(t2 + ms(1))
                .unwrap()
                .meeting
                .unwrap()
                .screen_switch
        );
        pill.meeting_screen("frames", true);
        pill.meeting("m-a", MeetingPhase::Processing, 40_000, t2 + ms(2));
        assert!(
            !pill
                .take(t2 + ms(2))
                .unwrap()
                .meeting
                .unwrap()
                .screen_switch
        );
    }

    #[test]
    fn a_lost_microphone_shows_again_after_a_screen_notice() {
        let (mut pill, t0) = start();
        pill.meeting("m-a", MeetingPhase::Recording, 0, t0);
        pill.meeting_mic(false, t0);
        pill.take(t0);
        pill.meeting_notice(notices::SCREEN_FAILED, Map::new(), DROP_HOLD, t0 + ms(100));
        assert_eq!(
            pill.take(t0 + ms(100)).unwrap().notice.unwrap().key,
            "pill.screenFailed"
        );
        let s = pill.take(t0 + ms(100) + DROP_HOLD).unwrap();
        assert_eq!(s.notice.unwrap().key, "pill.micLost");
    }

    #[test]
    fn the_watchdog_nudges_then_rebuilds_an_unpainted_pill() {
        let (mut pill, t0) = start();
        pill.press(1, Activity::Dictate, false);
        let seq = pill.take(t0).unwrap().seq;
        assert_eq!(pill.repair(t0 + ms(499)), None);
        assert_eq!(pill.repair(t0 + ms(500)), Some((seq, PillRepair::Nudge)));
        assert_eq!(pill.repair(t0 + ms(900)), None);
        assert_eq!(pill.repair(t0 + ms(2000)), Some((seq, PillRepair::Rebuild)));
        assert_eq!(pill.repair(t0 + ms(5000)), None);

        pill.listening(1);
        let seq = pill.take(t0 + ms(6000)).unwrap().seq;
        pill.ack(seq);
        assert_eq!(pill.repair(t0 + ms(9000)), None, "acked in time");
    }

    #[test]
    fn looks_follow_settings() {
        let (mut pill, t0) = start();
        pill.press(1, Activity::Dictate, false);
        pill.take(t0);
        pill.set_looks(Looks {
            edge: Edge::Top,
            theme: Theme::Light,
            locale: Locale::Nl,
            dev: true,
        });
        let s = pill.take(t0).unwrap();
        assert_eq!(
            (s.edge, s.theme, s.locale, s.dev),
            (Edge::Top, Theme::Light, Locale::Nl, true)
        );
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["phase"], "arming");
        assert_eq!(json["dev"], true);
        assert_eq!(json["edge"], "top");
    }
}
