//! `rukoo-voice-screen record`: one process per meeting, next to the recorder.
//!
//! This module starts the session: it refuses a live `screen.json`, a journal with `stop` or one
//! without `--resume`, opens the journal, repairs what a dead session left open, waits for the
//! recorder's `session` line, binds the control port, chooses the monitor and hands the running
//! session to `screen::Screen`. The session's own loop, journal and drop rules are there.
//!
//! Startup order (spec 4.2): refuse a live `screen.json` (4), a journal with `stop` (3) or a
//! journal without `--resume` (3); open the journal; on `--resume` repair the open video file,
//! write its `video_end crash` and journal orphan JPEGs; wait up to 5 s for a `session` line in
//! the timeline (else exit 1); bind the control port; write `session` (and `crash`); write
//! `screen.json`; choose the monitor and write `monitor`; start capturing, writing `capture` on
//! the first frame.

use std::fs;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;
use voice_screen_files::clock::ScreenClock;
use voice_screen_files::control::{self, Request};
use voice_screen_files::frames::Saver;
use voice_screen_files::gate::VideoGate;
use voice_screen_files::journal::{self, History, JournalWriter, Line, OpenError, FORMAT_VERSION};
use voice_screen_files::monitors::{self, MonitorInfo};
use voice_screen_files::paths;
use voice_screen_files::policy::{DropState, Part};
use voice_screen_files::screenfile::{self, ScreenFile};
use voice_screen_files::timeline::{TimelineFollower, TimelineLine};

use crate::args::{Mode, RecordOptions, TestSpec};
use crate::finalize;
use crate::frames::worker::FrameWorker;
use crate::platform::Platform;
use crate::{log, ExitError, VERSION};

mod screen;

pub use self::screen::{CaptureStop, Ev};
use self::screen::{Screen, EVENT_QUEUE, HEARTBEAT, MEMORY_LOG};

/// What the timeline said before the screen process started.
#[derive(Default)]
struct TimelineState {
    sessions: u64,
    paused: bool,
    stop: Option<u64>,
}

impl TimelineState {
    fn read(&mut self, line: &TimelineLine) {
        match line {
            TimelineLine::Session { .. } => self.sessions += 1,
            TimelineLine::Pause { .. } => self.paused = true,
            TimelineLine::Resume { .. } => self.paused = false,
            TimelineLine::Stop { t, .. } => self.stop = Some(*t),
            _ => {}
        }
    }
}

/// How long to wait for the recorder's `session` line at startup.
const SESSION_WAIT: Duration = Duration::from_secs(5);
/// No new timeline line and no status request for this long, awake: the recorder is gone.
const ORPHAN: Duration = Duration::from_secs(60);

/// Records one monitor into the meeting, on `platform`'s graphics device and encoders.
pub fn record<P: Platform>(platform: P, opts: RecordOptions) -> Result<(), ExitError> {
    platform.init();
    let dir = opts.meeting.clone();
    if !dir.is_dir() {
        return Err(ExitError::usage(format!(
            "meeting folder {} doesn't exist",
            dir.display()
        )));
    }
    if screenfile::live(&dir).is_some() {
        return Err(ExitError::busy());
    }
    let lines = journal::read_all(&dir)
        .map_err(|e| ExitError::failed(format!("can't read the screen journal: {e}")))?;
    let history = History::from_lines(&lines);
    if history.stopped.is_some() {
        return Err(ExitError::conflict(
            "the meeting's screen recording has stopped; nothing can follow its stop line",
        ));
    }
    if (!lines.is_empty() || finalize::has_media(&dir)) && !opts.resume {
        return Err(ExitError::conflict(
            "the meeting already has a screen recording; continue it with --resume",
        ));
    }
    let monitor_list = platform.displays().list(opts.test.as_ref());
    let pinned = match &opts.monitor {
        Some(id) => Some(
            monitor_list
                .iter()
                .find(|m| &m.id == id)
                .cloned()
                .ok_or_else(|| ExitError::failed(format!("monitor {id} isn't there")))?,
        ),
        None => None,
    };
    if monitor_list.is_empty() {
        return Err(ExitError::failed("there is no monitor to record"));
    }
    if opts.test.is_none() {
        // The grant comes first: a Mac that can capture but isn't allowed to is told apart, so
        // Voice names the setting instead of retrying a start that can't work.
        if let Some(why) = platform.displays().missing_permission() {
            return Err(ExitError::no_permission(why));
        }
        if !platform.displays().can_capture() {
            return Err(ExitError::failed("this computer can't capture the screen"));
        }
    }
    let frames_dir = paths::screen_dir(&dir).join("frames");
    fs::create_dir_all(&frames_dir)
        .map_err(|e| ExitError::failed(format!("can't create the screen folder: {e}")))?;
    let mut journal = match JournalWriter::open(&dir) {
        Ok(journal) => journal,
        Err(OpenError::Busy) => return Err(ExitError::busy()),
        Err(OpenError::Io(e)) => {
            return Err(ExitError::failed(format!(
                "can't open the screen journal: {e}"
            )))
        }
    };

    // A resumed session first closes what the dead one left open.
    let resumed = history.sessions > 0;
    let mut last_seen = history.last_seen;
    if resumed {
        let (_, video_end) = finalize::close_open_video(&dir, &mut journal, &lines, &history);
        last_seen = last_seen.max(video_end);
        let reader = platform.reader();
        last_seen = last_seen.max(finalize::journal_orphans(
            &dir,
            &mut journal,
            &history,
            reader.as_ref(),
        ));
    }
    finalize::remove_temp_files(&dir);

    // The recorder's clock: its `session` and `heartbeat` lines anchor ours.
    let mut follower = TimelineFollower::new(&dir);
    let mut clock = ScreenClock::new();
    let mut timeline = TimelineState::default();
    let deadline = Instant::now() + SESSION_WAIT;
    loop {
        for line in follower.poll().unwrap_or_default() {
            timeline.read(&line);
            clock.feed(&line, platform.clocks().now());
        }
        if timeline.sessions > 0 && clock.at(platform.clocks().now()).is_some() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(ExitError::failed(
                "the recorder's timeline has no session line to take the meeting clock from",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let listener = control::bind()
        .map_err(|e| ExitError::failed(format!("can't open the control port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ExitError::failed(e.to_string()))?
        .port();
    let token = control::new_token().map_err(|e| ExitError::failed(e.to_string()))?;
    let now = platform.clocks().now();
    let t0 = clock.at(now).unwrap_or(0);
    let session = history.sessions + 1;
    let line = Line::Session {
        t: t0,
        wall: now.wall,
        boot: now.boot,
        session,
        pid: std::process::id(),
        resumed,
        version: FORMAT_VERSION,
        mode: opts.mode.as_str().into(),
        anchor: if clock.uses_boot() { "boot" } else { "wall" }.into(),
        screen: VERSION.into(),
    };
    journal
        .write(&line)
        .map_err(|e| ExitError::failed(format!("can't write the screen journal: {e}")))?;
    if !resumed
        && opts.mode != Mode::Off
        && fs::read(dir.join("meeting.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|meeting| meeting["screen"] == "off")
    {
        journal
            .write(&Line::Mode {
                t: t0,
                id: format!("screen-mode-{session}-0"),
                mode: opts.mode.as_str().into(),
                reason: "user".into(),
            })
            .map_err(|e| ExitError::failed(format!("can't write the screen mode: {e}")))?;
    }
    if resumed {
        let _ = journal.write(&Line::Crash {
            t: t0,
            last_seen: last_seen.min(t0),
        });
    }
    journal.sync();
    let drops = DropState::from_history(&history, opts.mode == Mode::Video);

    let (events, rx) = mpsc::sync_channel::<Ev<P::Picture>>(EVENT_QUEUE);
    let (requests, request_rx) = mpsc::sync_channel::<Request>(16);
    control::serve(listener, token.clone(), requests)
        .map_err(|e| ExitError::failed(format!("can't start the control port: {e}")))?;
    let forward = events.clone();
    std::thread::Builder::new()
        .name("control-forward".into())
        .spawn(move || {
            for request in request_rx {
                if forward.send(Ev::Control(request)).is_err() {
                    return;
                }
            }
        })
        .map_err(|e| ExitError::failed(format!("can't start the control thread: {e}")))?;
    screenfile::write(
        &dir,
        &ScreenFile {
            pid: std::process::id(),
            port,
            token,
            session,
            started_at: now.wall,
        },
    )
    .map_err(|e| ExitError::failed(format!("can't write screen.json: {e}")))?;

    let previous_monitor = lines.iter().rev().find_map(|line| match line {
        Line::Monitor { id, .. } => monitor_list.iter().find(|m| m.id == *id).cloned(),
        _ => None,
    });
    let monitor = pinned.or(previous_monitor).unwrap_or_else(|| {
        choose_monitor(
            &platform,
            opts.test.as_ref(),
            &monitor_list,
            &opts.exclude_pids,
        )
    });
    let video_seq = history
        .last_video_seq
        .max(finalize::highest_video_seq(&dir));
    let write_fail_frames = match opts.test.as_ref().and_then(|t| t.write_fail) {
        Some((Part::Frames, at)) => Some(at),
        _ => None,
    };
    let worker = FrameWorker::start(
        dir.clone(),
        events.clone(),
        write_fail_frames,
        platform.frame_io(),
    );
    let awake = platform.clocks().awake_100ns();
    let orphan = orphan_after();
    let mut screen = Screen {
        platform,
        dir,
        mode: opts.mode,
        device_fault_at: opts.test.as_ref().and_then(|t| t.device_lost_at_ms),
        test: opts.test,
        journal,
        clock,
        follower,
        drops,
        events,
        device: None,
        source: None,
        thumbs: None,
        saver: Saver::new(),
        worker,
        generation: 0,
        capturing: false,
        awaiting_capture: false,
        captured: false,
        monitor_cache: (Instant::now(), monitor_list),
        monitor,
        pinned: opts.monitor,
        exclude_pids: opts.exclude_pids,
        video: None,
        gate: VideoGate::new(),
        video_seq,
        video_bytes_closed: 0,
        encoder: None,
        rebuild_at: None,
        capture_retry_at: None,
        no_monitor: false,
        paused: timeline.paused,
        timeline_paused: timeline.paused,
        pause_from: None,
        locked: false,
        frames_saved: 0,
        last_frame_id: None,
        last: None,
        last_thumb_at: None,
        create_failures: 0,
        no_video_devices: 0,
        had_video_device: false,
        recovering: false,
        stop: None,
        stop_t: None,
        last_activity: awake,
        last_status: awake,
        orphan_100ns: orphan.as_millis() as u64 * 10_000,
        next_heartbeat: Instant::now() + HEARTBEAT,
        next_disk_check: Instant::now(),
        next_lock_check: Instant::now(),
        next_memory_log: Instant::now() + MEMORY_LOG,
        session,
        mode_changes: 0,
    };
    log(format_args!(
        "session {session}: recording {} of {} ({}x{}){}",
        screen.mode.as_str(),
        screen.monitor.id,
        screen.monitor.width,
        screen.monitor.height,
        if resumed { ", resumed" } else { "" }
    ));
    screen.write_monitor(if resumed { "resume" } else { "start" });
    // At capture start in video mode, a disk already under the floor drops the video at once.
    screen.check_disk();
    if let Some(t) = timeline.stop {
        screen.request_stop(t, "meeting", None);
    }
    screen.start_capture();
    screen.run(rx);
    Ok(())
}

/// The 60 s orphan rule; a development copy's tests shorten it with
/// `VOICE_SCREEN_TEST_ORPHAN_MS`.
fn orphan_after() -> Duration {
    if crate::dev_mode() {
        if let Some(ms) = std::env::var("VOICE_SCREEN_TEST_ORPHAN_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            return Duration::from_millis(ms.max(500));
        }
    }
    ORPHAN
}

/// The monitor that shows the call (spec 2.7): the topmost real window's, else the cursor's,
/// else the primary.
fn choose_monitor<P: Platform>(
    platform: &P,
    test: Option<&TestSpec>,
    list: &[MonitorInfo],
    exclude_pids: &[u32],
) -> MonitorInfo {
    let id = match test {
        Some(_) => monitors::choose(&[], None, list),
        None => monitors::choose(
            &platform.displays().z_order(exclude_pids),
            platform.displays().cursor_monitor().as_deref(),
            list,
        ),
    };
    id.and_then(|id| list.iter().find(|m| m.id == id).cloned())
        .or_else(|| list.iter().find(|m| m.primary).cloned())
        .unwrap_or_else(|| list[0].clone())
}
