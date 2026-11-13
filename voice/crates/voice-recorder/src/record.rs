//! `record`: the recorder thread. It alone writes the timeline and the segments; capture
//! callbacks and control connections only send it events.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use voice_audio::{Resampler, SegmentWriter, SAMPLE_RATE};

use crate::clock::{self, wall_ms, MeetingClock, Stamp, HEARTBEAT};
use crate::control::{self, Command, Reply};
use crate::meeting::{self, RecorderFile};
use crate::silence::{self, SilenceCheck, Verdict};
use crate::source::{Backend, Capture, DeviceInfo, Failure, Opened, Sink, SourceSpec, StreamFlags};
use crate::timeline::{self, samples_to_ms, History, Line, TimelineWriter};
use crate::{token, ExitError, VERSION};

/// A meeting stops by itself after 8 hours.
pub const DEFAULT_LIMIT_MS: u64 = 8 * 60 * 60 * 1000;
/// A new segment starts every 30 s of audio.
pub const DEFAULT_SEGMENT_MS: u64 = 30_000;
const SAMPLES_PER_MS: u64 = SAMPLE_RATE as u64 / 1000;
/// Pending audio goes to disk on every tick, or sooner when this much has piled up, so an
/// append is 100 to 200 ms of audio.
const TICK: Duration = Duration::from_millis(100);
const APPEND_SAMPLES: usize = 2 * 1_600;
/// A healthy loopback track is padded once it trails the clock by this much (Ownkey's
/// PAD_THRESHOLD_SECONDS).
const PAD_THRESHOLD_MS: u64 = 500;
const RETRY: Duration = Duration::from_secs(1);
/// A macOS process tap that fails to open is retried less and less often, up to this: each try
/// creates and destroys a tap and an aggregate device in the audio server.
const TAP_RETRY_MAX: Duration = Duration::from_secs(30);
const DEFAULT_POLL: Duration = Duration::from_secs(2);
const SYNC: Duration = Duration::from_secs(5);
/// A microphone delivers blocks continuously, even of silence; one that stops is gone.
const STALL: Duration = Duration::from_secs(5);
/// A continuing stream's next segment starts where the previous one ended, unless the device's
/// clock has drifted this far from the meeting clock.
const RESYNC_MS: u64 = 100;
/// About 20 s of 10 ms blocks from two tracks.
const EVENT_QUEUE: usize = 4_096;
/// A device that hasn't opened after this long is marked lost until it does, so the gap shows.
/// The open itself goes on: macOS holds the first process tap until the user answers its prompt.
const OPEN_SLOW: Duration = Duration::from_secs(5);
/// What a call track waiting on a slow macOS process tap says in the timeline and the status.
pub const WAITING_FOR_TAP: &str = "Waiting for macOS to start recording system audio. It may be \
     asking whether Rukoo Voice may record it.";
const WAITING: &str = "The device is taking long to open.";
/// How often the silence check asks whether another app plays sound.
const PLAYING_POLL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct RecordOptions {
    pub meeting: PathBuf,
    pub resume: bool,
    pub tracks: Vec<(String, SourceSpec)>,
    pub limit_ms: u64,
    /// Segment length; only tests change it.
    pub segment_ms: u64,
    /// How long the silence check waits; only tests change it.
    pub silence_ms: u64,
}

pub(crate) enum Event {
    Audio {
        track: usize,
        generation: u64,
        at: Instant,
        data: Vec<f32>,
    },
    Failed {
        track: usize,
        generation: u64,
    },
    Command {
        command: Command,
        reply: Sender<Reply>,
        /// When the control port received it.
        at: Instant,
    },
    /// A device open finished on its own thread.
    Opened {
        track: usize,
        generation: u64,
        result: Result<Opened, String>,
    },
}

/// Samples of silence a track needs so its audio reaches `now_ms`; `covered_end` is where its
/// audio ends, in samples since meeting time 0.
///
/// Only a loopback endpoint gets padding: it delivers nothing while nothing plays, and real
/// silence must keep the call track aligned with the microphone (Ownkey `capture.py:414-418`).
/// It stops the moment the device reports an error, and a pause records nothing.
pub fn padding(
    loopback: bool,
    healthy: bool,
    paused: bool,
    covered_end: u64,
    now_ms: u64,
    threshold_ms: u64,
) -> u64 {
    if !loopback || !healthy || paused {
        return 0;
    }
    let deficit = (now_ms * SAMPLES_PER_MS).saturating_sub(covered_end);
    if deficit > threshold_ms * SAMPLES_PER_MS {
        deficit
    } else {
        0
    }
}

/// How long a track waits before it tries its device again after `failures` failed opens in a
/// row: a second, except for a macOS process tap, which doubles that up to `TAP_RETRY_MAX`.
pub fn retry_after(tap: bool, failures: u32) -> Duration {
    if !tap {
        return RETRY;
    }
    RETRY
        .saturating_mul(1 << failures.saturating_sub(1).min(5))
        .min(TAP_RETRY_MAX)
}

/// Where the next segment of a continuing stream starts: right after the previous one, unless
/// the device's clock drifted more than `RESYNC_MS` from the meeting clock's estimate.
pub fn next_segment_start(contiguous: u64, estimate: u64) -> u64 {
    if contiguous.abs_diff(estimate) > RESYNC_MS {
        estimate
    } else {
        contiguous
    }
}

/// Records until `stop`, the length limit or a kill.
pub fn record(opts: RecordOptions) -> Result<(), ExitError> {
    let dir = opts.meeting;
    if !dir.is_dir() {
        return Err(ExitError::usage(format!(
            "meeting folder {} doesn't exist",
            dir.display()
        )));
    }
    if meeting::live_recorder(&dir).is_some() {
        return Err(ExitError::busy());
    }
    let lines = timeline::read(&meeting::timeline_path(&dir))
        .map_err(|e| ExitError::failed(format!("can't read the timeline: {e}")))?;
    let history = History::from_lines(&lines);
    if !history.is_empty() && !opts.resume {
        return Err(ExitError::conflict(
            "the meeting already has a recording; continue it with --resume",
        ));
    }
    if history.stopped {
        return Err(ExitError::conflict(
            "the meeting's recording has stopped; nothing can follow its stop line",
        ));
    }
    let mut journal = Journal(meeting::open_timeline(&dir)?);
    for (name, _) in &opts.tracks {
        fs::create_dir_all(dir.join("audio").join(name))
            .map_err(|e| ExitError::failed(format!("can't create the audio folder: {e}")))?;
    }

    // A resumed session first closes what the dead one left open, so every segment ends.
    let resumed = !history.is_empty();
    let mut problems = Vec::new();
    let repaired = meeting::repair_segments(&dir, history.open_segments(), &mut problems);
    for problem in problems {
        log(format_args!("previous session: {problem}"));
    }
    for s in history.open_segments() {
        let samples = repaired[&(s.track.clone(), s.seq)];
        journal.write(Line::SegmentEnd {
            t: s.t + samples_to_ms(samples),
            track: s.track.clone(),
            seq: s.seq,
            samples,
        });
    }
    let last_seen = history.last_seen(|s| {
        repaired
            .get(&(s.track.clone(), s.seq))
            .copied()
            .or(s.ended)
            .unwrap_or(0)
    });
    // The app that dictated read the microphone through a tap of the dead session, so its
    // dictation ended with it.
    if let Some(since) = history.dictating {
        journal.write(Line::Dictation {
            t: last_seen.max(since),
            state: "end".into(),
        });
    }

    let started = Stamp::now();
    let wall = started.wall;
    let start_t = if resumed {
        let first_wall = history.first_wall.unwrap_or(wall);
        clock::resumed_start(first_wall, wall, history.latest_t.max(last_seen))
    } else {
        0
    };
    let listener = control::bind()
        .map_err(|e| ExitError::failed(format!("can't open the control port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ExitError::failed(e.to_string()))?
        .port();
    let token = token::new_token().map_err(|e| ExitError::failed(e.to_string()))?;
    let session = history.sessions + 1;
    // Set up the audio host before the clock starts, so the devices open close to the session
    // start.
    let backend = Arc::new(Backend::prepare());
    let clock = MeetingClock::starting_at(start_t);
    // Read next to the clock's start, so another process can map its own moments onto `t`.
    let boot = clock::boot_100ns();

    journal.write(Line::Session {
        t: start_t,
        wall,
        boot,
        session,
        pid: std::process::id(),
        resumed,
        version: timeline::FORMAT_VERSION,
        recorder: VERSION.into(),
    });
    if resumed {
        journal.write(Line::Crash {
            t: start_t,
            last_seen,
        });
    }
    // The user paused before the previous recorder died; a restart must not record them.
    if history.paused {
        journal.write(Line::Pause { t: start_t });
    }
    journal.sync();
    meeting::write_recorder_file(
        &dir,
        &RecorderFile {
            pid: std::process::id(),
            port,
            token: token.clone(),
            session,
            started_at: wall,
        },
    )
    .map_err(|e| ExitError::failed(format!("can't write recorder.json: {e}")))?;

    let (events, rx) = mpsc::sync_channel(EVENT_QUEUE);
    control::spawn(listener, token, events.clone())
        .map_err(|e| ExitError::failed(format!("can't start the control thread: {e}")))?;

    let tracks = opts
        .tracks
        .into_iter()
        .map(|(name, spec)| Track {
            seq: history
                .last_seq(&name)
                .max(meeting::highest_seq_on_disk(&dir, &name)),
            name,
            spec,
            generation: 0,
            state: State::Lost {
                error: String::new(),
                next_try: Instant::now(),
            },
            device_id: None,
            segment: None,
            pending: Vec::new(),
            pending_end: start_t,
            covered_until: start_t,
            cover_from: None,
            taps: Vec::new(),
            silence: SilenceCheck::new(opts.silence_ms),
            playing: None,
            failures: 0,
            silence_us: 0,
        })
        .collect();
    let mut recorder = Recorder {
        dir,
        journal,
        clock,
        backend,
        events,
        tracks,
        paused: history.paused,
        dictating: false,
        last_mark: history.last_mark,
        limit_ms: opts.limit_ms,
        segment_samples: opts.segment_ms.max(1) * SAMPLES_PER_MS,
        heartbeat: (start_t, started),
        stop: None,
    };
    // The devices open on threads of their own, so the control port answers from the start,
    // also while macOS holds a process tap for its permission prompt.
    for i in 0..recorder.tracks.len() {
        let at = recorder.clock.now();
        recorder.begin_open(i, false, at);
    }
    recorder.run(rx);
    Ok(())
}

/// Runs an open, turning a panic inside it (cpal, Core Audio) into a failed open, so the track
/// retries instead of waiting for an `Opened` that never comes.
fn catching(open: impl FnOnce() -> Result<Opened, String>) -> Result<Opened, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(open)).unwrap_or_else(|panic| {
        let why = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".into());
        Err(format!("Opening the device failed: {why}"))
    })
}

/// A macOS process tap: opening one creates objects in the audio server.
fn is_tap(spec: &SourceSpec) -> bool {
    cfg!(target_os = "macos") && matches!(spec, SourceSpec::DefaultOutput | SourceSpec::Output(_))
}

fn log(message: impl Display) {
    let _ = writeln!(io::stderr(), "rukoo-voice-recorder: {message}");
}

/// The timeline, with write errors logged instead of stopping the recording: audio still goes
/// to disk, and `finalize` can complete the timeline later.
struct Journal(TimelineWriter);

impl Journal {
    fn write(&mut self, line: Line) {
        if let Err(e) = self.0.write(&line) {
            log(format_args!("can't write the timeline: {e}"));
        }
    }

    fn sync(&self) {
        if let Err(e) = self.0.sync() {
            log(format_args!("can't sync the timeline: {e}"));
        }
    }
}

struct Track {
    name: String,
    spec: SourceSpec,
    /// Counts opened streams; events from an earlier stream are ignored.
    generation: u64,
    state: State,
    /// The device the track opened last.
    device_id: Option<String>,
    /// The last segment number used.
    seq: u64,
    segment: Option<Segment>,
    /// Converted audio not yet appended to the segment.
    pending: Vec<i16>,
    /// Meeting time, by the clock, at which the last pending sample arrived.
    pending_end: u64,
    /// Where the track's last segment ended; a new one never starts before it.
    covered_until: u64,
    /// With no segment open, a healthy loopback track is padded from here.
    cover_from: Option<u64>,
    taps: Vec<SyncSender<Vec<u8>>>,
    /// Watches a source that records silence when it isn't allowed to record.
    silence: SilenceCheck,
    /// Whether another app played sound at the last poll, for the silence check.
    playing: Option<bool>,
    /// Failed opens in a row, for `retry_after`.
    failures: u32,
    /// Microseconds of audio the silence check hasn't counted yet.
    silence_us: u64,
}

enum State {
    Open(Box<Live>),
    /// The source is opening on a thread of its own; `Event::Opened` ends this.
    Opening(Opening),
    Lost {
        error: String,
        next_try: Instant,
    },
}

struct Opening {
    /// When the open started, and its meeting time.
    since: Instant,
    at: u64,
    /// The track was without a device before; an open device is `reconnected` then.
    after_loss: bool,
    /// The open took longer than `OPEN_SLOW` and the timeline says the track has no device.
    waiting: Option<String>,
    flags: Arc<StreamFlags>,
}

struct Live {
    _capture: Box<dyn Capture>,
    device: DeviceInfo,
    resampler: Resampler,
    flags: Arc<StreamFlags>,
    last_packet: Instant,
    /// Native frames per second and channels, to measure what a block holds.
    rate: u32,
    channels: u16,
    /// The silence check watches this stream.
    silent_when_denied: bool,
}

struct Segment {
    writer: SegmentWriter,
    seq: u64,
    t: u64,
}

impl Track {
    /// Where the track's audio ends, in samples since meeting time 0.
    fn covered_end(&self) -> Option<u64> {
        match (&self.segment, self.cover_from) {
            (Some(s), _) => {
                Some(s.t * SAMPLES_PER_MS + s.writer.samples() + self.pending.len() as u64)
            }
            (None, Some(from)) => Some(from * SAMPLES_PER_MS),
            (None, None) => None,
        }
    }

    /// Sends converted audio to every tap. A tap that can't keep up is disconnected: dropping
    /// blocks would hand dictation broken audio without a sign. A tap whose client closed it
    /// goes too. Returns whether the track's last tap went.
    fn send_taps(&mut self, pcm: &[i16]) -> bool {
        if self.taps.is_empty() {
            return false;
        }
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for sample in pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.taps.retain(|tap| tap.try_send(bytes.clone()).is_ok());
        self.taps.is_empty()
    }
}

struct Stopping {
    reason: String,
    reply: Option<Sender<Reply>>,
}

struct Schedule {
    tick: Instant,
    heartbeat: Instant,
    poll: Instant,
    sync: Instant,
    playing: Instant,
}

/// The next time of a periodic task; a schedule that fell behind restarts from now.
fn later(previous: Instant, period: Duration, now: Instant) -> Instant {
    let next = previous + period;
    if next <= now {
        now + period
    } else {
        next
    }
}

struct Recorder {
    dir: PathBuf,
    journal: Journal,
    clock: MeetingClock,
    backend: Arc<Backend>,
    events: SyncSender<Event>,
    tracks: Vec<Track>,
    paused: bool,
    /// A dictation stretch is open: the last `dictation` line says `start`.
    dictating: bool,
    last_mark: u64,
    limit_ms: u64,
    segment_samples: u64,
    /// Meeting time and clock readings of the last heartbeat.
    heartbeat: (u64, Stamp),
    stop: Option<Stopping>,
}

impl Recorder {
    fn run(&mut self, rx: Receiver<Event>) {
        let now = Instant::now();
        let mut schedule = Schedule {
            tick: now + TICK,
            heartbeat: now + HEARTBEAT,
            poll: now + DEFAULT_POLL,
            sync: now + SYNC,
            playing: now,
        };
        while self.stop.is_none() {
            let wait = schedule.tick.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(event) => {
                    self.handle(event);
                    // Catch up on everything queued before the timers run, so audio that piled
                    // up during a stall is written before a suspend check closes the segments.
                    for event in rx.try_iter().take(EVENT_QUEUE) {
                        if self.stop.is_some() {
                            break;
                        }
                        self.handle(event);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            let now = Instant::now();
            if self.stop.is_none() && now >= schedule.tick {
                self.tick(now, &mut schedule);
            }
        }
        self.finish();
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Audio {
                track,
                generation,
                at,
                data,
            } => self.on_audio(track, generation, at, &data),
            Event::Failed { track, generation } => {
                if self.tracks[track].generation == generation {
                    self.check_failure(track);
                }
            }
            Event::Command { command, reply, at } => self.on_command(command, reply, at),
            Event::Opened {
                track,
                generation,
                result,
            } => self.on_opened(track, generation, result),
        }
    }

    fn tick(&mut self, now: Instant, schedule: &mut Schedule) {
        schedule.tick = now + TICK;
        let stamp = Stamp::now();
        if clock::slept(&self.heartbeat.1, &stamp).is_some() {
            self.suspend(&stamp);
            schedule.heartbeat = Instant::now() + HEARTBEAT;
        }
        if now >= schedule.playing {
            schedule.playing = now + PLAYING_POLL;
            self.poll_playing();
        }
        for i in 0..self.tracks.len() {
            self.check_failure(i);
            self.check_stall(i, now);
            self.check_opening(i, now);
            self.retry(i, now);
            self.pad(i, PAD_THRESHOLD_MS);
            self.flush_pending(i);
        }
        if now >= schedule.heartbeat {
            schedule.heartbeat = later(schedule.heartbeat, HEARTBEAT, now);
            self.write_heartbeat();
        }
        if now >= schedule.poll {
            schedule.poll = now + DEFAULT_POLL;
            for i in 0..self.tracks.len() {
                self.follow_default(i);
            }
        }
        if now >= schedule.sync {
            schedule.sync = now + SYNC;
            self.sync();
        }
        if self.clock.now() >= self.limit_ms {
            self.stop = Some(Stopping {
                reason: "limit".into(),
                reply: None,
            });
        }
    }

    fn on_audio(&mut self, i: usize, generation: u64, at: Instant, data: &[f32]) {
        let paused = self.paused;
        let arrived = self.clock.at(at);
        let track = &mut self.tracks[i];
        if track.generation != generation {
            return;
        }
        let State::Open(live) = &mut track.state else {
            return;
        };
        live.last_packet = Instant::now();
        if live.silent_when_denied {
            let frames = data.len() as u64 / live.channels.max(1) as u64;
            let us = frames * 1_000_000 / live.rate.max(1) as u64;
            self.check_silence(i, silence::all_zero(data), us, arrived);
        }
        let track = &mut self.tracks[i];
        let State::Open(live) = &mut track.state else {
            return;
        };
        let overrun = live.flags.overrun.swap(false, Ordering::Relaxed);
        if overrun && !paused && track.segment.is_some() {
            // Blocks were dropped before this one: end the segment first, so the gap shows
            // between segments instead of hiding inside one.
            log(format_args!(
                "{}: the recorder fell behind; audio was lost",
                track.name
            ));
            self.close_segment(i, None);
            self.tracks[i].cover_from = Some(self.clock.now());
        }
        let track = &mut self.tracks[i];
        let State::Open(live) = &mut track.state else {
            return;
        };
        let pcm = live.resampler.process(data);
        if pcm.is_empty() {
            return;
        }
        if track.send_taps(&pcm) {
            self.last_tap_gone(i);
        }
        if paused {
            return;
        }
        if self.tracks[i].segment.is_none() {
            let estimate = arrived.saturating_sub(samples_to_ms(pcm.len() as u64));
            let start = estimate.max(self.tracks[i].covered_until);
            if !self.start_segment(i, start) {
                return;
            }
        }
        let track = &mut self.tracks[i];
        track.pending.extend_from_slice(&pcm);
        track.pending_end = arrived;
        if track.pending.len() >= APPEND_SAMPLES {
            self.flush_pending(i);
        }
    }

    /// Marks and dictation are moments the user pointed at, so they keep the time the request
    /// arrived even when this thread is busy. Pause, resume and stop take effect when they run,
    /// which keeps them in line with the segment boundaries.
    fn on_command(&mut self, command: Command, reply: Sender<Reply>, at: Instant) {
        let answer = match command {
            Command::Status => Reply::ok(self.status()),
            Command::Stop { reason } => {
                self.stop = Some(Stopping {
                    reason,
                    reply: Some(reply),
                });
                return;
            }
            Command::Pause if self.paused => Reply::error("not_recording"),
            Command::Pause => Reply::ok(json!({ "ok": true, "t": self.pause() })),
            Command::Resume if !self.paused => Reply::error("not_recording"),
            Command::Resume => Reply::ok(json!({ "ok": true, "t": self.resume() })),
            Command::Mark { label } => {
                self.last_mark += 1;
                let id = format!("k-{}", self.last_mark);
                let t = self.clock.at(at);
                self.journal.write(Line::Mark {
                    t,
                    id: id.clone(),
                    label,
                });
                Reply::ok(json!({ "ok": true, "id": id, "t": t }))
            }
            Command::Dictation { state } => {
                let t = self.clock.at(at);
                // Only a change is written: the stretch may have ended already when the app's
                // tap closed, just before the app's own `end`.
                let starting = state == "start";
                if starting != self.dictating {
                    self.dictating = starting;
                    self.journal.write(Line::Dictation { t, state });
                }
                Reply::ok(json!({ "ok": true, "t": t }))
            }
            Command::Tap { track, pcm } => match self.tracks.iter_mut().find(|t| t.name == track) {
                Some(track) if matches!(track.state, State::Open(_)) => {
                    track.taps.push(pcm);
                    Reply::ok(json!({ "ok": true, "rate": SAMPLE_RATE }))
                }
                // A tap waiting for the device to come back would hold a control connection
                // while sending nothing; the client asks again instead.
                Some(_) => Reply::error("no_device"),
                None => Reply::error("unknown_track"),
            },
        };
        let _ = reply.send(answer);
    }

    fn status(&self) -> Value {
        let tracks: serde_json::Map<String, Value> = self
            .tracks
            .iter()
            .map(|track| {
                let mut value = match &track.state {
                    State::Open(live) => json!({ "device": live.device.name, "ok": true }),
                    // A short open isn't a problem to report; a long one is marked lost.
                    State::Opening(Opening { waiting: None, .. }) => {
                        json!({ "device": null, "ok": true, "opening": true })
                    }
                    State::Opening(Opening {
                        waiting: Some(error),
                        ..
                    }) => json!({ "device": null, "ok": false, "opening": true, "error": error }),
                    State::Lost { error, .. } => {
                        json!({ "device": null, "ok": false, "error": error })
                    }
                };
                if track.silence.silent() {
                    value["silent"] = json!(true);
                }
                if track.silence.heard() {
                    value["heard"] = json!(true);
                }
                (track.name.clone(), value)
            })
            .collect();
        json!({
            "ok": true,
            "state": if self.paused { "paused" } else { "recording" },
            "t": self.clock.now(),
            "tracks": tracks,
        })
    }

    fn pause(&mut self) -> u64 {
        // The pause happens now. Every track is closed at this moment, however long the others take to reach the
        // disk, so a padded call track meets the pause line instead of stopping a sync short of it.
        let at = self.clock.now();
        let mut t = at;
        for i in 0..self.tracks.len() {
            if let Some(end) = self.close_segment(i, Some(at)) {
                t = t.max(end);
            }
            self.tracks[i].cover_from = None;
        }
        self.paused = true;
        self.journal.write(Line::Pause { t });
        t
    }

    fn resume(&mut self) -> u64 {
        let t = self.clock.now();
        self.paused = false;
        for track in &mut self.tracks {
            track.cover_from = Some(t);
            track.covered_until = track.covered_until.max(t);
        }
        self.journal.write(Line::Resume { t });
        t
    }

    /// Starts opening the track's source on a thread of its own. `Event::Opened` brings the
    /// result to `on_opened`; a failure counts from meeting time `at`.
    fn begin_open(&mut self, i: usize, after_loss: bool, at: u64) {
        let track = &mut self.tracks[i];
        track.generation += 1;
        let generation = track.generation;
        let flags = Arc::new(StreamFlags::default());
        let sink = Sink::new(self.events.clone(), i, generation, flags.clone());
        track.state = State::Opening(Opening {
            since: Instant::now(),
            at,
            after_loss,
            waiting: None,
            flags,
        });
        let backend = self.backend.clone();
        let spec = track.spec.clone();
        let events = self.events.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("open-{}", track.name))
            .spawn(move || {
                let result = catching(|| backend.open(&spec, sink));
                // Blocks while the queue is full: the result must not be lost. A recorder that
                // stopped has dropped the receiver, and the stream then stops right here.
                let _ = events.send(Event::Opened {
                    track: i,
                    generation,
                    result,
                });
            });
        if let Err(e) = spawned {
            self.open_failed(i, format!("can't start opening the device: {e}"));
        }
    }

    /// An open finished. One that a later open, a suspend or the stop replaced is dropped, which
    /// stops its stream.
    fn on_opened(&mut self, i: usize, generation: u64, result: Result<Opened, String>) {
        if self.tracks[i].generation != generation
            || !matches!(self.tracks[i].state, State::Opening(_))
        {
            return;
        }
        match result.and_then(|opened| self.opened(i, opened)) {
            Ok(()) => {}
            Err(error) => self.open_failed(i, error),
        }
    }

    /// The open of an `Opening` track failed: a first failure writes `device_lost`, a failed
    /// retry only waits for the next one.
    fn open_failed(&mut self, i: usize, error: String) {
        let State::Opening(opening) = &self.tracks[i].state else {
            return;
        };
        if opening.after_loss || opening.waiting.is_some() {
            let track = &mut self.tracks[i];
            track.failures += 1;
            track.state = State::Lost {
                error,
                next_try: Instant::now() + retry_after(is_tap(&track.spec), track.failures),
            };
        } else {
            let at = opening.at;
            self.mark_lost(i, error, at);
        }
    }

    /// The track's source is open: writes `track`, preceded by `device` when the track got its
    /// device back or moved to another one.
    fn opened(&mut self, i: usize, opened: Opened) -> Result<(), String> {
        let resampler = Resampler::new(opened.rate, opened.channels)
            .ok_or_else(|| "The device reports no sample rate or channels.".to_string())?;
        let t = self.clock.now();
        let track = &mut self.tracks[i];
        let State::Opening(opening) = &track.state else {
            return Ok(());
        };
        let after_loss = opening.after_loss || opening.waiting.is_some();
        let flags = opening.flags.clone();
        let moved = track
            .device_id
            .as_ref()
            .is_some_and(|id| *id != opened.device.id);
        let reason = match (after_loss, moved) {
            (_, true) => Some("default"),
            (true, false) => Some("reconnected"),
            (false, false) => None,
        };
        if let Some(reason) = reason {
            self.journal.write(Line::Device {
                t,
                track: track.name.clone(),
                device: opened.device.name.clone(),
                reason: reason.into(),
            });
        }
        self.journal.write(Line::Track {
            t,
            track: track.name.clone(),
            source: track.spec.to_string(),
            device: opened.device.name.clone(),
            device_id: opened.device.id.clone(),
            native_rate: opened.rate,
            native_channels: opened.channels,
        });
        track.device_id = Some(opened.device.id.clone());
        track.cover_from = (!self.paused).then_some(t);
        track.covered_until = track.covered_until.max(t);
        track.failures = 0;
        track.state = State::Open(Box::new(Live {
            _capture: opened.capture,
            device: opened.device,
            resampler,
            flags,
            last_packet: Instant::now(),
            rate: opened.rate,
            channels: opened.channels,
            silent_when_denied: opened.silent_when_denied,
        }));
        Ok(())
    }

    /// An open that takes longer than `OPEN_SLOW` goes on, but the track counts as lost from
    /// when it started, so the timeline shows the gap and the status says why.
    fn check_opening(&mut self, i: usize, now: Instant) {
        let track = &self.tracks[i];
        let State::Opening(opening) = &track.state else {
            return;
        };
        if opening.waiting.is_some() || now.duration_since(opening.since) < OPEN_SLOW {
            return;
        }
        let tap = cfg!(target_os = "macos")
            && matches!(
                track.spec,
                SourceSpec::DefaultOutput | SourceSpec::Output(_)
            );
        let error = if tap { WAITING_FOR_TAP } else { WAITING };
        let (t, already_lost) = (opening.at.max(track.covered_until), opening.after_loss);
        if let State::Opening(opening) = &mut self.tracks[i].state {
            opening.waiting = Some(error.into());
        }
        // A track that was lost already has its `device_lost` line.
        if !already_lost {
            self.journal.write(Line::DeviceLost {
                t,
                track: self.tracks[i].name.clone(),
                error: error.into(),
            });
        }
        self.drop_taps(i);
    }

    /// Asks whether another app plays sound, for the tracks the silence check still watches.
    fn poll_playing(&mut self) {
        for i in 0..self.tracks.len() {
            let track = &self.tracks[i];
            let watched = matches!(&track.state, State::Open(live) if live.silent_when_denied);
            if watched && !track.silence.heard() {
                self.tracks[i].playing = crate::source::others_playing(&self.tracks[i].spec);
            }
        }
    }

    /// Records that the track has no device from meeting time `at`, or from where its audio ends
    /// if that is later, and schedules the first retry.
    fn mark_lost(&mut self, i: usize, error: String, at: u64) {
        let track = &mut self.tracks[i];
        let t = at.max(track.covered_until);
        track.failures += 1;
        track.state = State::Lost {
            error: error.clone(),
            next_try: Instant::now() + retry_after(is_tap(&track.spec), track.failures),
        };
        track.cover_from = None;
        self.journal.write(Line::DeviceLost {
            t,
            track: track.name.clone(),
            error,
        });
        self.drop_taps(i);
    }

    /// Ends the track's taps: without a device there is nothing to send them, and a client
    /// waiting on a tap would hold one of the control port's connections.
    fn drop_taps(&mut self, i: usize) {
        if self.tracks[i].taps.is_empty() {
            return;
        }
        self.tracks[i].taps.clear();
        self.last_tap_gone(i);
    }

    /// The track's last tap is gone. The app reads the microphone for a dictation through a
    /// tap and closes it when the dictation ends, and also when it quits or crashes, so an open
    /// dictation ends here instead of running on for the rest of the meeting.
    fn last_tap_gone(&mut self, i: usize) {
        if self.dictating && self.tracks[i].name == "mic" {
            self.dictating = false;
            let t = self.clock.now();
            self.journal.write(Line::Dictation {
                t,
                state: "end".into(),
            });
        }
    }

    /// The device failed at meeting time `at`: end the segment, then drop the stream and retry
    /// every second. Closing the segment syncs it to disk, which can take a while; the loss
    /// still counts from `at`.
    fn lose(&mut self, i: usize, error: String, at: u64) {
        self.close_segment(i, None);
        self.mark_lost(i, error, at);
    }

    /// The stream must be rebuilt at meeting time `at`, usually on a new default device.
    fn rebuild(&mut self, i: usize, reason: String, at: u64) {
        self.close_segment(i, None);
        // Drop the old stream before opening the new one.
        self.tracks[i].state = State::Lost {
            error: reason,
            next_try: Instant::now(),
        };
        self.begin_open(i, false, at);
    }

    fn check_failure(&mut self, i: usize) {
        let State::Open(live) = &self.tracks[i].state else {
            return;
        };
        match live.flags.take_failure() {
            Some((Failure::Lost(error), at)) => self.lose(i, error, self.clock.at(at)),
            Some((Failure::Rebuild(reason), at)) => self.rebuild(i, reason, self.clock.at(at)),
            None => {}
        }
    }

    fn check_stall(&mut self, i: usize, now: Instant) {
        let track = &self.tracks[i];
        let State::Open(live) = &track.state else {
            return;
        };
        if !track.spec.is_loopback() && now.duration_since(live.last_packet) > STALL {
            let at = self.clock.at(now);
            self.lose(i, "The device stopped delivering audio.".into(), at);
        }
    }

    fn retry(&mut self, i: usize, now: Instant) {
        let State::Lost { next_try, .. } = &self.tracks[i].state else {
            return;
        };
        if now < *next_try {
            return;
        }
        let at = self.clock.now();
        self.begin_open(i, true, at);
    }

    /// Moves a `default-*` track to the new default device.
    fn follow_default(&mut self, i: usize) {
        let track = &self.tracks[i];
        if !track.spec.follows_default() {
            return;
        }
        let State::Open(live) = &track.state else {
            return;
        };
        let Some(id) = self.backend.default_device_id(&track.spec) else {
            return;
        };
        if id != live.device.id {
            let at = self.clock.now();
            self.rebuild(i, "The default device changed.".into(), at);
        }
    }

    /// The computer slept, as `woke` shows against the last heartbeat: end every segment, move
    /// the clock ahead by the sleep and reopen the devices, which rarely survive a sleep.
    fn suspend(&mut self, woke: &Stamp) {
        let (from, before) = self.heartbeat;
        let mut was_lost = Vec::with_capacity(self.tracks.len());
        for i in 0..self.tracks.len() {
            self.close_segment(i, None);
            // A track that was opening counts as lost when the timeline already says so.
            was_lost.push(match &self.tracks[i].state {
                State::Lost { .. } => true,
                State::Opening(o) => o.after_loss || o.waiting.is_some(),
                State::Open(_) => false,
            });
            self.tracks[i].state = State::Lost {
                error: "The computer was asleep.".into(),
                next_try: Instant::now(),
            };
            self.drop_taps(i);
        }
        let current = self.clock.now();
        self.clock
            .advance_to(clock::after_sleep(from, current, &before, woke));
        let until = self.clock.now();
        self.journal.write(Line::Suspend { t: from, until });
        // The devices open on their own threads, as at the start.
        for (i, lost) in was_lost.into_iter().enumerate() {
            self.begin_open(i, lost, until);
        }
        self.heartbeat = (self.clock.now(), Stamp::now());
    }

    fn pad(&mut self, i: usize, threshold_ms: u64) {
        let now = self.clock.now();
        self.pad_until(i, now, threshold_ms);
    }

    /// Feeds the silence check `ms` of the track's audio up to meeting time `at`, and writes what
    /// it found out.
    fn check_silence(&mut self, i: usize, zero: bool, us: u64, at: u64) {
        let track = &mut self.tracks[i];
        // Whole milliseconds go to the check and the rest waits for the next block, so a block of
        // 512 frames at 48 kHz counts its 10.67 ms and one under a millisecond still adds up.
        track.silence_us += us;
        let ms = track.silence_us / 1000;
        track.silence_us %= 1000;
        let verdict = track.silence.feed(zero, ms, at, track.playing);
        let name = track.name.clone();
        match verdict {
            Some(Verdict::Silent { since }) => {
                log(format_args!(
                    "{name}: only exact zeros since {since} ms while another app plays sound"
                ));
                self.journal.write(Line::Silent {
                    t: at,
                    track: name,
                    since,
                });
            }
            Some(Verdict::Sound) => self.journal.write(Line::Sound { t: at, track: name }),
            None => {}
        }
    }

    fn pad_until(&mut self, i: usize, now: u64, threshold_ms: u64) {
        let track = &self.tracks[i];
        let Some(covered) = track.covered_end() else {
            return;
        };
        let healthy = matches!(track.state, State::Open(_));
        let needed = padding(
            track.spec.is_loopback(),
            healthy,
            self.paused,
            covered,
            now,
            threshold_ms,
        );
        if needed == 0 {
            return;
        }
        if track.segment.is_none() {
            let from = covered / SAMPLES_PER_MS;
            if !self.start_segment(i, from) {
                return;
            }
        }
        // At most one segment's worth per call; the next tick continues.
        let needed = needed.min(self.segment_samples) as usize;
        // A process tap delivers nothing while nothing plays, and a denied one may never deliver:
        // the padding is the silence the check has to hear about (`silence.rs`).
        if matches!(&self.tracks[i].state, State::Open(live) if live.silent_when_denied) {
            self.check_silence(i, true, needed as u64 * 1000 / SAMPLES_PER_MS, now);
        }
        let track = &mut self.tracks[i];
        track.pending.resize(track.pending.len() + needed, 0);
        track.pending_end = now;
        self.flush_pending(i);
    }

    fn start_segment(&mut self, i: usize, t: u64) -> bool {
        loop {
            let track = &mut self.tracks[i];
            track.seq += 1;
            let seq = track.seq;
            let file = meeting::segment_file(&track.name, seq);
            match SegmentWriter::create(&self.dir.join(&file)) {
                Ok(writer) => {
                    track.segment = Some(Segment { writer, seq, t });
                    track.cover_from = None;
                    self.journal.write(Line::Segment {
                        t,
                        track: track.name.clone(),
                        seq,
                        file,
                    });
                    return true;
                }
                // A file left by a session that died before naming it; never overwrite it.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    self.write_failed(i, &e);
                    return false;
                }
            }
        }
    }

    /// Appends pending audio, starting a new segment every 30 s.
    fn flush_pending(&mut self, i: usize) {
        let segment_samples = self.segment_samples;
        loop {
            let track = &mut self.tracks[i];
            let Some(segment) = track.segment.as_mut() else {
                track.pending.clear();
                return;
            };
            if track.pending.is_empty() && segment.writer.samples() < segment_samples {
                return;
            }
            let room = (segment_samples - segment.writer.samples()) as usize;
            let n = room.min(track.pending.len());
            if let Err(e) = segment.writer.append(&track.pending[..n]) {
                self.write_failed(i, &e);
                return;
            }
            track.pending.drain(..n);
            if segment.writer.samples() < segment_samples {
                return;
            }
            let contiguous = segment.t + samples_to_ms(segment_samples);
            let estimate = track
                .pending_end
                .saturating_sub(samples_to_ms(track.pending.len() as u64));
            let next = next_segment_start(contiguous, estimate);
            if next != contiguous {
                // The device delivered more or less audio than the clock says passed, by more
                // than RESYNC_MS: a stretch it never delivered, or a clock that drifts. The next
                // segment starts where its audio was heard (RECORDING.md, `segment_end`).
                log(format_args!(
                    "{}: segment {} ends at {contiguous} ms but its audio arrived until {} ms; \
                     the next one starts there",
                    track.name, segment.seq, estimate,
                ));
            }
            self.end_segment(i);
            if !self.start_segment(i, next) {
                return;
            }
        }
    }

    /// Closes the segment file and writes `segment_end`.
    fn end_segment(&mut self, i: usize) {
        let track = &mut self.tracks[i];
        let Some(segment) = track.segment.take() else {
            return;
        };
        let samples = segment.writer.samples();
        if let Err(e) = segment.writer.close() {
            log(format_args!(
                "{}: can't sync segment {}: {e}",
                track.name, segment.seq
            ));
        }
        let end = segment.t + samples_to_ms(samples);
        track.covered_until = track.covered_until.max(end);
        self.journal.write(Line::SegmentEnd {
            t: end,
            track: track.name.clone(),
            seq: segment.seq,
            samples,
        });
    }

    /// Writes what's pending and ends the segment. With `pad_to`, a healthy loopback track is
    /// first padded up to that meeting time. Returns where the closed segment's audio ends, in ms.
    fn close_segment(&mut self, i: usize, pad_to: Option<u64>) -> Option<u64> {
        // The resampler holds back about a millisecond; it belongs to this segment.
        if let State::Open(live) = &mut self.tracks[i].state {
            let tail = live.resampler.flush();
            if self.tracks[i].segment.is_some() {
                self.tracks[i].pending.extend_from_slice(&tail);
            }
        }
        if let Some(t) = pad_to {
            self.pad_until(i, t, 0);
        }
        let end = self.tracks[i]
            .segment
            .as_ref()
            .and(self.tracks[i].covered_end())
            .map(|samples| samples.div_ceil(SAMPLES_PER_MS));
        self.flush_pending(i);
        self.end_segment(i);
        self.tracks[i].pending.clear();
        end
    }

    /// The disk refused audio: end the segment with what it holds and treat the track like a
    /// lost device, so it retries every second and the gap is marked.
    fn write_failed(&mut self, i: usize, e: &io::Error) {
        let at = self.clock.now();
        self.tracks[i].pending.clear();
        self.end_segment(i);
        self.lose(i, format!("Writing the audio failed: {e}"), at);
    }

    fn write_heartbeat(&mut self) {
        let samples: BTreeMap<String, u64> = self
            .tracks
            .iter()
            .filter_map(|t| Some((t.name.clone(), t.segment.as_ref()?.writer.samples())))
            .collect();
        let stamp = Stamp::now();
        let t = self.clock.now();
        let boot = clock::boot_100ns();
        self.journal.write(Line::Heartbeat {
            t,
            wall: stamp.wall,
            boot,
            samples,
        });
        self.heartbeat = (t, stamp);
    }

    fn sync(&mut self) {
        for track in &self.tracks {
            if let Some(segment) = &track.segment {
                if let Err(e) = segment.writer.sync() {
                    log(format_args!("{}: can't sync: {e}", track.name));
                }
            }
        }
        self.journal.sync();
    }

    fn finish(&mut self) {
        let stopping = self.stop.take().unwrap_or(Stopping {
            reason: "interrupted".into(),
            reply: None,
        });
        // The recording ends here, for every track at once, however long the closing syncs take; stopping the
        // streams afterwards can take a while too.
        let (at, wall) = (self.clock.now(), wall_ms());
        let mut t = at;
        for i in 0..self.tracks.len() {
            if let Some(end) = self.close_segment(i, Some(at)) {
                t = t.max(end);
            }
        }
        for track in &mut self.tracks {
            track.state = State::Lost {
                error: "Stopped.".into(),
                next_try: Instant::now(),
            };
            track.taps.clear();
        }
        self.journal.write(Line::Stop {
            t,
            wall,
            reason: stopping.reason,
        });
        self.journal.sync();
        meeting::remove_recorder_file(&self.dir);
        if let Some(reply) = stopping.reply {
            let (written, done) = mpsc::channel();
            let _ = reply.send(Reply {
                value: json!({ "ok": true, "t": t }),
                written: Some(written),
            });
            let _ = done.recv_timeout(Duration::from_secs(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_a_healthy_silent_loopback_track_past_the_threshold() {
        // 1 s of the call covered, the clock at 1.4 s: within the threshold, no padding yet.
        assert_eq!(padding(true, true, false, 16_000, 1_400, 500), 0);
        // At 1.6 s the track trails by 600 ms and is padded up to the clock.
        assert_eq!(padding(true, true, false, 16_000, 1_600, 500), 9_600);
        // On stop or pause the threshold is 0: any deficit is filled.
        assert_eq!(padding(true, true, false, 16_000, 1_001, 0), 16);
        assert_eq!(padding(true, true, false, 16_000, 1_000, 0), 0);
    }

    #[test]
    fn never_pads_a_microphone_a_failed_device_or_a_pause() {
        // A microphone always delivers; a gap in it is real loss and stays a gap.
        assert_eq!(padding(false, true, false, 0, 10_000, 500), 0);
        // Padding stops the moment the device reports an error.
        assert_eq!(padding(true, false, false, 0, 10_000, 500), 0);
        // A pause records nothing, not even silence.
        assert_eq!(padding(true, true, true, 0, 10_000, 500), 0);
    }

    #[test]
    fn a_track_running_ahead_is_never_trimmed() {
        assert_eq!(padding(true, true, false, 32_000, 1_000, 0), 0);
    }

    #[test]
    fn segments_stay_contiguous_unless_the_device_clock_drifted() {
        assert_eq!(next_segment_start(30_000, 30_000), 30_000);
        assert_eq!(next_segment_start(30_000, 30_060), 30_000);
        assert_eq!(next_segment_start(30_000, 29_950), 30_000);
        assert_eq!(next_segment_start(30_000, 30_150), 30_150);
        assert_eq!(next_segment_start(30_000, 29_850), 29_850);
    }

    #[test]
    fn a_panic_while_opening_is_a_failed_open() {
        let result = catching(|| panic!("the HAL went away"));
        assert_eq!(
            result.err().as_deref(),
            Some("Opening the device failed: the HAL went away")
        );
        let missing = catching(|| Err("not connected".into()));
        assert_eq!(missing.err().as_deref(), Some("not connected"));
    }

    #[test]
    fn a_failing_tap_backs_off_and_a_microphone_does_not() {
        let secs = |f| retry_after(true, f).as_secs();
        assert_eq!(
            (1..=7).map(secs).collect::<Vec<_>>(),
            [1, 2, 4, 8, 16, 30, 30]
        );
        assert_eq!(retry_after(true, 100), TAP_RETRY_MAX);
        assert_eq!(retry_after(false, 100), RETRY);
    }

    #[test]
    fn late_schedules_restart_from_now() {
        let now = Instant::now();
        let period = Duration::from_secs(1);
        assert_eq!(later(now, period, now), now + period);
        let long_ago = now.checked_sub(Duration::from_secs(10));
        if let Some(long_ago) = long_ago {
            assert_eq!(later(long_ago, period, now), now + period);
        }
    }
}
