//! Follows one meeting while it records and afterwards, until every recorded second is
//! transcribed, and sends every meeting event to the event log.
//!
//! The recorder writes the audio and the timeline; the pipeline only reads them. It can stop,
//! crash or wait for consent at any moment without costing audio: its progress lives in
//! `transcript.db`, and a pipeline started again later continues from there. Each event has a key
//! that `transcript.db` keeps once the event is in the log, and at start the keys are checked
//! against the log itself, so nothing is sent twice, even after a crash between the two writes.
//!
//! With speaker labels, the call track has its own follower thread that stores who spoke when
//! (`speakers/`). The pipeline then cuts call windows only up to the follower's settled position,
//! splits a window's words by speaker before it commits them, and later names passages it had to
//! commit without a speaker.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use voice_bridge::{EventLog, LogError};
use voice_core::{Hints, LocalEngine, ProviderError, SpeechToText};
use voice_protocol::{events, MeetingState, MeetingStateChange, Passage, Speaker, Track};

use crate::library::{MeetingDir, MeetingFile};
use crate::reader::{segment_samples, TrackReader, PER_MS};
use crate::screen::{derived_key, ScreenDerived, ScreenFollower};
use crate::segmentation::{
    next_boundary_with, owned_tokens, timed_passages, window_passage, within, worth_recognizing,
    Draft, CORE_LIMIT, LEFT_CONTEXT, RATE, RIGHT_CONTEXT,
};
use crate::speakers::follower::{
    Follower, FollowerHandle, FollowerShared, FollowerTiming, Snapshot,
};
use crate::speakers::stream::StreamFollower;
use crate::speakers::turns::{
    passage_speaker, segment_passages, speaker_passages, turn_cuts, window_speaker, Turn,
};
use crate::speakers::{LabelState, LabelStatus, Labels, StreamOptions};
use crate::stt::{Heard, Recognizer, TimedSpeechToText};
use crate::transcript::{
    call_number, taken_key, Outcome, StoredPassage, StoredSegment, Transcript, BACKFILL_KEY,
    DIARIZE_KEY, LABELLED_KEY, MODE_KEY, REASON_KEY, SETTLED_KEY,
};
use crate::watcher::{
    mark_key, state_key, tracks_of, Derived, Mapper, Stop, Stretch, TimelineWatcher,
};
#[cfg(test)]
use voice_protocol::ScreenMode;

/// A track without an open segment may still get a segment that starts this far before the
/// timeline's latest time: a padded loopback writes its `segment` line up to 0.5 s late.
const SETTLE_MS: u64 = 1_000;
/// While a track has no audio, the pipeline moves past the silence in steps of at least 5 s.
const GAP_STEP: u64 = 5 * RATE as u64;
/// Neighbouring segments this close count as one stretch of audio: drift correction leaves
/// gaps of about 0.1 s (voice/RECORDING.md). A lost device is retried every second, so a real
/// gap is longer.
pub(crate) const DRIFT_GAP: u64 = 300 * PER_MS;
/// Dictation comes first: recognition waits while the user dictates, up to two minutes, as in
/// Ownkey (`meetings/service.py:973-986`).
const DICTATION_YIELD_MS: u64 = 120_000;
/// How long [`PipelineHandle::shutdown`] waits for a recognition call to return.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);
/// Turns this far around a window still decide its words and cuts.
const TURN_MARGIN_MS: u64 = 2_000;
/// The [`Labels::Off`] reason of a meeting provider that streams: the stream labels the call.
/// Without a stream in the options too, the stream couldn't be set up yet.
pub const STREAM_REASON: &str = "stream";

/// The reader for one track of a meeting folder, from the timeline read so far. `lengths` keeps
/// the samples of segments that ended, which can't change any more.
pub(crate) fn read_track(
    dir: &Path,
    track: Track,
    mapper: &Mapper,
    lengths: &mut HashMap<(String, u64), u64>,
) -> TrackReader {
    TrackReader::with_lengths(dir, track.as_str(), mapper.segments(), |s| match s.ended {
        Some(ended) => *lengths
            .entry((s.track.clone(), s.seq))
            .or_insert_with(|| segment_samples(&dir.join(&s.file), Some(ended))),
        None => segment_samples(&dir.join(&s.file), None),
    })
}

/// Whether the track has a segment the recorder still writes.
pub(crate) fn has_open_segment(mapper: &Mapper, track: Track) -> bool {
    mapper
        .segments()
        .iter()
        .any(|s| s.track == track.as_str() && s.ended.is_none())
}

/// The meeting sample up to which the track's audio is settled: recorded, or known to be a gap
/// that no later segment can fill.
pub(crate) fn track_horizon(
    mapper: &Mapper,
    track: Track,
    reader: &TrackReader,
    stopped: bool,
) -> u64 {
    let end = reader.end();
    if stopped || has_open_segment(mapper, track) {
        end
    } else {
        end.max(mapper.latest_t().saturating_sub(SETTLE_MS) * PER_MS)
    }
}

/// Whether the user dictates now, and recognition should wait.
pub(crate) fn dictating(mapper: &Mapper, yield_ms: u64) -> bool {
    mapper
        .dictating_since()
        .is_some_and(|since| mapper.latest_t().saturating_sub(since) < yield_ms)
}

/// `meta` keys a window sets in the transaction that stores it.
type MetaUpdates = Vec<(String, String)>;

/// The decision 10 consent check: `Err(reason)` keeps transcription waiting.
pub type ConsentCheck = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// How a pipeline transcribes.
#[derive(Clone)]
pub struct PipelineOptions {
    /// The meeting's recognizer.
    pub stt: Arc<dyn SpeechToText>,
    /// The same recognizer with word timings, when it has them, as Orukeet does. It is used
    /// instead of `stt`: windows get context on both sides and passages split at pauses.
    pub timed: Option<Arc<dyn TimedSpeechToText>>,
    pub hints: Hints,
    /// Decision 10: `Err(reason)` while audio may not go to the meeting's provider. Recording
    /// goes on; transcription waits, and [`PipelineState::Waiting`] says why.
    pub transcription_allowed: ConsentCheck,
    pub timing: Timing,
    /// Who labels the speakers on the call. [`Labels::Off`] makes the whole call `call-1`, as in
    /// Voice 0.1.
    pub labels: Labels,
    /// Whether a meeting that chooses its label mode now gets labels: the Settings switch. A
    /// meeting that has its mode keeps it whatever the switch says now, so `labels` still
    /// carries Nemotron's diarizer when the switch is off.
    pub live_speakers: bool,
    /// Mistral's realtime stream, when the meeting provider streams; it then transcribes both
    /// tracks and labels the call instead of `stt`.
    pub stream: Option<StreamOptions>,
    /// How the speaker and stream followers poll, push and retry.
    pub speakers: FollowerTiming,
}

impl PipelineOptions {
    /// Transcribes with `stt`, always allowed, without hints or speaker labels.
    pub fn new(stt: Arc<dyn SpeechToText>) -> PipelineOptions {
        PipelineOptions {
            stt,
            timed: None,
            hints: Hints::default(),
            transcription_allowed: Arc::new(|| Ok(())),
            timing: Timing::default(),
            labels: Labels::Off {
                reason: "off".into(),
            },
            live_speakers: true,
            stream: None,
            speakers: FollowerTiming::default(),
        }
    }

    /// Transcribes on this computer with Orukeet, using its word timings.
    pub fn local(engine: LocalEngine) -> PipelineOptions {
        PipelineOptions {
            timed: Some(Arc::new(engine.clone())),
            ..PipelineOptions::new(Arc::new(engine))
        }
    }
}

impl fmt::Debug for PipelineOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let labels = match &self.labels {
            Labels::Off { reason } => format!("off ({reason})"),
            Labels::Local { .. } => "nemotron".to_owned(),
        };
        f.debug_struct("PipelineOptions")
            .field("timed", &self.timed.is_some())
            .field("hints", &self.hints)
            .field("timing", &self.timing)
            .field("labels", &labels)
            .field("live_speakers", &self.live_speakers)
            .field("stream", &self.stream.is_some())
            .finish_non_exhaustive()
    }
}

/// How often the pipeline looks and how it retries. The defaults are Ownkey's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Between looks when nothing was ready.
    pub poll: Duration,
    /// Between looks right after a window.
    pub busy_poll: Duration,
    /// The first wait after a failed recognition; it doubles up to `retry_max`.
    pub retry_first: Duration,
    pub retry_max: Duration,
    /// After the recording ended, this many failed recognitions in a row make the meeting
    /// `failed`. While it records, the pipeline never gives up.
    pub attempts_after_stop: u32,
}

impl Default for Timing {
    fn default() -> Timing {
        Timing {
            poll: Duration::from_millis(150),
            busy_poll: Duration::from_millis(30),
            retry_first: Duration::from_secs(2),
            retry_max: Duration::from_secs(60),
            attempts_after_stop: 5,
        }
    }
}

/// What a pipeline is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineState {
    Starting,
    /// Up to date with the recording, waiting for more audio.
    Listening,
    /// A recognition call is running.
    Transcribing,
    /// Transcription waits for consent (decision 10); recording goes on.
    Waiting {
        reason: String,
    },
    /// Recognition or the disk failed; the pipeline tries again.
    Retrying {
        error: String,
    },
    /// Every recorded second is transcribed.
    Done,
    /// The recording ended and recognition kept failing. The audio stays; a pipeline started
    /// again later tries once more.
    Failed {
        reason: String,
    },
    /// Shut down, or the meeting is gone.
    Stopped,
}

impl PipelineState {
    /// Whether the pipeline's thread has nothing more to do.
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            PipelineState::Done | PipelineState::Failed { .. } | PipelineState::Stopped
        )
    }
}

/// Starts pipelines.
pub struct Pipeline;

impl Pipeline {
    /// Follows `dir` on a thread of its own until its timeline has `stop` and every recorded
    /// second is transcribed.
    ///
    /// Sends every meeting event to `log`: `meeting.started` (from meeting.json), the speakers,
    /// `meeting.state` and `mark` from the timeline, `passage` as windows are transcribed, and
    /// finally `done` (or `failed`). Without speaker labels the speakers are `mic` "You" and
    /// `call-1` "Speaker 1" from the start, as in Voice 0.1; with them, each `call-N` comes right
    /// before the first passage that names it. Start it once the recorder runs; it never touches
    /// the recorder.
    pub fn start(dir: MeetingDir, log: EventLog, opts: PipelineOptions) -> PipelineHandle {
        let shared = Arc::new(Shared::default());
        let meeting = dir.id().to_string();
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name(format!("meeting-{meeting}"))
                .spawn(move || {
                    run(dir, log, opts, &shared);
                    shared.finish();
                })
        };
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(e) => {
                shared.set(PipelineState::Failed {
                    reason: format!("Voice couldn't start transcribing: {e}"),
                });
                shared.finish();
                None
            }
        };
        PipelineHandle {
            meeting,
            shared,
            thread,
        }
    }
}

/// A running pipeline. Dropping it asks the pipeline to stop without waiting.
pub struct PipelineHandle {
    meeting: String,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl PipelineHandle {
    pub fn meeting(&self) -> &str {
        &self.meeting
    }

    pub fn state(&self) -> PipelineState {
        self.shared.state()
    }

    /// What speaker labels are doing for this meeting, for Settings.
    pub fn labels(&self) -> LabelStatus {
        match &*lock(&self.shared.labels) {
            LabelView::Fixed(status) => status.clone(),
            LabelView::Live(follower) => follower.status(),
        }
    }

    /// Looks again now instead of at the next poll, for instance after the user gave consent.
    pub fn wake(&self) {
        self.shared.wake();
    }

    /// Stops following the meeting, for an app that quits or a pipeline that is replaced. The
    /// recorder isn't touched. The speaker and stream followers stop at once: when this returns
    /// they write nothing more, also while a recognition call keeps the pipeline's thread busy.
    /// Waits up to 5 s for such a call; what it returns is still stored.
    pub fn shutdown(mut self) {
        self.shared.request_stop();
        if self.shared.wait_finished(SHUTDOWN_WAIT) {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for PipelineHandle {
    fn drop(&mut self) {
        self.shared.request_stop();
    }
}

impl fmt::Debug for PipelineHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PipelineHandle")
            .field("meeting", &self.meeting)
            .field("state", &self.state())
            .finish()
    }
}

/// Where [`PipelineHandle::labels`] reads from.
enum LabelView {
    Fixed(LabelStatus),
    Live(Arc<FollowerShared>),
}

impl Default for LabelView {
    fn default() -> LabelView {
        LabelView::Fixed(fixed_status("off", LabelState::Off, None))
    }
}

fn fixed_status(mode: &str, state: LabelState, reason: Option<String>) -> LabelStatus {
    LabelStatus {
        mode: mode.to_owned(),
        state,
        reason,
        epoch: 0,
        speakers: 0,
        settled_ms: None,
        worker_pid: None,
    }
}

#[derive(Default)]
struct Shared {
    state: Mutex<Option<PipelineState>>,
    stop: AtomicBool,
    flags: Mutex<Flags>,
    signal: Condvar,
    labels: Mutex<LabelView>,
    /// Every follower this pipeline started, so a stop reaches them without waiting for the
    /// pipeline's thread, which a recognition call can keep busy for minutes.
    followers: Mutex<Vec<Arc<FollowerShared>>>,
}

#[derive(Default)]
struct Flags {
    woken: bool,
    finished: bool,
}

impl Shared {
    fn state(&self) -> PipelineState {
        lock(&self.state).clone().unwrap_or(PipelineState::Starting)
    }

    fn set(&self, state: PipelineState) {
        *lock(&self.state) = Some(state);
    }

    fn set_labels(&self, view: LabelView) {
        *lock(&self.labels) = view;
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Asks the pipeline and its followers to stop. When this returns, no follower writes again.
    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        for follower in lock(&self.followers).iter() {
            follower.request_stop();
        }
        self.wake();
    }

    /// The status of a follower that is about to start, kept so a stop reaches it. A pipeline
    /// asked to stop already stops it before its thread can write.
    fn follower(&self, mode: &str) -> Arc<FollowerShared> {
        let follower = FollowerShared::new(mode);
        let mut followers = lock(&self.followers);
        if self.stopping() {
            follower.request_stop();
        }
        followers.push(follower.clone());
        follower
    }

    fn wake(&self) {
        lock(&self.flags).woken = true;
        self.signal.notify_all();
    }

    /// Sleeps for `wait`, or until woken or asked to stop.
    fn sleep(&self, wait: Duration) {
        let flags = lock(&self.flags);
        let (mut flags, _) = self
            .signal
            .wait_timeout_while(flags, wait, |f| !f.woken && !self.stopping())
            .unwrap_or_else(PoisonError::into_inner);
        flags.woken = false;
    }

    fn finish(&self) {
        {
            let mut state = lock(&self.state);
            if !state.as_ref().is_some_and(PipelineState::is_final) {
                *state = Some(PipelineState::Stopped);
            }
        }
        lock(&self.flags).finished = true;
        self.signal.notify_all();
    }

    fn wait_finished(&self, wait: Duration) -> bool {
        let flags = lock(&self.flags);
        let (flags, _) = self
            .signal
            .wait_timeout_while(flags, wait, |f| !f.finished)
            .unwrap_or_else(PoisonError::into_inner);
        flags.finished
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn run(dir: MeetingDir, log: EventLog, opts: PipelineOptions, shared: &Shared) {
    let timing = opts.timing.clone();
    let mut errors = 0u32;
    let mut worker = loop {
        if shared.stopping() {
            return;
        }
        match Worker::open(dir.clone(), log.clone(), &opts) {
            Ok(Some(worker)) => break worker,
            Ok(None) => return,
            Err(e) => {
                errors += 1;
                shared.set(PipelineState::Retrying {
                    error: e.to_string(),
                });
                shared.sleep(backoff(&timing, errors));
            }
        }
    };
    errors = 0;
    while !shared.stopping() {
        match worker.step(shared) {
            Ok(Step::Progress) => {
                errors = 0;
                shared.sleep(timing.busy_poll);
            }
            Ok(Step::Idle) => {
                errors = 0;
                shared.sleep(timing.poll);
            }
            Ok(Step::Finished) | Ok(Step::Gone) => break,
            Err(e) => {
                // A full disk or a locked file: the recording is safe, so try again later.
                errors += 1;
                shared.set(PipelineState::Retrying {
                    error: e.to_string(),
                });
                shared.sleep(backoff(&timing, errors));
            }
        }
    }
    worker.stop_followers();
}

fn backoff(timing: &Timing, attempts: u32) -> Duration {
    let doublings = attempts.saturating_sub(1).min(16);
    timing
        .retry_first
        .saturating_mul(1 << doublings)
        .min(timing.retry_max)
}

#[derive(Debug)]
enum Error {
    Db(rusqlite::Error),
    Log(LogError),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(e) => write!(f, "the transcript store failed: {e}"),
            Error::Log(e) => write!(f, "{e}"),
            Error::Io(e) => write!(f, "the meeting folder can't be read: {e}"),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Db(e)
    }
}

impl From<LogError> for Error {
    fn from(e: LogError) -> Error {
        Error::Log(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

enum Step {
    Progress,
    Idle,
    Finished,
    Gone,
}

enum Advance {
    /// A window or a gap was stored.
    Moved,
    /// The track needs more audio first.
    Waiting,
    /// The recording ended and every second of the track is transcribed.
    Complete,
    /// Recognition failed; wait before the next try.
    Backoff,
    /// Recognition failed too often after the recording ended.
    GiveUp(String),
}

/// Failed recognitions in a row.
struct Failure {
    attempts: u32,
    after_stop: u32,
    until: Instant,
}

/// Who labels the call's speakers in this pipeline.
enum Run {
    /// `begin` hasn't decided yet.
    Undecided,
    /// The whole call is `call-1`, as in Voice 0.1.
    Off,
    /// Nemotron on this computer. `follower` is `None` when none runs: the meeting finished
    /// before, or (`lost`) no diarizer can run now, which counts as a failed follower.
    Nemotron {
        follower: Option<FollowerHandle>,
        lost: bool,
    },
    /// Mistral's realtime stream, one follower per track. `lost` when this pipeline has no stream
    /// for it: the rest is transcribed with the pipeline's recognizer, as after a failed
    /// follower.
    Mistral {
        followers: Vec<(Track, FollowerHandle)>,
        diarize: bool,
        lost: bool,
    },
}

/// A track's gate: how far its windows may go.
enum Gate {
    /// As far as the audio goes.
    Open,
    /// Up to this meeting sample.
    At(u64),
    /// As far as the audio goes for now, because the follower stopped moving; the track isn't
    /// complete until the follower finishes or fails.
    Stalled,
    /// The stream failed after the recording ended.
    Failed(String),
}

/// The speaker of a track's passages without speaker labels. Voice 0.1 labels the whole call as
/// one speaker, which stays the rule for meetings whose label mode is `off`.
pub fn speaker_of(track: Track) -> Speaker {
    let (id, label) = match track {
        Track::Mic => ("mic", "You"),
        Track::Call => ("call-1", "Speaker 1"),
    };
    Speaker {
        id: id.to_owned(),
        track,
        label: label.to_owned(),
    }
}

/// A labelled call speaker: `call-N` is "Speaker N".
fn call_speaker(id: &str) -> Speaker {
    Speaker {
        id: id.to_owned(),
        track: Track::Call,
        label: format!("Speaker {}", call_number(id).unwrap_or(1)),
    }
}

fn passage_key(id: &str, revision: u64) -> String {
    format!("passage:{id}:{revision}")
}

/// The key of an event already in the log, as the pipeline would have made it.
fn event_key(event: &str, data: &Value) -> Option<String> {
    let text = |key: &str| data[key].as_str();
    Some(match event {
        events::MEETING_STARTED => "started".to_owned(),
        events::MEETING_STATE => {
            state_key(MeetingState::parse(text("state")?)?, data["at"].as_u64()?)
        }
        events::SPEAKER => format!("speaker:{}", text("id")?),
        events::PASSAGE => passage_key(text("id")?, data["revision"].as_u64()?),
        events::MARK => mark_key(
            text("id")?,
            text("kind").unwrap_or_default(),
            data["until"].as_u64(),
        ),
        events::FRAME => format!("frame:{}", text("id")?),
        _ => return None,
    })
}

fn to_data(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("protocol types serialize")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `epoch:n` of a stream segment.
fn parse_taken(value: Option<String>) -> (u64, u64) {
    value
        .and_then(|v| {
            let (e, n) = v.split_once(':')?;
            Some((e.parse().ok()?, n.parse().ok()?))
        })
        .unwrap_or((0, 0))
}

struct Worker {
    dir: MeetingDir,
    id: String,
    file: MeetingFile,
    log: EventLog,
    transcript: Transcript,
    emitted: HashSet<String>,
    watcher: TimelineWatcher,
    recognizer: Recognizer,
    hints: Hints,
    allowed: ConsentCheck,
    timing: Timing,
    tracks: Vec<Track>,
    /// Samples of segments that ended, which can't change any more.
    lengths: HashMap<(String, u64), u64>,
    /// The dictation stretches the passages were last flagged with.
    stretches: Vec<Stretch>,
    failure: Option<Failure>,
    next_track: usize,
    begun: bool,
    labels: Labels,
    live_speakers: bool,
    stream: Option<StreamOptions>,
    speaker_timing: FollowerTiming,
    run: Run,
    /// The Nemotron follower's status at the start of this step, so one step sees one state.
    snapshot: Option<Snapshot>,
    /// The screen journal's follower, for a meeting that records the screen.
    screen: Option<ScreenFollower>,
    /// Screen events the follower has passed but the log doesn't have yet, oldest first. The
    /// follower never gives an event twice, so one whose save failed waits here for the next
    /// step, and the meeting isn't done while any wait.
    screen_pending: VecDeque<ScreenDerived>,
}

impl Worker {
    /// `None` when the meeting is gone.
    fn open(
        dir: MeetingDir,
        log: EventLog,
        opts: &PipelineOptions,
    ) -> Result<Option<Worker>, Error> {
        if !dir.path().is_dir() {
            return Ok(None);
        }
        let file = dir.meeting()?;
        let mut transcript = Transcript::open(dir.path())?;
        let id = dir.id().to_string();
        let (_, stored) = log.meeting_events(&id)?;
        if stored.iter().any(|e| e.event == events::MEETING_REMOVED) {
            return Ok(None);
        }
        // The log is the record of what Rukoo can get. A key the store remembers but the log
        // lacks (a power cut lost it) is sent again; one the log has but the store doesn't (Voice
        // died between the two writes) isn't.
        let keys: HashMap<String, u64> = stored
            .iter()
            .filter_map(|e| Some((event_key(&e.event, &e.data)?, e.seq)))
            .collect();
        if transcript.emitted()? != keys {
            transcript.set_emitted(&keys)?;
        }
        // The screen folder may first appear after an audio-only meeting is already running.
        let screen = Some(ScreenFollower::new(dir.path()));
        let recognizer = match &opts.timed {
            Some(timed) => Recognizer::Timed(timed.clone()),
            None => Recognizer::Text(opts.stt.clone()),
        };
        Ok(Some(Worker {
            watcher: TimelineWatcher::new(dir.path()),
            tracks: tracks_of(&file.tracks),
            dir,
            id,
            file,
            log,
            transcript,
            emitted: keys.into_keys().collect(),
            recognizer,
            hints: opts.hints.clone(),
            allowed: opts.transcription_allowed.clone(),
            timing: opts.timing.clone(),
            lengths: HashMap::new(),
            stretches: Vec::new(),
            failure: None,
            next_track: 0,
            begun: false,
            labels: opts.labels.clone(),
            live_speakers: opts.live_speakers,
            stream: opts.stream.clone(),
            speaker_timing: opts.speakers.clone(),
            run: Run::Undecided,
            snapshot: None,
            screen,
            screen_pending: VecDeque::new(),
        }))
    }

    /// Sends an event unless the log has it already.
    fn emit(&mut self, key: String, event: &str, data: Value) -> Result<(), Error> {
        if self.emitted.contains(&key) {
            return Ok(());
        }
        match self.log.append(event, Some(&self.id), data) {
            Ok(seq) => {
                // The log has it now, which is what counts (see `open`): when the store's note
                // fails, the step that tries again mustn't send it a second time.
                self.emitted.insert(key.clone());
                self.transcript.record_emitted(&key, seq)?;
            }
            // A bug built an event Rukoo would drop; sending it again can't help.
            Err(e @ (LogError::Invalid(_) | LogError::TooLarge { .. })) => {
                eprintln!("voice-meetings: {}: dropped a {event} event: {e}", self.id);
            }
            Err(e) => return Err(e.into()),
        }
        self.emitted.insert(key);
        Ok(())
    }

    /// Sends a passage, after the speaker it names when that speaker is new to Rukoo.
    fn emit_passage(&mut self, passage: &Passage) -> Result<(), Error> {
        if let Some(id) = passage.speaker.as_deref() {
            if passage.track == Track::Call && call_number(id).is_some() {
                self.emit_speaker_id(id)?;
            }
        }
        self.emit(
            passage_key(&passage.id, passage.revision),
            events::PASSAGE,
            to_data(passage),
        )
    }

    fn send_speaker(&mut self, speaker: Speaker) -> Result<(), Error> {
        self.transcript.save_speaker(&speaker)?;
        self.emit(
            format!("speaker:{}", speaker.id),
            events::SPEAKER,
            to_data(&speaker),
        )
    }

    /// The speaker of a track without labels: `mic`, or `call-1` for the whole call.
    fn emit_speaker(&mut self, track: Track) -> Result<(), Error> {
        self.send_speaker(speaker_of(track))
    }

    /// A labelled call speaker, `call-N` "Speaker N", unless Rukoo has it.
    fn emit_speaker_id(&mut self, id: &str) -> Result<(), Error> {
        if self.emitted.contains(&format!("speaker:{id}")) {
            return Ok(());
        }
        self.send_speaker(call_speaker(id))
    }

    /// Whether the whole call is one speaker, `call-1`.
    fn call_is_one_speaker(&self) -> bool {
        match &self.run {
            Run::Off | Run::Undecided => true,
            Run::Mistral { diarize, .. } => !diarize,
            Run::Nemotron { .. } => false,
        }
    }

    fn call_emitted(&self) -> bool {
        self.emitted.iter().any(|k| k.starts_with("speaker:call-"))
    }

    /// Decides how the call's speakers are labelled, once per meeting: the first pipeline stores
    /// the mode, and every later one keeps it whatever the current options say.
    fn decide(&mut self, shared: &Shared) -> Result<(), Error> {
        // `begin` runs again after an error; the followers it started keep running.
        if !matches!(self.run, Run::Undecided) {
            return Ok(());
        }
        let stored = self.transcript.meta_get(MODE_KEY)?;
        let mode = match stored {
            Some(mode) => mode,
            None => {
                let upgrade = self.transcript.has_passage_on(Track::Call)?;
                let stream_missing = self.stream.is_none()
                    && matches!(&self.labels, Labels::Off { reason } if reason == STREAM_REASON);
                if !upgrade && stream_missing {
                    // The provider streams, but its stream can't be set up yet, as without an
                    // API key: the windows fail with that reason, and a pipeline that has the
                    // stream chooses the mode, so adding the key still streams this meeting.
                    return Ok(());
                }
                let (mode, reason) = if upgrade {
                    // Voice 0.1 transcribed part of this meeting with the whole call as
                    // `call-1`; it finishes that way, so a name given to `call-1` keeps its
                    // meaning.
                    ("off", Some("upgrade".to_owned()))
                } else if self.stream.is_some() {
                    ("mistral", None)
                } else if !self.live_speakers {
                    // Switched off before this meeting chose; one that chose earlier keeps
                    // labelling below.
                    ("off", Some("off".to_owned()))
                } else {
                    match &self.labels {
                        Labels::Off { reason } => ("off", Some(reason.clone())),
                        Labels::Local { .. } if !self.tracks.contains(&Track::Call) => {
                            ("off", Some("noCall".to_owned()))
                        }
                        Labels::Local { .. } => ("nemotron", None),
                    }
                };
                if let Some(reason) = &reason {
                    self.transcript.meta_set(REASON_KEY, reason)?;
                }
                if mode == "mistral" {
                    let diarize = self.stream.as_ref().is_some_and(|s| s.diarize);
                    self.transcript
                        .meta_set(DIARIZE_KEY, if diarize { "1" } else { "0" })?;
                }
                self.transcript.meta_set(MODE_KEY, mode)?;
                mode.to_owned()
            }
        };
        let reason = self.transcript.meta_get(REASON_KEY)?;
        let finished = self
            .transcript
            .outcome()?
            .is_some_and(|o| o.state == MeetingState::Done);
        let failed = self
            .transcript
            .outcome()?
            .is_some_and(|o| o.state == MeetingState::Failed);
        // Nemotron heard the whole call after the stop. The meeting can still wait for its
        // transcription, as for consent, through many Voice starts.
        let labelled = self.transcript.meta_get(LABELLED_KEY)?.as_deref() == Some("1");
        self.run = match mode.as_str() {
            "nemotron" => match &self.labels {
                // No follower for a meeting that ended `done` or `failed`, such as one restored
                // from the bucket, or whose labels are done: its speaker ids stay as stored.
                Labels::Local { .. } if finished || failed || labelled => {
                    shared.set_labels(LabelView::Fixed(fixed_status(
                        "nemotron",
                        LabelState::Finished,
                        None,
                    )));
                    Run::Nemotron {
                        follower: None,
                        lost: false,
                    }
                }
                Labels::Local { diarizer } => {
                    let follower = Follower::start(
                        self.dir.clone(),
                        diarizer.clone(),
                        self.speaker_timing.clone(),
                        shared.follower("nemotron"),
                    );
                    shared.set_labels(LabelView::Live(follower.shared().clone()));
                    Run::Nemotron {
                        follower: Some(follower),
                        lost: false,
                    }
                }
                Labels::Off { reason } => {
                    let lost = !finished && !failed && !labelled;
                    shared.set_labels(LabelView::Fixed(fixed_status(
                        "nemotron",
                        if lost {
                            LabelState::Failed
                        } else {
                            LabelState::Finished
                        },
                        lost.then(|| reason.clone()),
                    )));
                    Run::Nemotron {
                        follower: None,
                        lost,
                    }
                }
            },
            "mistral" => {
                let diarize = self.transcript.meta_get(DIARIZE_KEY)?.as_deref() == Some("1");
                match self.stream.clone() {
                    // The stream transcribes as well as labels, so a meeting that failed gets
                    // it again; a reconnect keeps the ids.
                    Some(mut opts) if !finished => {
                        // Whether the call is labelled was stored with the mode; a Settings
                        // change since then applies to the next meeting.
                        opts.diarize = diarize;
                        let followers: Vec<(Track, FollowerHandle)> = self
                            .tracks
                            .iter()
                            .map(|&track| (track, self.start_stream(shared, track, &opts)))
                            .collect();
                        let shown = followers
                            .iter()
                            .find(|(t, _)| *t == Track::Call)
                            .or(followers.first())
                            .map(|(_, f)| f.shared().clone());
                        if let Some(shown) = shown {
                            shared.set_labels(LabelView::Live(shown));
                        }
                        self.recognizer = Recognizer::Stream;
                        Run::Mistral {
                            followers,
                            diarize,
                            lost: false,
                        }
                    }
                    Some(_) => {
                        self.recognizer = Recognizer::Stream;
                        shared.set_labels(LabelView::Fixed(fixed_status(
                            "mistral",
                            LabelState::Finished,
                            None,
                        )));
                        Run::Mistral {
                            followers: Vec::new(),
                            diarize,
                            lost: false,
                        }
                    }
                    None => {
                        shared.set_labels(LabelView::Fixed(fixed_status(
                            "mistral",
                            LabelState::Failed,
                            Some("stream".into()),
                        )));
                        Run::Mistral {
                            followers: Vec::new(),
                            diarize,
                            lost: true,
                        }
                    }
                }
            }
            _ => {
                shared.set_labels(LabelView::Fixed(fixed_status(
                    "off",
                    LabelState::Off,
                    reason,
                )));
                Run::Off
            }
        };
        Ok(())
    }

    /// A stream follower for one track of a Mistral meeting.
    fn start_stream(&self, shared: &Shared, track: Track, opts: &StreamOptions) -> FollowerHandle {
        StreamFollower::start(
            self.dir.clone(),
            track,
            opts.clone(),
            self.allowed.clone(),
            self.speaker_timing.clone(),
            self.timing.attempts_after_stop,
            shared.follower("mistral"),
        )
    }

    /// The events that come before anything recorded, and passages a pipeline before this one
    /// stored but didn't send.
    fn begin(&mut self, shared: &Shared) -> Result<(), Error> {
        self.emit(
            "started".into(),
            events::MEETING_STARTED,
            self.file.started(),
        )?;
        self.decide(shared)?;
        // Still undecided: a stream that can't be set up yet. Its call speakers come once it can.
        let decided = !matches!(self.run, Run::Undecided);
        for track in self.tracks.clone() {
            if track == Track::Mic || (decided && self.call_is_one_speaker()) {
                self.emit_speaker(track)?;
            }
        }
        // A restarted or restored pipeline sends its call speakers again, before their passages.
        if !self.call_is_one_speaker() {
            for id in self.transcript.speaker_ids()? {
                self.emit_speaker_id(&id)?;
            }
        }
        if matches!(self.run, Run::Off) && self.tracks.contains(&Track::Call) {
            self.unlabelled_to_call_1()?;
        }
        for stored in self.transcript.passages()? {
            self.emit_passage(&stored.passage)?;
        }
        self.begun = true;
        Ok(())
    }

    /// Stops the speaker and stream followers, waiting a little for each.
    fn stop_followers(&mut self) {
        match std::mem::replace(&mut self.run, Run::Undecided) {
            Run::Nemotron {
                follower: Some(f), ..
            } => f.stop(),
            Run::Mistral { followers, .. } => {
                for (_, f) in followers {
                    f.stop();
                }
            }
            _ => {}
        }
    }

    fn step(&mut self, shared: &Shared) -> Result<Step, Error> {
        if !self.dir.path().is_dir() {
            return Ok(Step::Gone);
        }
        if !self.begun {
            self.begin(shared)?;
        }
        for derived in self.watcher.poll()? {
            self.apply(derived)?;
        }
        self.poll_screen()?;
        self.follow_new_tracks(shared)?;
        self.flag_dictation()?;
        self.snapshot = match &self.run {
            Run::Nemotron {
                follower: Some(f), ..
            } => Some(f.shared().snapshot()),
            _ => None,
        };
        self.switch_to_off_if_failed(shared)?;
        self.backfill_speakers()?;
        let stop = self.watcher.mapper().stop().cloned();

        if let Err(reason) = (self.allowed)() {
            shared.set(PipelineState::Waiting { reason });
            return Ok(Step::Idle);
        }
        if self
            .failure
            .as_ref()
            .is_some_and(|f| Instant::now() < f.until)
        {
            return Ok(Step::Idle);
        }
        if stop.is_none() && dictating(self.watcher.mapper(), DICTATION_YIELD_MS) {
            shared.set(PipelineState::Listening);
            return Ok(Step::Idle);
        }

        let count = self.tracks.len();
        let mut complete = true;
        for k in 0..count {
            let i = (self.next_track + k) % count;
            match self.advance(self.tracks[i], stop.is_some(), shared)? {
                Advance::Moved => {
                    self.next_track = (i + 1) % count;
                    if self.failure.is_none() {
                        shared.set(PipelineState::Listening);
                    }
                    return Ok(Step::Progress);
                }
                Advance::Waiting => complete = false,
                Advance::Complete => {}
                Advance::Backoff => return Ok(Step::Idle),
                Advance::GiveUp(reason) => {
                    if let Some(stop) = &stop {
                        if !self.screen_settled(stop)? {
                            return Ok(Step::Idle);
                        }
                        self.fail(stop, reason.clone())?;
                    }
                    shared.set(PipelineState::Failed { reason });
                    return Ok(Step::Finished);
                }
            }
        }
        match stop {
            Some(stop) if complete && self.screen_settled(&stop)? => {
                // The follower finished: name what the last turns settle before `done`.
                self.backfill_speakers()?;
                self.done(&stop)?;
                shared.set(PipelineState::Done);
                Ok(Step::Finished)
            }
            _ => {
                if self.failure.is_none() {
                    shared.set(PipelineState::Listening);
                }
                Ok(Step::Idle)
            }
        }
    }

    fn apply(&mut self, derived: Derived) -> Result<(), Error> {
        let key = derived.key();
        match derived {
            Derived::Mark(mark) => {
                self.transcript.save_mark(&mark)?;
                self.emit(key, events::MARK, to_data(&mark))
            }
            Derived::State(change) => {
                if change.state == MeetingState::Processing && self.transcript.outcome()?.is_none()
                {
                    self.transcript.set_outcome(&Outcome {
                        state: MeetingState::Processing,
                        at: change.at,
                        ended_at: change.ended_at.unwrap_or_else(now_ms),
                        reason: change.reason.clone(),
                        done_at: None,
                    })?;
                }
                self.emit(key, events::MEETING_STATE, to_data(&change))
            }
        }
    }

    /// A track the recorder records that meeting.json didn't name gets followed too, and in a
    /// Mistral meeting streamed, since its windows read only what a stream stored.
    fn follow_new_tracks(&mut self, shared: &Shared) -> Result<(), Error> {
        let seen = tracks_of(
            self.watcher
                .mapper()
                .segments()
                .iter()
                .map(|s| s.track.as_str()),
        );
        for track in seen {
            if !self.tracks.contains(&track) {
                self.tracks.push(track);
                self.tracks.sort();
                if track == Track::Mic || self.call_is_one_speaker() {
                    self.emit_speaker(track)?;
                }
                // A finished meeting runs no followers and starts none.
                let follower = match (&self.run, &self.stream) {
                    (
                        Run::Mistral {
                            followers,
                            diarize,
                            lost: false,
                        },
                        Some(opts),
                    ) if !followers.is_empty() => {
                        let mut opts = opts.clone();
                        opts.diarize = *diarize;
                        Some(self.start_stream(shared, track, &opts))
                    }
                    _ => None,
                };
                if let (Some(follower), Run::Mistral { followers, .. }) = (follower, &mut self.run)
                {
                    if track == Track::Call {
                        shared.set_labels(LabelView::Live(follower.shared().clone()));
                    }
                    followers.push((track, follower));
                }
            }
        }
        Ok(())
    }

    /// Marks the microphone's passages that overlap a dictation stretch, and unmarks them when a
    /// stretch turns out shorter. A changed flag is a new revision.
    fn flag_dictation(&mut self) -> Result<(), Error> {
        let stretches = self.watcher.mapper().stretches();
        if stretches == self.stretches {
            return Ok(());
        }
        self.stretches = stretches;
        for stored in self.transcript.passages()? {
            let mut passage = stored.passage;
            let dictation = self.in_dictation(passage.track, passage.start, passage.end);
            if dictation != passage.dictation {
                passage.dictation = dictation;
                passage.revision += 1;
                self.transcript.update_passage(&passage)?;
                self.emit_passage(&passage)?;
            }
        }
        Ok(())
    }

    /// A Nemotron meeting whose labeller failed before any call speaker reached Rukoo goes on
    /// as a meeting without labels. The gate held the call's windows, so usually no call passage
    /// exists yet; those a stalled labeller let through without a speaker get `call-1` too.
    ///
    /// A Mistral meeting never switches: its stream transcribes as well as labels, and the
    /// recorded windows can't go to a realtime model. One whose stream can't be set up now, as
    /// without an API key, keeps its mode, its windows wait, and a pipeline that has the stream
    /// again streams the rest.
    fn switch_to_off_if_failed(&mut self, shared: &Shared) -> Result<(), Error> {
        let failed = match &self.run {
            Run::Nemotron {
                follower: Some(_), ..
            } => self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.state == LabelState::Failed),
            Run::Nemotron { lost, .. } => *lost,
            _ => false,
        };
        if !failed || self.call_emitted() {
            return Ok(());
        }
        let reason = match (&self.run, &self.labels) {
            (Run::Nemotron { lost: true, .. }, Labels::Off { reason }) => reason.clone(),
            _ => "runtimeFailed".to_owned(),
        };
        if let Some(error) = self.snapshot.as_ref().and_then(|s| s.error.clone()) {
            eprintln!(
                "voice-meetings: {}: speaker labels are off for this meeting: {error}",
                self.id
            );
        }
        self.transcript.meta_set(MODE_KEY, "off")?;
        self.transcript.meta_set(REASON_KEY, &reason)?;
        self.stop_followers();
        self.run = Run::Off;
        self.snapshot = None;
        shared.set_labels(LabelView::Fixed(fixed_status(
            "off",
            LabelState::Failed,
            Some(reason),
        )));
        if self.tracks.contains(&Track::Call) {
            self.emit_speaker(Track::Call)?;
        }
        self.unlabelled_to_call_1()
    }

    /// In a meeting without labels the whole call is `call-1`, also the passages a labeller
    /// that stalled and then failed let through without a speaker. Also run at `begin`, for a
    /// pipeline that stopped between the switch and this.
    fn unlabelled_to_call_1(&mut self) -> Result<(), Error> {
        let call = speaker_of(Track::Call).id;
        for stored in self.transcript.null_call_passages(0, u64::MAX)? {
            let mut passage = stored.passage;
            passage.speaker = Some(call.clone());
            passage.revision += 1;
            self.transcript.update_passage(&passage)?;
            self.emit_passage(&passage)?;
        }
        Ok(())
    }

    /// Names call passages that were committed without a speaker, once the turns over them are
    /// settled and exactly one speaker overlaps them. A named passage never changes its speaker.
    fn backfill_speakers(&mut self) -> Result<(), Error> {
        if !matches!(self.run, Run::Nemotron { .. }) {
            return Ok(());
        }
        let settled = self.transcript.meta_u64(SETTLED_KEY)?.unwrap_or(0);
        let mark = self.transcript.meta_u64(BACKFILL_KEY)?.unwrap_or(0);
        let waiting = self.transcript.null_call_passages(mark, settled)?;
        let (Some(first), Some(last)) = (waiting.first(), waiting.last()) else {
            return Ok(());
        };
        let turns = self.transcript.turns(
            Track::Call,
            first.passage.start.saturating_sub(TURN_MARGIN_MS),
            last.passage.end + TURN_MARGIN_MS,
        )?;
        let mut checked = mark;
        for stored in waiting {
            let mut passage = stored.passage;
            checked = checked.max(passage.end);
            if let Some(id) = passage_speaker(&turns, passage.start, passage.end) {
                passage.speaker = Some(id);
                passage.revision += 1;
                self.transcript.update_passage(&passage)?;
                self.emit_passage(&passage)?;
            }
        }
        self.transcript
            .meta_set(BACKFILL_KEY, &checked.to_string())?;
        Ok(())
    }

    /// Dictation goes into the microphone; the call track carries the other people.
    fn in_dictation(&self, track: Track, start: u64, end: u64) -> bool {
        track == Track::Mic
            && self
                .watcher
                .mapper()
                .stretches()
                .iter()
                .any(|s| s.overlaps(start, end))
    }

    fn reader(&mut self, track: Track) -> TrackReader {
        read_track(
            self.dir.path(),
            track,
            self.watcher.mapper(),
            &mut self.lengths,
        )
    }

    /// How far the track's windows may go now.
    fn gate(&self, track: Track, reader: &TrackReader, stopped: bool) -> Gate {
        match &self.run {
            Run::Nemotron {
                follower: Some(_), ..
            } if track == Track::Call => {
                let Some(snapshot) = &self.snapshot else {
                    return Gate::Open;
                };
                match snapshot.state {
                    LabelState::Finished | LabelState::Failed => Gate::Open,
                    _ => {
                        let settled = snapshot.settled_ms.unwrap_or(0) * PER_MS;
                        // A follower that stopped moving lets transcription go on without it;
                        // those passages get their speaker later.
                        if snapshot.progress.elapsed() >= self.speaker_timing.stall
                            && reader.end() > settled
                        {
                            Gate::Stalled
                        } else {
                            Gate::At(settled)
                        }
                    }
                }
            }
            Run::Mistral { followers, .. } => {
                let Some((_, follower)) = followers.iter().find(|(t, _)| *t == track) else {
                    return Gate::Open;
                };
                let snapshot = follower.shared().snapshot();
                match snapshot.state {
                    LabelState::Finished => Gate::Open,
                    LabelState::Failed if stopped => Gate::Failed(
                        snapshot
                            .error
                            .unwrap_or_else(|| "The streaming transcription failed.".into()),
                    ),
                    _ => Gate::At(snapshot.settled_ms.unwrap_or(0) * PER_MS),
                }
            }
            _ => Gate::Open,
        }
    }

    /// Whether the call's words get their speakers from Nemotron's turns.
    fn labels_by_turns(&self, track: Track) -> bool {
        track == Track::Call && matches!(self.run, Run::Nemotron { .. })
    }

    /// Transcribes the track's next window, or moves past a gap.
    fn advance(&mut self, track: Track, stopped: bool, shared: &Shared) -> Result<Advance, Error> {
        let reader = self.reader(track);
        let horizon = track_horizon(self.watcher.mapper(), track, &reader, stopped);
        let cursor = self.transcript.cursor(track)?;
        let (gate, released) = match self.gate(track, &reader, stopped) {
            Gate::Failed(reason) => return Ok(Advance::GiveUp(reason)),
            Gate::At(g) => (Some(g), false),
            Gate::Stalled => (None, false),
            Gate::Open => (None, true),
        };
        if cursor >= horizon {
            // With labels, `done` also waits for the follower to finish or fail.
            if !(stopped && released) {
                return Ok(Advance::Waiting);
            }
            // A stream's answers that came after the window for their audio had gone, as when
            // the follower padded a pause and counted that audio as answered, have no window
            // after them: they become passages at their own times before the track is done.
            if self.recognizer.is_stream() {
                let (drafts, meta) = self.stream_drafts(track, 0, horizon / PER_MS, true)?;
                if !meta.is_empty() {
                    let passages = drafts
                        .into_iter()
                        .map(|draft| self.passage(track, draft, None))
                        .collect();
                    return self.commit(track, cursor, cursor, "end", passages, &meta);
                }
            }
            return Ok(Advance::Complete);
        }
        if !reader.covered(cursor) {
            let target = match reader.next_audio(cursor) {
                Some(next) => next.min(horizon),
                None if horizon - cursor >= GAP_STEP => horizon,
                None => return Ok(Advance::Waiting),
            };
            return self.commit(track, cursor, target, "gap", Vec::new(), &[]);
        }
        if gate.is_some_and(|g| cursor >= g) {
            return Ok(Advance::Waiting);
        }

        // A window never runs into a gap (a crash, a lost device, a pause): once the gap is
        // settled, the audio before it ends as if the recording ended there.
        let span_end = reader.span_end(cursor, DRIFT_GAP).unwrap_or(horizon);
        let gap_ahead = span_end + DRIFT_GAP <= horizon;
        let limit = if gap_ahead { span_end } else { horizon };

        let timed = self.recognizer.is_timed();
        let left = if timed {
            cursor.saturating_sub(LEFT_CONTEXT as u64)
        } else {
            cursor
        };
        let read_end = (cursor + (CORE_LIMIT + RIGHT_CONTEXT) as u64).min(limit);
        // The window ends by the gate; the audio after it still gives recognition its context.
        let owned_end = gate.map_or(read_end, |g| g.min(read_end));
        let data = reader.read(left, read_end)?;
        let from = (cursor - left) as usize;
        let last = read_end == limit && owned_end == read_end && (stopped || gap_ahead);
        let cuts = self.speaker_cuts(track, cursor, owned_end)?;
        let Some(boundary) =
            next_boundary_with(&data[from..(owned_end - left) as usize], last, &cuts)
        else {
            return Ok(Advance::Waiting);
        };
        let until = from + boundary.end;
        let end = cursor + boundary.end as u64;
        let (start_ms, end_ms) = (cursor / PER_MS, end / PER_MS);

        let mut drafts = Vec::new();
        let mut language = None;
        let mut meta = Vec::new();
        if self.recognizer.is_stream() {
            (drafts, meta) = self.stream_drafts(track, start_ms, end_ms, false)?;
        } else if worth_recognizing(&data[from..until]) {
            shared.set(PipelineState::Transcribing);
            let right = if timed { until + RIGHT_CONTEXT } else { until }.min(data.len());
            match self.recognizer.recognize(&data[..right], &self.hints) {
                Ok(Heard::Timed(t)) => {
                    let seconds = |samples: u64| samples as f64 / RATE as f64;
                    let owned = owned_tokens(
                        &t.tokens,
                        &t.timestamps,
                        &t.durations,
                        seconds(left),
                        seconds(cursor),
                        seconds(end),
                    );
                    drafts = if self.labels_by_turns(track) {
                        let turns = self.window_turns(start_ms, end_ms)?;
                        speaker_passages(&owned, &turns)
                    } else {
                        timed_passages(&owned)
                    };
                    within(&mut drafts, start_ms, end_ms);
                    language = t.language;
                }
                Ok(Heard::Text(text, lang)) => {
                    let mut draft = window_passage(&text, start_ms, end_ms);
                    if self.labels_by_turns(track) {
                        let turns = self.window_turns(start_ms, end_ms)?;
                        if let Some(d) = &mut draft {
                            d.speaker = window_speaker(&turns, start_ms, end_ms);
                        }
                    }
                    drafts.extend(draft);
                    language = lang;
                }
                Err(e) => return Ok(self.recognition_failed(e, stopped, shared)),
            }
            self.failure = None;
        }
        let passages = drafts
            .into_iter()
            .map(|draft| self.passage(track, draft, language.clone()))
            .collect();
        self.commit(
            track,
            cursor,
            end,
            boundary.reason.as_str(),
            passages,
            &meta,
        )
    }

    /// The stored turns around a window, in ms.
    fn window_turns(&self, start_ms: u64, end_ms: u64) -> Result<Vec<Turn>, Error> {
        Ok(self.transcript.turns(
            Track::Call,
            start_ms.saturating_sub(TURN_MARGIN_MS),
            end_ms + TURN_MARGIN_MS,
        )?)
    }

    /// Where the window from `cursor` may be cut because one voice hands over to another, in
    /// samples from the cursor.
    fn speaker_cuts(&self, track: Track, cursor: u64, owned_end: u64) -> Result<Vec<usize>, Error> {
        let (start_ms, end_ms) = (cursor / PER_MS, owned_end / PER_MS);
        if self.labels_by_turns(track) {
            return Ok(turn_cuts(&self.window_turns(start_ms, end_ms)?, cursor));
        }
        match &self.run {
            Run::Mistral {
                diarize: true,
                lost: false,
                ..
            } if track == Track::Call => {
                let map = self.transcript.speaker_map()?;
                let turns: Vec<Turn> = self
                    .transcript
                    .segments(
                        track,
                        start_ms.saturating_sub(TURN_MARGIN_MS),
                        end_ms + TURN_MARGIN_MS,
                    )?
                    .into_iter()
                    .filter_map(|s| {
                        let speaker = map.get(&(s.epoch, s.label.clone()?))?.clone();
                        Some(Turn {
                            speaker,
                            start: s.start,
                            end: s.end,
                        })
                    })
                    .collect();
                Ok(turn_cuts(&turns, cursor))
            }
            _ => Ok(Vec::new()),
        }
    }

    /// A streamed window's passages: the segments that came after the last one a window took,
    /// up to the first whose middle lies past the window, clamped to the window. One that came
    /// late, for time before the window, joins this one. With `rest`, every segment left.
    fn stream_drafts(
        &self,
        track: Track,
        start_ms: u64,
        end_ms: u64,
        rest: bool,
    ) -> Result<(Vec<Draft>, MetaUpdates), Error> {
        let (epoch, n) = parse_taken(self.transcript.meta_get(&taken_key(track))?);
        let mut taken: Vec<StoredSegment> = Vec::new();
        for s in self.transcript.segments_after(track, epoch, n)? {
            if !rest && (s.start + s.end) / 2 >= end_ms {
                break;
            }
            taken.push(s);
        }
        let Some(last) = taken.last() else {
            return Ok((Vec::new(), Vec::new()));
        };
        let meta = vec![(taken_key(track), format!("{}:{}", last.epoch, last.n))];
        let diarize = matches!(self.run, Run::Mistral { diarize: true, .. });
        let map = if track == Track::Call && diarize {
            self.transcript.speaker_map()?
        } else {
            HashMap::new()
        };
        let drafts = segment_passages(&taken, start_ms, end_ms, |s| {
            map.get(&(s.epoch, s.label.clone()?)).cloned()
        });
        Ok((drafts, meta))
    }

    fn passage(&self, track: Track, draft: Draft, language: Option<String>) -> StoredPassage {
        let speaker = match track {
            Track::Mic => Some(speaker_of(Track::Mic).id),
            Track::Call if self.call_is_one_speaker() => Some(speaker_of(Track::Call).id),
            Track::Call => draft.speaker.clone(),
        };
        StoredPassage {
            passage: Passage {
                // The store gives it the id that goes with its start.
                id: String::new(),
                track,
                speaker,
                start: draft.start,
                end: draft.end,
                dictation: self.in_dictation(track, draft.start, draft.end),
                text: draft.text,
                revision: 1,
                language,
            },
            tokens: draft.tokens,
        }
    }

    fn commit(
        &mut self,
        track: Track,
        start: u64,
        end: u64,
        reason: &str,
        passages: Vec<StoredPassage>,
        meta: &[(String, String)],
    ) -> Result<Advance, Error> {
        // `None`: another pipeline moved the cursor first; the next look reads it again.
        if let Some(stored) = self
            .transcript
            .commit_window_with(track, start, end, reason, passages, meta)?
        {
            for p in stored {
                self.emit_passage(&p.passage)?;
            }
        }
        Ok(Advance::Moved)
    }

    fn recognition_failed(&mut self, e: ProviderError, stopped: bool, shared: &Shared) -> Advance {
        let failure = self.failure.get_or_insert(Failure {
            attempts: 0,
            after_stop: 0,
            until: Instant::now(),
        });
        failure.attempts += 1;
        if stopped {
            failure.after_stop += 1;
        }
        failure.until = Instant::now() + backoff(&self.timing, failure.attempts);
        let message = if e.message.trim().is_empty() {
            "The speech recognizer failed.".to_owned()
        } else {
            e.message
        };
        if stopped && failure.after_stop >= self.timing.attempts_after_stop {
            return Advance::GiveUp(message);
        }
        shared.set(PipelineState::Retrying { error: message });
        Advance::Backoff
    }

    fn done(&mut self, stop: &Stop) -> Result<(), Error> {
        let change = MeetingStateChange {
            state: MeetingState::Done,
            at: stop.t,
            ended_at: Some(stop.wall),
            reason: None,
        };
        self.emit(
            state_key(MeetingState::Done, stop.t),
            events::MEETING_STATE,
            to_data(&change),
        )?;
        let finished = self.transcript.outcome()?;
        // A pipeline that runs again over a finished meeting keeps the first time it was done,
        // which the retention clock counts from.
        if finished
            .as_ref()
            .is_none_or(|o| o.state != MeetingState::Done)
        {
            self.transcript.set_outcome(&Outcome {
                state: MeetingState::Done,
                at: stop.t,
                ended_at: stop.wall,
                reason: Some(stop.reason.clone()),
                done_at: Some(now_ms()),
            })?;
        }
        Ok(())
    }

    fn fail(&mut self, stop: &Stop, reason: String) -> Result<(), Error> {
        let change = MeetingStateChange {
            state: MeetingState::Failed,
            at: stop.t,
            ended_at: Some(stop.wall),
            reason: Some(reason.clone()),
        };
        self.emit(
            state_key(MeetingState::Failed, stop.t),
            events::MEETING_STATE,
            to_data(&change),
        )?;
        self.transcript.set_outcome(&Outcome {
            state: MeetingState::Failed,
            at: stop.t,
            ended_at: stop.wall,
            reason: Some(reason),
            done_at: None,
        })?;
        Ok(())
    }
}

/// The screen journal (RUK-108): frames and screen marks flow while the meeting records and while
/// transcription waits, and a meeting is `done` or `failed` only once its screen part is settled.
impl Worker {
    /// Sends what the screen journal says since the last look, after the events an earlier step
    /// couldn't store.
    fn poll_screen(&mut self) -> Result<(), Error> {
        self.flush_screen()?;
        if let Some(screen) = self.screen.as_mut() {
            let read = screen.poll()?;
            self.screen_pending.extend(read);
        }
        self.flush_screen()
    }

    /// Sends the waiting screen events in order. One that fails stays first in line, with every
    /// later one behind it, for the next step.
    fn flush_screen(&mut self) -> Result<(), Error> {
        while let Some(next) = self.screen_pending.front().cloned() {
            self.emit_screen(next)?;
            self.screen_pending.pop_front();
        }
        Ok(())
    }

    /// Whether the screen part of a stopped meeting is over: at once without a screen, else once
    /// the journal has `stop`, or 60 s after the timeline's stop without a live screen process,
    /// and every screen event is in the log. When it turns true, the marks that close the
    /// screen part went out first.
    fn screen_settled(&mut self, stop: &Stop) -> Result<bool, Error> {
        let Some(screen) = self.screen.as_mut() else {
            return Ok(true);
        };
        // The follower gives its closing marks once, so they wait in line like the rest.
        let Some(closing) = screen.settle(stop.t) else {
            return Ok(false);
        };
        self.screen_pending.extend(closing);
        self.flush_screen()?;
        Ok(true)
    }

    fn emit_screen(&mut self, derived: ScreenDerived) -> Result<(), Error> {
        let key = derived_key(&derived);
        match derived {
            ScreenDerived::Frame(frame) => self.emit(key, events::FRAME, to_data(&frame)),
            ScreenDerived::Mark(mark) => {
                self.transcript.save_mark(&mark)?;
                self.emit(key, events::MARK, to_data(&mark))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_limit() {
        let timing = Timing::default();
        assert_eq!(backoff(&timing, 1), Duration::from_secs(2));
        assert_eq!(backoff(&timing, 3), Duration::from_secs(8));
        assert_eq!(backoff(&timing, 40), Duration::from_secs(60));
    }

    #[test]
    fn events_already_in_the_log_get_the_keys_the_pipeline_gives_them() {
        let passage = serde_json::json!({"id":"mic-000001000","revision":2});
        assert_eq!(
            event_key("passage", &passage).as_deref(),
            Some("passage:mic-000001000:2")
        );
        let state = serde_json::json!({"state":"paused","at":4000});
        assert_eq!(
            event_key("meeting.state", &state).as_deref(),
            Some("state:paused:4000")
        );
        assert_eq!(
            event_key("mark", &serde_json::json!({"id":"k-1"})).as_deref(),
            Some("mark:k-1")
        );
        let ended = serde_json::json!({"id":"silent-call-30","kind":"silent","until":41000});
        assert_eq!(
            event_key("mark", &ended).as_deref(),
            Some("mark:silent-call-30:until:41000")
        );
        assert_eq!(
            event_key("frame", &serde_json::json!({"id":"f-000412000"})).as_deref(),
            Some("frame:f-000412000")
        );
        assert_eq!(event_key("status", &serde_json::json!({})), None);
        assert_eq!(event_key("passage", &serde_json::json!({"id": 1})), None);
    }

    #[test]
    fn the_speakers_of_voice_0_1() {
        assert_eq!(speaker_of(Track::Mic).label, "You");
        assert_eq!(speaker_of(Track::Call).id, "call-1");
        assert_eq!(speaker_of(Track::Call).label, "Speaker 1");
    }

    #[test]
    fn labelled_call_speakers_are_numbered() {
        let speaker = call_speaker("call-10");
        assert_eq!(speaker.label, "Speaker 10");
        assert_eq!(speaker.track, Track::Call);
        assert_eq!(parse_taken(Some("3:17".into())), (3, 17));
        assert_eq!(parse_taken(Some("x".into())), (0, 0));
        assert_eq!(parse_taken(None), (0, 0));
    }
}

/// A screen event whose save fails goes out on a later step, with every event read after it, and
/// the meeting isn't done while one waits.
#[cfg(test)]
mod screen_retry_tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::Path;

    use rusqlite::Connection;
    use serde_json::json;
    use voice_audio::SegmentWriter;
    use voice_core::Recognized;
    use voice_protocol::StartedBy;

    use super::*;
    use crate::library::{CreateMeeting, Library};
    use crate::test_support::TempDir;

    struct Silence;

    impl SpeechToText for Silence {
        fn transcribe(&self, _pcm: &[i16], _hints: &Hints) -> Result<Recognized, ProviderError> {
            Ok(Recognized {
                text: String::new(),
                language: None,
            })
        }
    }

    fn append(path: &Path, lines: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for line in lines {
            file.write_all(format!("{line}\n").as_bytes()).unwrap();
        }
    }

    /// Takes a table of transcript.db away from the pipeline, as a failing disk would: every
    /// write to it fails until it is back.
    fn hide(meeting: &Path, table: &str) {
        Connection::open(meeting.join("transcript.db"))
            .unwrap()
            .execute_batch(&format!("ALTER TABLE {table} RENAME TO {table}_away"))
            .unwrap();
    }

    fn restore(meeting: &Path, table: &str) {
        Connection::open(meeting.join("transcript.db"))
            .unwrap()
            .execute_batch(&format!("ALTER TABLE {table}_away RENAME TO {table}"))
            .unwrap();
    }

    /// Steps until the worker stops making progress, and returns how that step ended.
    fn run(worker: &mut Worker, shared: &Shared) -> Result<Step, Error> {
        for _ in 0..500 {
            match worker.step(shared) {
                Ok(Step::Progress) => continue,
                other => return other,
            }
        }
        panic!("the worker never stopped making progress");
    }

    fn ids(log: &EventLog, meeting: &str, event: &str) -> Vec<String> {
        log.meeting_events(meeting)
            .unwrap()
            .1
            .into_iter()
            .filter(|e| e.event == event)
            .map(|e| e.data["id"].as_str().unwrap().to_owned())
            .collect()
    }

    fn frame(t: u64) -> Value {
        json!({"kind":"frame","t":t,"id":format!("f-{t:09}"),"file":format!("screen/frames/f-{t:09}.jpg"),"width":1920,"height":1080,"text":"Slide","ocr":"ok","language":null})
    }

    #[test]
    fn screen_events_wait_out_a_failed_save() {
        let tmp = TempDir::new("pipeline-screen-retry");
        let lib = Library::open(tmp.path()).unwrap();
        let log = EventLog::open(tmp.path()).unwrap();
        let dir = lib
            .create(&CreateMeeting {
                title: "Retry".into(),
                tracks: vec![Track::Mic],
                sources: Default::default(),
                screen: ScreenMode::Frames,
                started_by: StartedBy::Voice,
                context: None,
            })
            .unwrap();
        let id = dir.id().to_string();
        let meeting = dir.path().to_path_buf();

        // Two seconds of silence on the microphone, and the meeting still records.
        let timeline = meeting.join("timeline.jsonl");
        append(
            &timeline,
            &[
                json!({"kind":"session","t":0,"wall":1_793_000_000_000u64,"session":1,"pid":1,"resumed":false,"version":1}),
            ],
        );
        let file = "audio/mic/000001.wav";
        fs::create_dir_all(meeting.join("audio/mic")).unwrap();
        let mut writer = SegmentWriter::create(&meeting.join(file)).unwrap();
        writer.append(&vec![0i16; 16_000 * 2]).unwrap();
        let samples = writer.close().unwrap();
        append(
            &timeline,
            &[
                json!({"kind":"segment","t":0,"track":"mic","seq":1,"file":file}),
                json!({"kind":"segment_end","t":samples / 16,"track":"mic","seq":1,"samples":samples}),
            ],
        );
        let journal = voice_screen_files::paths::journal_path(&meeting);
        append(
            &journal,
            &[
                json!({"kind":"session","t":40,"wall":1,"session":1,"pid":7,"resumed":false,"version":1,"mode":"frames","anchor":"boot","screen":"0.1.0"}),
            ],
        );

        let mut worker = Worker::open(
            dir.clone(),
            log.clone(),
            &PipelineOptions::new(Arc::new(Silence)),
        )
        .unwrap()
        .unwrap();
        // No screen process runs here, so the screen settles as soon as the recording stops.
        worker.screen = Some(ScreenFollower::with_wait(&meeting, Duration::ZERO));
        let shared = Shared::default();
        assert!(matches!(run(&mut worker, &shared), Ok(Step::Idle)));

        // One read of the journal: a frame, a mark that can't be saved, a frame, a lock.
        hide(&meeting, "marks");
        append(
            &journal,
            &[
                frame(500),
                json!({"kind":"dropped","t":900,"what":"video","reason":"encoder"}),
                frame(1200),
                json!({"kind":"lock","t":1500}),
            ],
        );
        assert!(worker.step(&shared).is_err(), "the mark can't be saved");
        assert!(worker.step(&shared).is_err(), "nor on the next step");
        assert_eq!(ids(&log, &id, "frame"), ["f-000000500"]);
        restore(&meeting, "marks");
        assert!(matches!(run(&mut worker, &shared), Ok(Step::Idle)));
        assert_eq!(
            ids(&log, &id, "frame"),
            ["f-000000500", "f-000001200"],
            "the frame read after the failed mark"
        );
        assert_eq!(ids(&log, &id, "mark"), ["dropped-video"]);

        // The log takes a frame but the note that it did fails: the frame still goes out once.
        hide(&meeting, "emitted");
        append(&journal, &[frame(1700)]);
        assert!(worker.step(&shared).is_err(), "the note can't be saved");
        restore(&meeting, "emitted");
        assert!(matches!(run(&mut worker, &shared), Ok(Step::Idle)));
        assert_eq!(
            ids(&log, &id, "frame"),
            ["f-000000500", "f-000001200", "f-000001700"]
        );

        // The recording stops with the screen locked. The mark that closes the lock can't be
        // saved, and the meeting isn't done without it.
        hide(&meeting, "marks");
        append(
            &timeline,
            &[json!({"kind":"stop","t":2000,"wall":1_793_000_002_000u64,"reason":"user"})],
        );
        let mut failed = 0;
        for _ in 0..500 {
            match worker.step(&shared) {
                Ok(Step::Finished) => panic!("done without the screen's last mark"),
                Err(_) => failed += 1,
                Ok(_) => failed = 0,
            }
            if failed == 3 {
                break;
            }
        }
        assert_eq!(failed, 3, "the closing mark's save never failed");
        restore(&meeting, "marks");
        assert!(matches!(run(&mut worker, &shared), Ok(Step::Finished)));
        let all = log.meeting_events(&id).unwrap().1;
        let seq_of = |what: &dyn Fn(&voice_bridge::Stored) -> bool| {
            all.iter().find(|e| what(e)).map(|e| e.seq)
        };
        let gap = seq_of(&|e| e.event == "mark" && e.data["id"] == "gap-screen-1500")
            .expect("the lock's gap was sent");
        let done = seq_of(&|e| e.event == "meeting.state" && e.data["state"] == "done").unwrap();
        assert!(gap < done, "the lock's gap comes before done");
        assert_eq!(
            ids(&log, &id, "mark"),
            ["dropped-video", "gap-screen-1500"],
            "nothing twice"
        );
    }
}
