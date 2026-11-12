//! The engine's shared state and the [`EngineApi`] the app drives.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use voice_bridge::ControlError;
use voice_core::config::{self as core_config, Config};
use voice_core::local::ModelRegistry;
use voice_core::providers::stream;
use voice_core::{Hints, ProviderError, Recognized, SpeechToText};
use voice_input::Key;
use voice_meetings::{Labels, StreamOptions};
use voice_protocol::{ScreenMode, StartArgs, StartedBy};
use voice_screen_files::policy::{Part, Reason};

use crate::api::{
    ApiError, EngineApi, EngineOptions, Folder, Locale, Notification, PillSnapshot, StopReason,
    Theme, TrayState, UiOptions, UiSink,
};
use crate::dictation::Dictation;
use crate::media::MediaService;
use crate::meetings::controller::{Change, NOTIFY_MEETING};
use crate::meetings::screen::ScreenChange;
use crate::meetings::{Meetings, PipelineOptions};
use crate::pill::{notices, Looks, MeetingPhase, PillDriver, DROP_HOLD, MOVED_HOLD};
use crate::platform::CHIME_COOLDOWN;
use crate::services::{Services, Speech};
use crate::settings::SettingsRuntime;
use crate::sync::SyncRuntime;

pub(crate) struct Core {
    pub opts: EngineOptions,
    pub svc: Services,
    pub sink: Arc<dyn UiSink>,
    config: RwLock<Arc<Config>>,
    /// Settings changes one at a time, so two saves can't undo each other.
    writing: Mutex<()>,
    pub pill: PillDriver,
    pub dictation: Dictation,
    pub settings: SettingsRuntime,
    pub meetings: Meetings,
    /// Bucket sync; `None` when its store didn't open, and Voice runs without it.
    pub sync: Option<Arc<voice_sync::BucketSync>>,
    pub sync_rt: SyncRuntime,
    /// `media.open` and `media.read` for the bridge.
    pub media: Arc<MediaService>,
    shutting_down: AtomicBool,
    last_chime: Mutex<Option<Instant>>,
    /// What Settings > Rukoo last showed, to tell when it changes.
    rukoo: Mutex<Value>,
    /// What Settings > Speaker labels last showed about running meetings.
    labels: Mutex<Value>,
    pub settings_seq: AtomicU64,
    /// Settings states go out one at a time, each built after its `seq` was given.
    emitting: Mutex<()>,
    /// The engine itself, for work that outlives a call: set once by `start_with`.
    this: OnceLock<Weak<Core>>,
}

/// What [`Core::new`] gets besides the options and services.
pub(crate) struct Parts {
    pub config: Config,
    pub meetings: Meetings,
    pub settings: SettingsRuntime,
    pub sync: Option<Arc<voice_sync::BucketSync>>,
    pub media: Arc<MediaService>,
}

impl Core {
    pub fn new(opts: EngineOptions, svc: Services, sink: Arc<dyn UiSink>, parts: Parts) -> Core {
        let Parts {
            config,
            meetings,
            settings,
            sync,
            media,
        } = parts;
        let looks = looks(&opts, &svc, &config);
        Core {
            pill: PillDriver::new(looks, Arc::clone(&sink)),
            dictation: Dictation::new(&config, opts.dev_mode),
            config: RwLock::new(Arc::new(config)),
            writing: Mutex::new(()),
            settings,
            meetings,
            sync,
            sync_rt: SyncRuntime::default(),
            media,
            opts,
            svc,
            sink,
            shutting_down: AtomicBool::new(false),
            last_chime: Mutex::new(None),
            rukoo: Mutex::new(Value::Null),
            labels: Mutex::new(Value::Null),
            settings_seq: AtomicU64::new(1),
            emitting: Mutex::new(()),
            this: OnceLock::new(),
        }
    }

    pub fn set_this(self: &Arc<Self>) {
        let _ = self.this.set(Arc::downgrade(self));
    }

    fn this(&self) -> Option<Arc<Core>> {
        self.this.get().and_then(Weak::upgrade)
    }

    pub fn config(&self) -> Arc<Config> {
        Arc::clone(&self.config.read().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    pub fn key_down(&self, key: Key) -> bool {
        match &self.svc.key_state {
            Some(state) => state(key),
            None => voice_input::is_key_down(key),
        }
    }

    /// Where Orukeet is looked for with these settings.
    pub fn registry(&self, cfg: &Config) -> ModelRegistry {
        ModelRegistry::for_config(&self.opts.data_dir, cfg)
            .with_ownkey_models(self.svc.ownkey_models.clone())
    }

    pub fn notify(&self, title_key: &str, body: String) {
        if !self.shutting_down() {
            self.sink.notify(Notification {
                title_key: title_key.to_string(),
                body,
                body_key: None,
            });
        }
    }

    /// A notification whose body has a catalog string; `english` is the fallback.
    pub fn notify_keyed(&self, title_key: &str, body_key: &str, english: &str) {
        if !self.shutting_down() {
            self.sink.notify(Notification {
                title_key: title_key.to_string(),
                body: english.to_string(),
                body_key: Some(body_key.to_string()),
            });
        }
    }

    /// The ready chime, read from the live settings as Ownkey did, at most every 0.2 s.
    pub fn play_chime(&self) {
        if !self.config().ready_chime {
            return;
        }
        let mut last = lock(&self.last_chime);
        if last.is_some_and(|at| at.elapsed() < CHIME_COOLDOWN) {
            return;
        }
        *last = Some(Instant::now());
        drop(last);
        self.svc.chime.play();
    }

    /// Changes Settings: checks, saves `config.json`, then applies the change everywhere. A
    /// failed check or save leaves the old settings in place.
    pub fn commit(
        &self,
        change: impl FnOnce(&mut Config) -> Result<(), ApiError>,
    ) -> Result<(), ApiError> {
        let _writing = lock(&self.writing);
        let old = self.config();
        let mut next = (*old).clone();
        change(&mut next)?;
        if next == *old {
            return Ok(());
        }
        next.save(&self.opts.data_dir)
            .map_err(|e| ApiError::new("failed", format!("Voice couldn't save Settings: {e}")))?;
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(next.clone());
        self.apply(&old, &next);
        self.settings_changed();
        Ok(())
    }

    fn apply(&self, old: &Config, new: &Config) {
        self.dictation.apply(new, self.opts.dev_mode);
        self.svc
            .speech
            .local_configure(self.registry(new), new.idle_timeout());
        // Switching away from Orukeet frees its memory, unless meetings still use it.
        if old.audio_provider == "orukeet"
            && new.audio_provider != "orukeet"
            && new.meeting_audio().provider != "orukeet"
        {
            self.svc.speech.local_unload();
        }
        if old.meeting_audio() != new.meeting_audio() || old.meeting_hints() != new.meeting_hints()
        {
            // The pipelines were built for the old provider; they start again on the new one
            // and continue where transcript.db says. Shutting one down waits for its call.
            if let Some(core) = self.this() {
                let _ = thread::Builder::new()
                    .name("meeting-restart".into())
                    .spawn(move || {
                        let opts = || core.pipeline_options();
                        core.meetings.restart(opts);
                    });
            }
        } else if old.meetings_transcription_consent != new.meetings_transcription_consent {
            // Decision 10: transcription waited for this; it needn't wait for the next poll.
            self.meetings.wake();
        }
        // Keep audio applies to meetings started from now on; each keeps the choice it began with.
        self.meetings
            .library()
            .set_retention(new.meetings_retention);
        // So does the screen; Rukoo learns the new default from the next status.
        if old.meetings_screen != new.meetings_screen {
            self.meetings
                .controller
                .set_screen(screen_mode(new), self.meetings.screen.available());
            self.meetings.publish_status();
        }
        if old.extra.get(crate::sync::SYNC_KEY) != new.extra.get(crate::sync::SYNC_KEY) {
            crate::sync::configure_sync(self, new);
            // Held media may go once the user switches sync off. A changed field or secret only
            // pauses uploads, and the hold stays.
            if crate::sync::switch_on(old) && !crate::sync::switch_on(new) {
                self.sync_rt.sweep_soon();
            }
        }
        let before = looks(&self.opts, &self.svc, old);
        let after = looks(&self.opts, &self.svc, new);
        if before != after || old.pill_hide_from_capture != new.pill_hide_from_capture {
            self.pill.update(|p, _| p.set_looks(after));
            self.sink.ui_changed();
        }
    }

    /// Sends the whole Settings state with a new `seq`. One at a time: a state is built after its
    /// `seq` is given and sent before the next one is, so a state built from older Settings never
    /// carries the newest `seq` (API.md, `seq`). Callers hold none of the locks the state reads.
    pub fn settings_changed(&self) {
        if self.shutting_down() {
            return;
        }
        let _emitting = lock(&self.emitting);
        self.settings_seq.fetch_add(1, Ordering::SeqCst);
        self.sink.settings(crate::settings::state(self));
    }

    pub fn looks(&self) -> Looks {
        looks(&self.opts, &self.svc, &self.config())
    }

    pub fn tray(&self) -> TrayState {
        let recording = self.meetings.controller.current();
        TrayState {
            meeting: recording
                .as_ref()
                .map(|r| if r.paused { "paused" } else { "recording" }.to_string()),
            meeting_at_ms: recording.map(|r| r.at_ms),
            status: None,
        }
    }

    /// A click on the pill's stop button or the stop key: the first asks, the second stops.
    pub fn stop_clicked(self: &Arc<Self>) {
        let stop = self.pill.update(|p, now| p.stop_click(now));
        if !stop {
            return;
        }
        // The click comes from the app's window thread; stopping waits for the recorder.
        let core = Arc::clone(self);
        let _ = thread::Builder::new()
            .name("stop-meeting".into())
            .spawn(move || {
                if let Err(e) = core.meetings.controller.stop(None, "user") {
                    core.notify(
                        crate::meetings::controller::NOTIFY_MEETING,
                        format!("The meeting couldn't be stopped: {}", e.message),
                    );
                }
            });
    }

    /// How meetings are transcribed with the current Settings: the meeting provider (Orukeet
    /// with word timings, or a realtime model's stream), the dictionary as hints, who labels
    /// the speakers on the call, and decision 10's consent gate.
    pub fn pipeline_options(self: &Arc<Self>) -> PipelineOptions {
        let cfg = self.config();
        let audio = cfg.meeting_audio();
        let mut opts = match self.svc.speech.meeting_recognizer(&audio) {
            Ok(opts) => opts,
            // A missing key or model: the pipeline retries while the meeting records and
            // reports this reason if it still fails after the stop.
            Err(error) => PipelineOptions::new(Arc::new(Unavailable(error))),
        };
        opts.hints = cfg.meeting_hints();
        // A development copy logs how long the speaker worker's calls take (spec 7).
        opts.speakers.log_calls = self.opts.dev_mode;
        let labels = meeting_labels(&cfg, self.svc.speech.as_ref());
        opts.labels = labels.labels;
        opts.live_speakers = labels.live_speakers;
        match labels.stream {
            Some(Ok(stream)) => opts.stream = Some(stream),
            // The stream can't be built (no key): the windows fail with that reason, the way a
            // recognizer that can't be built does, and never fall back to another provider. The
            // meeting keeps its label mode open, so a key added later still streams it.
            Some(Err(error)) => {
                opts.stt = Arc::new(Unavailable(error));
                opts.timed = None;
            }
            None => {}
        }
        let weak = Arc::downgrade(self);
        opts.transcription_allowed = Arc::new(move || {
            let Some(core) = weak.upgrade() else {
                return Err("Voice is quitting.".into());
            };
            let cfg = core.config();
            // Built for another provider: a restart with the new one is on its way, and no
            // audio goes to a provider the Settings no longer name.
            if cfg.meeting_audio() != audio {
                return Err("The meeting provider changed; transcription starts again.".into());
            }
            crate::meetings::transcription_allowed(&cfg)
        });
        opts
    }

    /// Transcribes a meeting for Rukoo.
    pub fn follow(self: &Arc<Self>, id: &str) {
        self.meetings.follow(id, self.pipeline_options());
    }

    /// What the meeting controller reports.
    pub fn on_meeting(self: &Arc<Self>, change: Change) {
        match change {
            Change::Started { id, at_ms } => {
                self.pill
                    .update(|p, now| p.meeting(&id, MeetingPhase::Recording, at_ms, now));
                // The recorder runs; the screen process follows on its own thread, after the
                // start handshake for a meeting Voice started.
                self.meetings.screen.begin(&id);
                self.pill_screen(&id);
                self.follow(&id);
                self.nudge_sync();
                self.meeting_changed();
            }
            Change::Reattached { id, paused, at_ms } => {
                let phase = if paused {
                    MeetingPhase::Paused
                } else {
                    MeetingPhase::Recording
                };
                self.pill.update(|p, now| p.meeting(&id, phase, at_ms, now));
                self.meetings.screen.reattach(&id);
                self.pill_screen(&id);
                self.follow(&id);
                self.meeting_changed();
            }
            Change::Paused { id, at_ms } => {
                self.pill
                    .update(|p, now| p.meeting(&id, MeetingPhase::Paused, at_ms, now));
                self.screen_paused(&id, true);
                self.meeting_changed();
            }
            Change::Resumed { id, at_ms } => {
                self.pill
                    .update(|p, now| p.meeting(&id, MeetingPhase::Recording, at_ms, now));
                self.screen_paused(&id, false);
                self.meeting_changed();
            }
            Change::Clock { at_ms, mic_ok, .. } => {
                self.pill.update(|p, now| {
                    p.meeting_clock(at_ms, now);
                    p.meeting_mic(mic_ok, now);
                });
                self.rukoo_changed();
            }
            Change::Restarted { .. } => {}
            Change::Ended { id, at_ms, .. } => {
                self.pill
                    .update(|p, now| p.meeting(&id, MeetingPhase::Processing, at_ms, now));
                // The screen process stops at the timeline's stop; waiting for it takes up to
                // 15 s, so it happens off this thread.
                let screen = Arc::clone(&self.meetings.screen);
                let ended = id.clone();
                let _ = thread::Builder::new()
                    .name("screen-end".into())
                    .spawn(move || screen.end(&ended));
                // The last segments and the timeline closed.
                self.nudge_sync();
                self.meeting_changed();
            }
            Change::Finalized { id } => {
                // Cut off while Voice wasn't running: what was recorded still gets transcribed.
                // `ScreenSupervisor::recover`, right after the controller's, gives its screen
                // journal the `stop` it lacks.
                self.follow(&id);
                self.notify(
                    crate::meetings::controller::NOTIFY_MEETING,
                    "A meeting was cut off last time. Everything recorded until then is saved."
                        .to_string(),
                );
            }
            Change::Notify { title_key, body } => self.notify(title_key, body),
        }
    }

    /// Forwards a pause or resume to the screen process, off the event thread.
    fn screen_paused(&self, id: &str, paused: bool) {
        let screen = Arc::clone(&self.meetings.screen);
        let id = id.to_string();
        let _ = thread::Builder::new()
            .name("screen-pause".into())
            .spawn(move || screen.pause(&id, paused));
    }

    /// What the pill shows of the meeting's screen recording.
    fn pill_screen(&self, id: &str) {
        let (mode, switch) = self.meetings.screen.pill_state(id);
        let current = self.meetings.controller.current().map(|r| r.id);
        if current.as_deref() == Some(id) {
            self.pill
                .update(|p, _| p.meeting_screen(mode.as_str(), switch));
        }
    }

    /// What the screen supervisor reports: drops and give-ups become pill notices and tray
    /// notifications, a move shows the monitor's name.
    pub fn on_screen(self: &Arc<Self>, change: ScreenChange) {
        match change {
            ScreenChange::Mode { id } => {
                self.pill_screen(&id);
                self.settings_changed();
            }
            ScreenChange::Dropped { id, what, reason } => {
                self.screen_notice(&id, Some(drop_notice(what, reason)), Map::new(), DROP_HOLD);
                if let Some(body) = drop_text(what, reason) {
                    self.notify(NOTIFY_MEETING, body.to_string());
                }
                self.pill_screen(&id);
                self.settings_changed();
            }
            ScreenChange::NoPermission { id } => {
                // At the start of a meeting or in the middle of one: either way the screen is
                // gone and the audio goes on. The notice names the setting, because retrying
                // can't help.
                self.screen_notice(&id, Some(notices::SCREEN_PERMISSION), Map::new(), DROP_HOLD);
                self.notify_keyed(
                    NOTIFY_MEETING,
                    NOTIFY_SCREEN_PERMISSION,
                    SCREEN_PERMISSION_TEXT,
                );
                self.pill_screen(&id);
                self.settings_changed();
            }
            ScreenChange::GaveUp { id } => {
                self.screen_notice(&id, Some(notices::SCREEN_FAILED), Map::new(), DROP_HOLD);
                self.notify(
                    NOTIFY_MEETING,
                    "Voice's screen recorder stopped 5 times in a row, so the screen isn't recorded any more. The audio goes on.".to_string(),
                );
                self.pill_screen(&id);
                self.settings_changed();
            }
            ScreenChange::Moved { id, name } => {
                let mut args = Map::new();
                args.insert("name".into(), Value::String(name));
                self.screen_notice(&id, Some(notices::SCREEN_MOVED), args, MOVED_HOLD);
            }
            ScreenChange::Monitors { .. } => {
                // The probe answered or a monitor came or went: Settings shows it, a computer
                // that can't capture starts meetings with `off`, and the pill's screen button
                // shows when there is another monitor.
                let cfg = self.config();
                self.meetings
                    .controller
                    .set_screen(screen_mode(&cfg), self.meetings.screen.available());
                if let Some(id) = self.meetings.controller.current().map(|r| r.id) {
                    self.pill_screen(&id);
                }
                self.meetings.publish_status();
                self.settings_changed();
            }
        }
    }

    /// A pill notice about the meeting that records now.
    fn screen_notice(&self, id: &str, key: Option<&str>, args: Map<String, Value>, hold: Duration) {
        let Some(key) = key else {
            return;
        };
        if self.meetings.controller.current().map(|r| r.id).as_deref() != Some(id) {
            return;
        }
        self.pill
            .update(|p, now| p.meeting_notice(key, args, hold, now));
    }

    /// The pill's screen button opens a menu without changing the recording.
    pub fn screen_clicked(self: &Arc<Self>) {
        let core = Arc::clone(self);
        let _ = thread::Builder::new()
            .name("screen-menu".into())
            .spawn(move || {
                let Some(meeting) = core.meetings.controller.current() else {
                    return;
                };
                match core
                    .meetings
                    .screen
                    .describe(&meeting.id, Instant::now() + Duration::from_secs(4))
                {
                    Ok(mut info) => {
                        info["meeting"] = Value::String(meeting.id);
                        core.sink.screen_menu(info);
                    }
                    Err(e) => log::info!("screen: could not open the pill menu: {e}"),
                }
            });
    }

    /// Tells Settings and the pill when Rukoo connects, leaves or is refused.
    pub fn rukoo_changed(&self) {
        if self.note_rukoo() {
            self.settings_changed();
        }
    }

    /// Records Rukoo's connection as it is now and shows it on the pill; whether it changed. The
    /// start records it before its first Settings state, so the watch loop's first look doesn't
    /// send a state of its own for what that state already showed.
    pub(crate) fn note_rukoo(&self) -> bool {
        let Some(bridge) = self.meetings.bridge.as_ref() else {
            return false;
        };
        let info = bridge.rukoo();
        // `behind` moves with every event; Settings reads it with the next change.
        let key = serde_json::json!([info.state, info.client_version, info.protocol]);
        let changed = {
            let mut last = lock(&self.rukoo);
            let changed = *last != key;
            *last = key;
            changed
        };
        if changed {
            let connected = info.connected();
            self.pill.update(|p, _| p.set_rukoo(connected));
        }
        changed
    }

    /// Tells Settings when speaker labels start, stop or fail in a meeting, or find another voice.
    pub fn labels_changed(&self) {
        let key = Value::Array(
            self.meetings
                .labels()
                .into_iter()
                .map(|(id, s)| {
                    serde_json::json!([id, s.mode, s.state, s.reason, s.speakers, s.worker_pid])
                })
                .collect(),
        );
        let changed = {
            let mut last = lock(&self.labels);
            let changed = *last != key;
            *last = key;
            changed
        };
        if changed {
            self.settings_changed();
        }
    }

    /// Removes meeting audio, with the screen media, whose retention ran out, and clips an hour
    /// after their last use. Each meeting keeps the Settings > Meetings > Keep audio it started
    /// with; the current choice counts only for meetings from before Voice stored it.
    ///
    /// While bucket sync is on, media that hasn't reached the bucket stays, and with the switch on
    /// but sync's queue unreadable, all of it stays (spec 2.10). Below 10 GiB free that gives way:
    /// held media of meetings whose time is up goes, oldest first.
    pub fn sweep(&self) {
        let cfg = self.config();
        let switch_on = crate::sync::switch_on(&cfg);
        let sync = self.sync.as_deref();
        let swept = self.meetings.sweep(cfg.meetings_retention, |id| {
            crate::sync::holds(sync, switch_on, id)
        });
        // Clips are copies made on request, kept an hour after their last use.
        let clips = voice_screen_files::media::sweep_clips(
            self.meetings.library().root(),
            now_ms(),
            CLIP_LIFETIME_MS,
        );
        if !swept.removed.is_empty() {
            log::info!(
                "retention: removed the media of {}",
                swept.removed.join(", ")
            );
        }
        if !swept.held.is_empty() {
            log::info!(
                "retention: kept the media of {} for the bucket",
                swept.held.join(", ")
            );
        }
        let released = crate::sync::release_below_floor(self, &swept.held);
        crate::sync::note_held(self, !swept.held.is_empty());
        if !swept.removed.is_empty() || released || clips > 0 {
            self.settings.forget_storage();
            self.settings_changed();
        }
    }

    /// Bucket sync's status changed: Settings show it, and held media may go now.
    ///
    /// Called on sync's own threads. The state is built on another thread, so building it (which
    /// asks sync for its status) never waits on the caller.
    pub fn sync_changed(self: &Arc<Self>) {
        self.sync_rt.changed();
        if self.sync_rt.status_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let core = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("sync-status".into())
            .spawn(move || {
                core.sync_rt.status_pending.store(false, Ordering::SeqCst);
                core.settings_changed();
            });
        if spawned.is_err() {
            self.sync_rt.status_pending.store(false, Ordering::SeqCst);
        }
    }

    /// Lets bucket sync look at the meetings now instead of at its next scan.
    fn nudge_sync(&self) {
        if let Some(sync) = &self.sync {
            sync.nudge();
        }
    }

    /// Whether a meeting records the screen now (frames or video): the key dialogs refuse to
    /// open then, and an open one closes, because the screen recording could capture the code.
    pub fn screen_recording(&self) -> bool {
        self.meetings
            .controller
            .current()
            .is_some_and(|r| self.meetings.screen.mode_of(&r.id) != ScreenMode::Off)
    }

    fn meeting_changed(&self) {
        self.meetings.publish_status();
        self.sink.tray(self.tray());
        self.settings.forget_storage();
        self.settings_changed();
    }

    pub fn shutdown(&self) {
        if self.shutting_down.swap(true, Ordering::SeqCst) {
            return;
        }
        // First, so an upload stops between requests while the rest shuts down (at most 2 s).
        if let Some(sync) = &self.sync {
            sync.shutdown();
        }
        self.media.shutdown();
        self.dictation.shutdown();
        self.meetings.shutdown();
        self.pill.update(|p, _| p.clear());
        self.pill.stop();
        // A meeting window that ignored its shutdown can still hold the local worker while the
        // model loads (up to 180 s) or a window transcribes. Quitting waits for it only so long:
        // Windows ends the worker with the app, because it runs in a kill-on-close job.
        let speech = Arc::clone(&self.svc.speech);
        let (stopped, wait) = mpsc::channel();
        let stopping = thread::Builder::new()
            .name("speech-shutdown".into())
            .spawn(move || {
                speech.shutdown();
                let _ = stopped.send(());
            });
        match stopping {
            Ok(_) => {
                let _ = wait.recv_timeout(SPEECH_SHUTDOWN_WAIT);
            }
            Err(_) => self.svc.speech.shutdown(),
        }
        self.svc.input.wait_for_clipboard_restore();
    }
}

/// How long a clip `clip.get` cut stays after its last use.
const CLIP_LIFETIME_MS: u64 = 3_600_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Settings > Meetings > Screen as the protocol names it.
pub(crate) fn screen_mode(cfg: &Config) -> ScreenMode {
    match cfg.meetings_screen {
        core_config::ScreenMode::Off => ScreenMode::Off,
        core_config::ScreenMode::Frames => ScreenMode::Frames,
        core_config::ScreenMode::Video => ScreenMode::Video,
    }
}

/// The body of the notification when macOS doesn't let Voice record the screen
/// (voice/ui/i18n `app.notify.screenPermission`), and its English text for a catalog without it.
pub const NOTIFY_SCREEN_PERMISSION: &str = "app.notify.screenPermission";
const SCREEN_PERMISSION_TEXT: &str = "Screen Recording is off for Rukoo Voice, so Voice records no screen in this meeting. The audio goes on. Switch it on in System Settings > Privacy & Security > Screen & System Audio Recording, then start a new meeting.";

/// The pill notice for a part of the screen recording that stopped.
fn drop_notice(what: Part, reason: Reason) -> &'static str {
    match (what, reason) {
        (Part::Video, Reason::Disk) => notices::VIDEO_DROPPED_DISK,
        (Part::Video, Reason::Encoder) => notices::VIDEO_DROPPED_ENCODER,
        (Part::Video, Reason::Device) => notices::VIDEO_DROPPED_DEVICE,
        (Part::Frames, Reason::Disk) => notices::FRAMES_DROPPED_DISK,
        (Part::Frames, _) => notices::SCREEN_FAILED,
    }
}

/// The tray notification for a part of the screen recording that stopped.
fn drop_text(what: Part, reason: Reason) -> Option<&'static str> {
    Some(match (what, reason) {
        (Part::Video, Reason::Disk) => "Voice stopped the screen video to keep recording audio: the disk is almost full. Frames go on.",
        (Part::Video, Reason::Encoder) => "Voice records the screen as frames only: no video encoder on this computer worked.",
        // The frames go on with a device without video support. When no device came back at
        // all, the frames stop right after and their notification says so.
        (Part::Video, Reason::Device) => "Voice stopped the screen video: the graphics device didn't come back with video support.",
        (Part::Frames, Reason::Disk) => "Voice stopped recording the screen to keep recording audio: the disk is almost full.",
        (Part::Frames, Reason::Device) => "Voice stopped recording the screen: the graphics device couldn't be started again. The audio goes on.",
        (Part::Frames, Reason::Encoder) => return None,
    })
}

/// How long quitting waits for the local model to stop. Longer than its own 2 s quit grace, so a
/// worker that is merely idle always gets to quit cleanly.
const SPEECH_SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

/// How often the engine looks at meeting outcomes and at Rukoo's connection.
const WATCH_POLL: Duration = Duration::from_secs(1);
/// How often retention runs besides at start and after a meeting is done.
const SWEEP_EVERY: Duration = Duration::from_secs(10 * 60);
/// Held media is looked at again at most this often after bucket sync made progress.
const RESWEEP_AFTER: Duration = Duration::from_secs(5);
/// How often prepared media is swept and bucket sync's notifications are checked.
const MINUTE: Duration = Duration::from_secs(60);

/// How far behind the audio Mistral's realtime model may answer, in ms: its default.
pub(crate) const STREAM_DELAY_MS: u32 = 480;

/// Who labels a meeting's call speakers, and the realtime stream when the provider streams.
pub(crate) struct MeetingLabels {
    pub labels: Labels,
    /// The Settings switch, which only a meeting that chooses its label mode now reads.
    pub live_speakers: bool,
    /// `Some` only when the meeting provider streams: the stream, or why it can't be built.
    pub stream: Option<Result<StreamOptions, ProviderError>>,
}

/// The label choice for a meeting started now. The meeting provider alone decides whether
/// Mistral's stream transcribes and labels it; otherwise Nemotron labels on this computer when
/// the switch is on and it can run, and else the whole call is one speaker. Neither engine
/// ever stands in for the other. Nemotron's diarizer comes with the switch off too: a meeting
/// that started with labels keeps them when its pipeline starts again (Settings: "Applies to
/// meetings you start from now on"), and the pipeline gives a new meeting none.
pub(crate) fn meeting_labels(cfg: &Config, speech: &dyn Speech) -> MeetingLabels {
    let audio = cfg.meeting_audio();
    if stream::streams(&audio) {
        let hints = cfg.meeting_hints();
        let stream = speech.streaming(&audio).map(|speech| StreamOptions {
            speech,
            diarize: cfg.meetings_live_speakers,
            delay_ms: STREAM_DELAY_MS,
            language: hints.language,
            context_bias: hints.vocabulary,
        });
        return MeetingLabels {
            // The stream labels the call, not Nemotron. Without a stream, the pipeline waits
            // with its choice until there is one.
            labels: Labels::Off {
                reason: voice_meetings::STREAM_REASON.into(),
            },
            live_speakers: cfg.meetings_live_speakers,
            stream: Some(stream),
        };
    }
    let labels = match speech.diarizer() {
        Ok(diarizer) => Labels::Local { diarizer },
        Err(unavailable) => Labels::Off {
            reason: unavailable.code().into(),
        },
    };
    MeetingLabels {
        labels,
        live_speakers: cfg.meetings_live_speakers,
        stream: None,
    }
}

/// The engine's slow loop. Shows a meeting whose transcription failed and lets a finished one
/// go (`meeting.state` `failed` and `done` come from the pipeline long after the recorder
/// stopped), keeps Settings > Rukoo and Speaker labels current, runs the retention sweep, and lets
/// prepared media go.
pub(crate) fn watch(core: Weak<Core>, mut seq: u64) {
    let mut swept = Instant::now();
    let mut minute = Instant::now();
    loop {
        thread::sleep(WATCH_POLL);
        let Some(core) = core.upgrade() else {
            return;
        };
        if core.shutting_down() {
            return;
        }
        core.rukoo_changed();
        core.labels_changed();
        if swept.elapsed() >= SWEEP_EVERY
            || (swept.elapsed() >= RESWEEP_AFTER && core.sync_rt.take_resweep())
        {
            swept = Instant::now();
            core.sweep();
        }
        if minute.elapsed() >= MINUTE {
            minute = Instant::now();
            core.media.sweep();
            crate::sync::check_failing(&core);
        }
        // Borrowed from the engine each round, so the log closes as soon as the engine is gone.
        let Some(log) = core.meetings.log.as_ref() else {
            return;
        };
        for stored in log.after(seq).unwrap_or_default() {
            seq = stored.seq;
            if stored.event != voice_protocol::events::MEETING_STATE {
                continue;
            }
            let Some(id) = stored.meeting.as_deref() else {
                continue;
            };
            let at = stored.data["at"].as_u64().unwrap_or(0);
            // A meeting can record while an earlier one still transcribes: these outcomes are
            // the earlier one's, and the pill keeps showing the one that records.
            match stored.data["state"].as_str() {
                Some("failed") => {
                    core.pill
                        .update(|p, now| p.meeting(id, MeetingPhase::Failed, at, now));
                    let reason = stored.data["reason"].as_str().unwrap_or_default();
                    core.notify(
                        crate::meetings::controller::NOTIFY_MEETING,
                        format!("Voice couldn't finish a meeting's transcript: {reason}. The audio is kept."),
                    );
                }
                Some("done") => {
                    core.pill.update(|p, _| p.meeting_gone(id));
                    // The transcript and the timeline can go up now.
                    if let Some(sync) = &core.sync {
                        sync.nudge();
                    }
                    // "After transcription" retention removes the audio now.
                    core.sweep();
                }
                _ => {}
            }
        }
    }
}

pub(crate) fn looks(opts: &EngineOptions, svc: &Services, cfg: &Config) -> Looks {
    Looks {
        edge: match cfg.pill_edge {
            core_config::PillEdge::Bottom => crate::api::Edge::Bottom,
            core_config::PillEdge::Top => crate::api::Edge::Top,
        },
        theme: match cfg.ui_theme {
            core_config::Theme::System => svc.system.theme(),
            core_config::Theme::Light => Theme::Light,
            core_config::Theme::Dark => Theme::Dark,
        },
        locale: match cfg.ui_language {
            core_config::UiLanguage::System => svc.system.locale(),
            core_config::UiLanguage::En => Locale::En,
            core_config::UiLanguage::Nl => Locale::Nl,
        },
        dev: opts.dev_mode,
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn control_error(error: ControlError) -> ApiError {
    ApiError::new(error.code.as_str(), error.message)
}

/// The engine as the app holds it.
pub(crate) struct EngineImpl(pub Arc<Core>);

impl EngineApi for EngineImpl {
    fn pill_snapshot(&self) -> PillSnapshot {
        self.0.pill.read(|p| p.current())
    }

    fn pill_ack(&self, seq: u64) {
        self.0.pill.update(|p, _| p.ack(seq));
    }

    fn pill_stop_clicked(&self) {
        self.0.stop_clicked();
    }

    fn pill_screen_clicked(&self) {
        self.0.screen_clicked();
    }

    fn pill_screen_set(
        &self,
        meeting: &str,
        mode: &str,
        monitor: Option<&str>,
    ) -> Result<(), ApiError> {
        let mode = match mode {
            "off" => ScreenMode::Off,
            "frames" => ScreenMode::Frames,
            "video" => ScreenMode::Video,
            _ => return Err(ApiError::new("bad_args", "Choose off, frames or video.")),
        };
        self.0
            .meetings
            .screen
            .set_mode(
                meeting,
                mode,
                monitor,
                Instant::now() + Duration::from_secs(15),
            )
            .map(|_| ())
            .map_err(|e| {
                self.0.screen_notice(
                    meeting,
                    Some(notices::SCREEN_CHANGE_FAILED),
                    Map::new(),
                    DROP_HOLD,
                );
                self.0.sink.notify(Notification {
                    title_key: NOTIFY_MEETING.into(),
                    body: e.message.clone(),
                    body_key: None,
                });
                ApiError::new(e.code.as_str(), e.message)
            })
    }

    fn settings_state(&self) -> Value {
        crate::settings::state(&self.0)
    }

    fn settings_set(&self, key: &str, value: Value) -> Result<(), ApiError> {
        crate::settings::set(&self.0, key, value)
    }

    fn settings_save_provider(&self, save: Value) -> Result<(), ApiError> {
        crate::settings::save_provider(&self.0, save)
    }

    fn settings_remove_key(&self, target: &str) -> Result<(), ApiError> {
        crate::settings::remove_key(&self.0, target)
    }

    fn settings_list_models(&self, request: Value) -> Result<Vec<String>, ApiError> {
        crate::settings::list_models(&self.0, request)
    }

    fn settings_model(&self, model: &str, action: &str) -> Result<(), ApiError> {
        crate::settings::model(&self.0, model, action)
    }

    fn settings_set_folder(&self, purpose: &str, path: PathBuf) -> Result<(), ApiError> {
        crate::settings::set_folder(&self.0, purpose, path)
    }

    fn settings_clean_preview(&self, preview: Value) -> Result<String, ApiError> {
        crate::settings::clean_preview(&self.0, preview)
    }

    fn settings_check_updates(&self) -> Value {
        crate::settings::check_updates(&self.0)
    }

    fn settings_new_token(&self) -> Result<(), ApiError> {
        crate::settings::new_token(&self.0)
    }

    fn settings_import_ownkey(&self) -> Result<Value, ApiError> {
        crate::settings::import_ownkey(&self.0)
    }

    fn settings_sync_test(&self) -> Result<Value, ApiError> {
        crate::settings::sync_test(&self.0)
    }

    fn settings_sync_save_secret(&self, secret: String) -> Result<(), ApiError> {
        crate::settings::sync_save_secret(&self.0, secret)
    }

    fn settings_sync_key(&self, action: &str, code: Option<String>) -> Result<Value, ApiError> {
        crate::settings::sync_key(&self.0, action, code)
    }

    fn settings_sync_key_copy(&self) -> Result<(), ApiError> {
        crate::settings::sync_key_copy(&self.0)
    }

    fn settings_sync_restore(&self, action: &str) -> Result<Value, ApiError> {
        crate::settings::sync_restore(&self.0, action)
    }

    fn settings_start_page(&self) -> Option<String> {
        self.0.settings.take_start_page()
    }

    fn folder_path(&self, folder: Folder) -> Option<PathBuf> {
        crate::settings::folder_path(&self.0, folder)
    }

    fn start_meeting(&self) -> Result<String, ApiError> {
        self.0
            .meetings
            .controller
            .start(StartArgs::default(), StartedBy::Voice)
            .map_err(control_error)
    }

    fn stop_meeting(&self, reason: StopReason) -> Result<(), ApiError> {
        let reason = match reason {
            StopReason::User => "user",
            StopReason::Rukoo => "rukoo",
        };
        self.0
            .meetings
            .controller
            .stop(None, reason)
            .map_err(control_error)
    }

    fn pause_meeting(&self) -> Result<(), ApiError> {
        self.0
            .meetings
            .controller
            .pause(None)
            .map_err(control_error)
    }

    fn resume_meeting(&self) -> Result<(), ApiError> {
        self.0
            .meetings
            .controller
            .resume(None)
            .map_err(control_error)
    }

    fn ui(&self) -> UiOptions {
        let cfg = self.0.config();
        let looks = self.0.looks();
        UiOptions {
            theme: looks.theme,
            locale: looks.locale,
            pill_edge: looks.edge,
            hide_pill_from_capture: cfg.pill_hide_from_capture,
            dev: self.0.opts.dev_mode,
        }
    }

    fn tray_state(&self) -> TrayState {
        self.0.tray()
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }

    fn meeting_control(&self) -> Option<Arc<dyn voice_bridge::MeetingControl>> {
        self.0
            .meetings
            .control()
            .map(|control| control as Arc<dyn voice_bridge::MeetingControl>)
    }
}

impl Drop for EngineImpl {
    fn drop(&mut self) {
        // Threads hold only weak references; this ends them if the app forgot to shut down.
        self.0.shutdown();
    }
}

/// The recognizer for a meeting provider that can't be built (a missing key or model): every
/// window fails with the reason, which `meeting.state` `failed` carries if it never changes.
struct Unavailable(ProviderError);

impl SpeechToText for Unavailable {
    fn transcribe(&self, _pcm: &[i16], _hints: &Hints) -> Result<Recognized, ProviderError> {
        Err(self.0.clone())
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;

    /// Every drop the screen process can journal reaches the pill, and every one but a frames
    /// drop for the encoder, which never happens, the tray.
    #[test]
    fn every_drop_has_a_notice_and_a_notification() {
        assert_eq!(
            drop_notice(Part::Video, Reason::Disk),
            "pill.videoDropped.disk"
        );
        assert_eq!(
            drop_notice(Part::Video, Reason::Encoder),
            "pill.videoDropped.encoder"
        );
        assert_eq!(
            drop_notice(Part::Video, Reason::Device),
            "pill.videoDropped.device"
        );
        assert_eq!(
            drop_notice(Part::Frames, Reason::Disk),
            "pill.framesDropped.disk"
        );
        assert_eq!(
            drop_notice(Part::Frames, Reason::Device),
            "pill.screenFailed"
        );
        assert!(drop_text(Part::Video, Reason::Encoder)
            .unwrap()
            .contains("frames only"));
        assert!(drop_text(Part::Video, Reason::Device)
            .unwrap()
            .contains("video support"));
        assert!(drop_text(Part::Video, Reason::Disk)
            .unwrap()
            .contains("Frames go on"));
        assert!(drop_text(Part::Frames, Reason::Device).is_some());
        assert!(drop_text(Part::Frames, Reason::Disk).is_some());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use voice_core::local::{
        LocalStatus, Located, ModelError, ModelErrorKind, ModelSpec, Progress,
    };
    use voice_core::providers::stream::{SpeechStream, StreamSession, StreamingSpeech};
    use voice_core::providers::{Activity, ProviderConfig};
    use voice_core::speakers::{
        DiarError, DiarErrorKind, DiarStream, Diarizer, RuntimeInfo, Unavailable as NotAvailable,
    };
    use voice_core::Rewriter;

    use super::*;

    struct NoDiarizer;

    impl Diarizer for NoDiarizer {
        fn open(&self) -> Result<Box<dyn DiarStream>, DiarError> {
            Err(DiarError::new(DiarErrorKind::Runtime, "not in this test"))
        }
    }

    struct NoStream;

    impl StreamingSpeech for NoStream {
        fn connect(&self, _: &StreamSession) -> Result<Box<dyn SpeechStream>, ProviderError> {
            Err(ProviderError::provider("not in this test"))
        }
    }

    /// Speech that only answers the label questions, and counts them.
    #[derive(Default)]
    struct Labelling {
        refuse: Option<NotAvailable>,
        diarizers: AtomicUsize,
        streams: AtomicUsize,
    }

    impl Speech for Labelling {
        fn recognizer(&self, _: &ProviderConfig) -> Result<Arc<dyn SpeechToText>, ProviderError> {
            Err(ProviderError::config("not in this test"))
        }
        fn rewriter(&self, _: &ProviderConfig) -> Result<Arc<dyn Rewriter>, ProviderError> {
            Err(ProviderError::config("not in this test"))
        }
        fn list_models(
            &self,
            _: Activity,
            _: &ProviderConfig,
        ) -> Result<Vec<String>, ProviderError> {
            Ok(Vec::new())
        }
        fn local_preload(&self) {}
        fn local_status(&self) -> LocalStatus {
            LocalStatus::default()
        }
        fn local_load(&self, _: ModelRegistry) -> Result<(), ProviderError> {
            Ok(())
        }
        fn local_configure(&self, _: ModelRegistry, _: Duration) {}
        fn local_unload(&self) {}
        fn download(
            &self,
            _: &ModelRegistry,
            _: &ModelSpec,
            _: &mut dyn FnMut(Progress),
            _: &AtomicBool,
        ) -> Result<Located, ModelError> {
            Err(ModelError::new(
                ModelErrorKind::Cancelled,
                "not in this test",
            ))
        }
        fn shutdown(&self) {}
        fn diarizer(&self) -> Result<Arc<dyn Diarizer>, NotAvailable> {
            self.diarizers.fetch_add(1, Ordering::SeqCst);
            match &self.refuse {
                Some(why) => Err(why.clone()),
                None => Ok(Arc::new(NoDiarizer)),
            }
        }
        fn speaker_availability(&self) -> Result<RuntimeInfo, NotAvailable> {
            Err(NotAvailable::Runtime)
        }
        fn streaming(&self, _: &ProviderConfig) -> Result<Arc<dyn StreamingSpeech>, ProviderError> {
            self.streams.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(NoStream))
        }
    }

    fn off_reason(labels: &Labels) -> Option<&str> {
        match labels {
            Labels::Off { reason } => Some(reason),
            Labels::Local { .. } => None,
        }
    }

    fn realtime(cfg: &mut Config) {
        cfg.switch_meeting_provider("mistral");
        cfg.meetings_audio_api_key = "sk-test".into();
        cfg.meetings_audio_model = "voxtral-mini-transcribe-realtime-3".into();
    }

    #[test]
    fn nemotron_labels_when_the_switch_is_on_and_it_can_run() {
        let speech = Labelling::default();
        let cfg = Config::defaults(false);
        assert!(cfg.meetings_live_speakers, "on by default");
        let chosen = meeting_labels(&cfg, &speech);
        assert!(matches!(chosen.labels, Labels::Local { .. }));
        assert!(chosen.stream.is_none());
        assert_eq!(speech.streams.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_switch_off_keeps_the_diarizer_for_meetings_that_label() {
        let speech = Labelling::default();
        let mut cfg = Config::defaults(false);
        assert!(meeting_labels(&cfg, &speech).live_speakers);
        cfg.meetings_live_speakers = false;
        let chosen = meeting_labels(&cfg, &speech);
        // A meeting that started with labels gets its diarizer again after a restart; the
        // pipeline gives a new meeting none, by the switch.
        assert!(matches!(chosen.labels, Labels::Local { .. }));
        assert!(!chosen.live_speakers);
        assert!(chosen.stream.is_none());
        assert_eq!(speech.streams.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn nemotron_that_can_t_run_leaves_the_call_one_speaker_and_never_streams() {
        let speech = Labelling {
            refuse: Some(NotAvailable::Cpu {
                missing: vec!["bmi2".into()],
            }),
            ..Labelling::default()
        };
        // A Mistral key for dictation changes nothing: only the meeting provider streams.
        let mut cfg = Config::defaults(false);
        cfg.switch_audio_provider("mistral");
        cfg.audio_api_key = "sk-test".into();
        let chosen = meeting_labels(&cfg, &speech);
        assert_eq!(off_reason(&chosen.labels), Some("cpu"));
        assert!(chosen.stream.is_none());
        assert_eq!(speech.streams.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_realtime_meeting_provider_streams_and_labels_without_nemotron() {
        let speech = Labelling::default();
        let mut cfg = Config::defaults(false);
        realtime(&mut cfg);
        cfg.language = "nl".into();
        cfg.vocabulary = vec!["Rukoo".into()];
        let chosen = meeting_labels(&cfg, &speech);
        assert_eq!(off_reason(&chosen.labels), Some("stream"));
        let stream = chosen
            .stream
            .expect("streams")
            .expect("the stream is built");
        assert!(stream.diarize);
        assert_eq!(stream.delay_ms, 480);
        assert_eq!(stream.language.as_deref(), Some("nl"));
        assert_eq!(stream.context_bias, ["Rukoo"]);
        assert_eq!(speech.diarizers.load(Ordering::SeqCst), 0);
        // With the switch off the stream still transcribes, without labels.
        cfg.meetings_live_speakers = false;
        let stream = meeting_labels(&cfg, &speech).stream.unwrap().unwrap();
        assert!(!stream.diarize);
        assert_eq!(speech.diarizers.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_batch_mistral_model_is_labelled_by_nemotron() {
        let speech = Labelling::default();
        let mut cfg = Config::defaults(false);
        realtime(&mut cfg);
        cfg.meetings_audio_model = "voxtral-mini-latest".into();
        let chosen = meeting_labels(&cfg, &speech);
        assert!(matches!(chosen.labels, Labels::Local { .. }));
        assert!(chosen.stream.is_none());
        assert_eq!(speech.streams.load(Ordering::SeqCst), 0);
    }
}
