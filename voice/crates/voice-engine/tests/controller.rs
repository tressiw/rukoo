//! The meeting controller against the real recorder, with synthetic sources.
//!
//! This test binary has its own main (`harness = false`) so it can also be the recorder: run
//! with `VOICE_ENGINE_TEST_ROLE=recorder` it calls `voice_recorder::run`, the same function the
//! `rukoo-voice-recorder` binary's main calls. Set `VOICE_ENGINE_TEST_RECORDER` to the path of a
//! built `rukoo-voice-recorder` to run the tests against that binary instead.

mod support;

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{lock, wait_for, Harness, TempDir};
use voice_bridge::{EventLog, MeetingControl};
use voice_engine::media::{MediaService, NoScreenMedia};
use voice_engine::meetings::bridge::Control;
use voice_engine::meetings::controller::{Change, Controller, ControllerOptions, ControllerTiming};
use voice_engine::meetings::screen::{ScreenOptions, ScreenSupervisor};
use voice_engine::{PillPhase, StopReason};
use voice_input::Key;
use voice_protocol::{ErrorCode, ScreenMode, StartArgs, StartedBy, Track};
use voice_recorder::meeting::read_recorder_file;
use voice_recorder::timeline::{self, Line};

const ROLE: &str = "VOICE_ENGINE_TEST_ROLE";
const REAL_RECORDER: &str = "VOICE_ENGINE_TEST_RECORDER";

fn main() {
    if std::env::var(ROLE).as_deref() == Ok("recorder") {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Err(e) = voice_recorder::run(&args) {
            eprintln!("rukoo-voice-recorder: {e}");
            std::process::exit(e.code);
        }
        return;
    }
    // Single-threaded here, so setting the variable is safe; every recorder the controller
    // starts from now on inherits it.
    std::env::set_var(ROLE, "recorder");
    #[allow(unused_mut)]
    let mut tests: Vec<(&str, fn())> = vec![
        ("commands_reach_the_recorder", commands_reach_the_recorder),
        (
            "the_watchdog_resumes_a_killed_recorder",
            the_watchdog_resumes_a_killed_recorder,
        ),
        (
            "the_watchdog_gives_up_after_its_restarts",
            the_watchdog_gives_up_after_its_restarts,
        ),
        (
            "a_new_controller_reattaches_to_a_live_recorder",
            a_new_controller_reattaches_to_a_live_recorder,
        ),
        (
            "a_cut_off_meeting_is_finished_at_start",
            a_cut_off_meeting_is_finished_at_start,
        ),
        (
            "rukoo_commands_go_through_meeting_control",
            rukoo_commands_go_through_meeting_control,
        ),
        ("a_meeting_through_the_engine", a_meeting_through_the_engine),
        (
            "a_restarted_engine_reattaches_and_stops_the_meeting",
            a_restarted_engine_reattaches_and_stops_the_meeting,
        ),
        (
            "a_cut_off_meeting_is_transcribed_at_the_next_start",
            a_cut_off_meeting_is_transcribed_at_the_next_start,
        ),
        (
            "transcription_waits_for_consent_and_starts_when_it_is_given",
            transcription_waits_for_consent_and_starts_when_it_is_given,
        ),
        (
            "a_reattach_ends_a_dictation_the_old_app_left_open",
            a_reattach_ends_a_dictation_the_old_app_left_open,
        ),
        (
            "a_meeting_records_the_settings_default_screen",
            a_meeting_records_the_settings_default_screen,
        ),
        ("rukoo_overrides_the_screen", rukoo_overrides_the_screen),
        ("no_screen_process_means_off", no_screen_process_means_off),
    ];
    // Only Windows refuses a second writer of the timeline, which this test plays.
    #[cfg(windows)]
    tests.push((
        "a_recorder_that_holds_the_meeting_without_answering_keeps_it_live",
        a_recorder_that_holds_the_meeting_without_answering_keeps_it_live,
    ));
    let filters: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .collect();
    let mut failed = Vec::new();
    let mut ran = 0;
    for (name, test) in tests {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        ran += 1;
        let started = Instant::now();
        match panic::catch_unwind(AssertUnwindSafe(test)) {
            Ok(()) => println!("test {name} ... ok ({} ms)", started.elapsed().as_millis()),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed.push(name);
            }
        }
    }
    println!(
        "test result: {}. {} passed; {} failed",
        if failed.is_empty() { "ok" } else { "FAILED" },
        ran - failed.len(),
        failed.len()
    );
    if !failed.is_empty() {
        std::process::exit(101);
    }
}

fn recorder_exe() -> PathBuf {
    std::env::var_os(REAL_RECORDER)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().unwrap())
}

fn timing() -> ControllerTiming {
    ControllerTiming {
        poll: Duration::from_millis(100),
        start_timeout: Duration::from_secs(20),
        backoff: vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ],
        stable_after: Duration::from_secs(60),
        ping_failures: 3,
        recorder_args: vec!["--segment-ms".into(), "1000".into()],
    }
}

fn sources() -> Vec<(String, String)> {
    vec![
        ("mic".into(), "synthetic:tone:440".into()),
        ("call".into(), "synthetic:idle".into()),
    ]
}

type Changes = Arc<Mutex<Vec<Change>>>;

fn controller(data: &Path, timing: ControllerTiming) -> (Arc<Controller>, Changes) {
    let changes: Changes = Arc::default();
    let controller = Controller::new(
        ControllerOptions {
            data_dir: data.to_path_buf(),
            recorder_exe: recorder_exe(),
            sources: sources(),
            timing,
        },
        {
            let changes = Arc::clone(&changes);
            move |change| lock(&changes).push(change)
        },
    )
    .unwrap();
    (controller, changes)
}

fn wait_change(changes: &Changes, what: &str, mut matches: impl FnMut(&Change) -> bool) -> Change {
    let mut found = None;
    wait_for(what, || {
        found = lock(changes).iter().find(|c| matches(c)).cloned();
        found.is_some()
    });
    found.unwrap()
}

fn count(changes: &Changes, matches: impl Fn(&Change) -> bool) -> usize {
    lock(changes).iter().filter(|c| matches(c)).count()
}

fn meeting_dir(data: &Path, id: &str) -> PathBuf {
    data.join("meetings").join(id)
}

fn lines(data: &Path, id: &str) -> Vec<Line> {
    timeline::read(&meeting_dir(data, id).join("timeline.jsonl")).unwrap()
}

fn wait_clock(controller: &Controller, at_least: u64) {
    wait_for(&format!("meeting time {at_least} ms"), || {
        controller.current().is_some_and(|r| r.at_ms >= at_least)
    });
}

/// Ends a recorder this test started, by its process id: `taskkill` on Windows, `kill -9` on every
/// Unix, macOS included.
fn kill(pid: u32) {
    #[cfg(windows)]
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    #[cfg(not(windows))]
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success(), "kill {pid}");
}

fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }
    #[cfg(not(windows))]
    {
        // Linux could read /proc/<pid>, but macOS has none, so every Unix asks `ps` for the process state.
        // It prints nothing for a process that is gone and "Z" for one that exited but hasn't been reaped,
        // which counts as gone here: a controller that reattached never reaps a recorder the last one
        // started, so that recorder waits to be reaped until this test process ends.
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output();
        out.is_ok_and(|out| {
            let stat = String::from_utf8_lossy(&out.stdout);
            let stat = stat.trim();
            !stat.is_empty() && !stat.starts_with('Z')
        })
    }
}

fn recorder_pid(data: &Path, id: &str) -> u32 {
    read_recorder_file(&meeting_dir(data, id))
        .expect("recorder.json")
        .pid
}

// The tests ----------------------------------------------------------------------------------

fn commands_reach_the_recorder() {
    let dir = TempDir::new("commands");
    let data = dir.join("data");
    let (c, changes) = controller(&data, timing());
    let id = c
        .start(
            StartArgs {
                title: Some("Weekly with Anna".into()),
                ..StartArgs::default()
            },
            StartedBy::Voice,
        )
        .unwrap();
    assert!(
        matches!(&lock(&changes)[0], Change::Started { id: started, at_ms: 0 } if *started == id)
    );
    let meeting: Value = serde_json::from_slice(
        &std::fs::read(meeting_dir(&data, &id).join("meeting.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meeting["title"], "Weekly with Anna");
    assert_eq!(meeting["startedBy"], "voice");
    assert_eq!(meeting["tracks"], json!(["mic", "call"]));
    assert_eq!(meeting["sources"]["mic"], "synthetic:tone:440");
    assert_eq!(meeting["screen"], "off");
    let busy = c.start(StartArgs::default(), StartedBy::Rukoo).unwrap_err();
    assert_eq!(
        (busy.code, busy.meeting.as_deref()),
        (ErrorCode::Busy, Some(id.as_str()))
    );

    wait_clock(&c, 1000);
    wait_change(&changes, "a clock tick", |c| {
        matches!(c, Change::Clock { mic_ok: true, .. })
    });
    c.pause(None).unwrap();
    assert_eq!(c.pause(None).unwrap_err().code, ErrorCode::NotRecording);
    assert!(c.current().unwrap().paused);
    c.resume(Some(&id)).unwrap();
    let (mark, at) = c.mark(Some(&id), Some("Decision")).unwrap();
    assert_eq!(mark, "k-1");
    assert!(at >= 1000);
    assert_eq!(c.dictation("start"), Some(id.clone()));

    // The tap carries the microphone while the meeting records.
    let file = c.tap().unwrap();
    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let tap = voice_engine::meetings::tap::open(&file, {
        let heard = Arc::clone(&heard);
        Box::new(move |pcm: &[i16]| {
            heard.fetch_add(pcm.len(), Ordering::SeqCst);
        })
    })
    .unwrap();
    wait_for("tapped audio", || heard.load(Ordering::SeqCst) >= 8000);
    drop(tap);
    c.dictation("end");

    c.stop(Some(&id), "user").unwrap();
    let ended = wait_change(&changes, "the end", |c| matches!(c, Change::Ended { .. }));
    assert!(matches!(ended, Change::Ended { reason, .. } if reason == "user"));
    assert!(c.current().is_none());
    assert!(!meeting_dir(&data, &id).join("recorder.json").exists());
    assert_eq!(
        c.stop(Some(&id), "user").unwrap_err().code,
        ErrorCode::NotRecording
    );
    assert_eq!(
        c.stop(Some("m-20000101-000000-0000"), "user")
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );

    let lines = lines(&data, &id);
    let kinds: Vec<String> = lines
        .iter()
        .map(|l| {
            serde_json::to_value(l).unwrap()["kind"]
                .as_str()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    for kind in ["session", "pause", "resume", "mark", "dictation", "stop"] {
        assert!(kinds.contains(&kind.to_string()), "{kind} in {kinds:?}");
    }
    assert!(lines.iter().any(|l| matches!(l, Line::Mark { id, label: Some(label), .. } if id == "k-1" && label == "Decision")));
    assert!(lines
        .iter()
        .any(|l| matches!(l, Line::Dictation { state, .. } if state == "end")));
    assert!(matches!(lines.last(), Some(Line::Stop { reason, .. }) if reason == "user"));
    c.shutdown();
}

fn the_watchdog_resumes_a_killed_recorder() {
    let dir = TempDir::new("watchdog");
    let data = dir.join("data");
    let (c, changes) = controller(&data, timing());
    let id = c.start(StartArgs::default(), StartedBy::Voice).unwrap();
    wait_clock(&c, 1500);
    let first = recorder_pid(&data, &id);
    kill(first);
    wait_change(&changes, "the restart", |c| {
        matches!(c, Change::Restarted { attempt: 1, .. })
    });
    let second = recorder_pid(&data, &id);
    assert_ne!(first, second);
    let restarted_at = c.current().unwrap().at_ms;
    wait_clock(&c, restarted_at + 1000);
    c.stop(None, "user").unwrap();
    let lines = lines(&data, &id);
    let sessions: Vec<bool> = lines
        .iter()
        .filter_map(|l| match l {
            Line::Session { resumed, .. } => Some(*resumed),
            _ => None,
        })
        .collect();
    assert_eq!(sessions, [false, true]);
    assert!(
        lines.iter().any(|l| matches!(l, Line::Crash { .. })),
        "the gap is marked"
    );
    assert!(matches!(lines.last(), Some(Line::Stop { reason, .. }) if reason == "user"));
    assert!(
        !alive(second) || {
            std::thread::sleep(Duration::from_secs(2));
            !alive(second)
        }
    );
    c.shutdown();
}

fn the_watchdog_gives_up_after_its_restarts() {
    let dir = TempDir::new("give-up");
    let data = dir.join("data");
    let mut timing = timing();
    timing.backoff.truncate(2);
    let (c, changes) = controller(&data, timing);
    let id = c.start(StartArgs::default(), StartedBy::Voice).unwrap();
    for attempt in 1..=2 {
        wait_for("a recorder", || {
            read_recorder_file(&meeting_dir(&data, &id)).is_some()
        });
        wait_clock(&c, 500);
        kill(recorder_pid(&data, &id));
        wait_change(
            &changes,
            "a restart",
            |c| matches!(c, Change::Restarted { attempt: a, .. } if *a == attempt),
        );
    }
    wait_for("a recorder", || {
        read_recorder_file(&meeting_dir(&data, &id)).is_some()
    });
    kill(recorder_pid(&data, &id));
    let ended = wait_change(&changes, "the end", |c| matches!(c, Change::Ended { .. }));
    assert!(matches!(ended, Change::Ended { reason, .. } if reason == "interrupted"));
    assert_eq!(count(&changes, |c| matches!(c, Change::Notify { .. })), 1);
    assert!(
        matches!(lines(&data, &id).last(), Some(Line::Stop { reason, .. }) if reason == "interrupted")
    );
    assert!(c.current().is_none());
    c.shutdown();
}

fn a_new_controller_reattaches_to_a_live_recorder() {
    let dir = TempDir::new("reattach");
    let data = dir.join("data");
    let (a, _) = controller(&data, timing());
    let id = a.start(StartArgs::default(), StartedBy::Voice).unwrap();
    wait_clock(&a, 500);
    let pid = recorder_pid(&data, &id);
    // The app quits: the recorder goes on.
    a.shutdown();
    drop(a);
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(pid), "the recorder outlives its controller");

    let (b, changes) = controller(&data, timing());
    b.recover();
    let reattached = wait_change(&changes, "the reattach", |c| {
        matches!(c, Change::Reattached { .. })
    });
    assert!(
        matches!(&reattached, Change::Reattached { id: found, paused: false, at_ms } if *found == id && *at_ms >= 500)
    );
    assert_eq!(b.current().unwrap().id, id);
    assert_eq!(b.mark(None, None).unwrap().0, "k-1");
    b.stop(None, "rukoo").unwrap();
    assert!(
        matches!(lines(&data, &id).last(), Some(Line::Stop { reason, .. }) if reason == "rukoo")
    );
    wait_for("the recorder to exit", || !alive(pid));
    b.shutdown();
}

fn a_reattach_ends_a_dictation_the_old_app_left_open() {
    let dir = TempDir::new("reattach-dictation");
    let data = dir.join("data");
    let (a, _) = controller(&data, timing());
    let id = a.start(StartArgs::default(), StartedBy::Voice).unwrap();
    wait_clock(&a, 500);
    // The app says the user dictates, then quits or crashes before the key comes up. This
    // dictation has no tap, so only the next app can end it.
    assert_eq!(a.dictation("start"), Some(id.clone()));
    a.shutdown();
    drop(a);
    std::thread::sleep(Duration::from_millis(300));

    let (b, changes) = controller(&data, timing());
    b.recover();
    let Change::Reattached { at_ms, .. } = wait_change(&changes, "the reattach", |c| {
        matches!(c, Change::Reattached { .. })
    }) else {
        unreachable!()
    };
    wait_clock(&b, at_ms + 1_000);
    b.stop(None, "user").unwrap();
    let dictation: Vec<(String, u64)> = lines(&data, &id)
        .iter()
        .filter_map(|l| match l {
            Line::Dictation { state, t } => Some((state.clone(), *t)),
            _ => None,
        })
        .collect();
    assert_eq!(dictation.len(), 2, "{dictation:?}");
    assert_eq!(dictation[1].0, "end", "{dictation:?}");
    // It ends at the reattach, not at the stop: the rest of the meeting isn't dictation.
    assert!(
        dictation[1].1 <= at_ms + 500,
        "the dictation ended at {} after a reattach at {at_ms}",
        dictation[1].1
    );
    b.shutdown();
}

/// Holds the meeting's timeline the way a live recorder does: open for writing, readable by
/// others, and refusing a second writer.
#[cfg(windows)]
fn hold_timeline(data: &Path, id: &str) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_DELETE: u32 = 0x4;
    let path = meeting_dir(data, id).join("timeline.jsonl");
    let mut held = None;
    // The killed recorder's own handle goes a moment after it does.
    wait_for("the timeline to come free", || {
        held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .open(&path)
            .ok();
        held.is_some()
    });
    held.unwrap()
}

/// A recorder that holds the meeting but doesn't answer, as one whose control port is full: the
/// watchdog's restart and `finalize` both find it busy. The meeting is live, so it never ends
/// on its own, and Stop keeps asking until it can finish the meeting.
#[cfg(windows)]
fn a_recorder_that_holds_the_meeting_without_answering_keeps_it_live() {
    let dir = TempDir::new("busy");
    let data = dir.join("data");
    let mut timing = timing();
    // Room to take the timeline between the kill and the first restart; the later ones are
    // quick, so giving up would come soon.
    timing.backoff = [1_500, 100, 100]
        .into_iter()
        .map(Duration::from_millis)
        .collect();
    let (c, changes) = controller(&data, timing);
    let id = c.start(StartArgs::default(), StartedBy::Voice).unwrap();
    wait_clock(&c, 500);
    kill(recorder_pid(&data, &id));
    let held = hold_timeline(&data, &id);

    // Three restarts, all refused as busy, used to end the meeting as interrupted after about
    // four seconds.
    std::thread::sleep(Duration::from_secs(7));
    assert_eq!(
        count(&changes, |c| matches!(c, Change::Ended { .. })),
        0,
        "{:?}",
        lock(&changes)
    );
    assert_eq!(c.current().map(|r| r.id), Some(id.clone()));

    // Stop finds the meeting held for a few seconds, then free.
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        drop(held);
    });
    c.stop(None, "user").unwrap();
    release.join().unwrap();
    let ended = wait_change(&changes, "the end", |c| matches!(c, Change::Ended { .. }));
    assert!(matches!(ended, Change::Ended { reason, .. } if reason == "user"));
    assert!(
        matches!(lines(&data, &id).last(), Some(Line::Stop { reason, .. }) if reason == "user")
    );
    assert!(c.current().is_none());
    c.shutdown();
}

fn a_cut_off_meeting_is_finished_at_start() {
    let dir = TempDir::new("finalize");
    let data = dir.join("data");
    let (a, _) = controller(&data, timing());
    let id = a.start(StartArgs::default(), StartedBy::Voice).unwrap();
    wait_clock(&a, 1500);
    a.shutdown();
    drop(a);
    kill(recorder_pid(&data, &id));
    assert_eq!(
        voice_engine::meetings::Library::open(&data)
            .unwrap()
            .interrupted(),
        std::slice::from_ref(&id)
    );

    let (b, changes) = controller(&data, timing());
    b.recover();
    wait_change(
        &changes,
        "the finalize",
        |c| matches!(c, Change::Finalized { id: f } if *f == id),
    );
    assert!(b.current().is_none());
    let lines = lines(&data, &id);
    assert!(
        matches!(lines.last(), Some(Line::Stop { reason, t, .. }) if reason == "interrupted" && *t >= 1000)
    );
    assert!(b.library().interrupted().is_empty());
    // A second start finds nothing left to do.
    let (c, again) = controller(&data, timing());
    c.recover();
    std::thread::sleep(Duration::from_millis(200));
    assert!(lock(&again).is_empty());
    b.shutdown();
    c.shutdown();
}

/// Media links without bucket sync or screen recording.
fn no_media(data: &Path, controller: &Arc<Controller>) -> Arc<MediaService> {
    MediaService::new(
        data,
        Arc::clone(controller),
        None,
        Arc::new(NoScreenMedia),
        Arc::new(|_| None),
    )
}

/// A screen supervisor without a screen program: meetings it would follow record no screen.
fn no_screen(data: &Path) -> Arc<ScreenSupervisor> {
    ScreenSupervisor::new(
        ScreenOptions {
            data_dir: data.to_path_buf(),
            exe: data.join("no-screen.exe"),
            test: None,
            test_monitor: None,
            timing: Default::default(),
            free_bytes: None,
        },
        None,
        |_| {},
    )
    .unwrap()
}

fn control(c: &Arc<Controller>, data: &Path) -> Control {
    Control::new(
        Arc::clone(c),
        no_screen(data),
        EventLog::open(data).unwrap(),
        Arc::default(),
        no_media(data, c),
    )
}

fn rukoo_commands_go_through_meeting_control() {
    let dir = TempDir::new("control");
    let data = dir.join("data");
    let (c, changes) = controller(&data, timing());
    let control = control(&c, &data);
    assert_eq!(MeetingControl::status(&control).recording, None);
    let id = MeetingControl::start(
        &control,
        StartArgs {
            title: Some("From Rukoo".into()),
            tracks: Some(vec![Track::Mic]),
            screen: None,
            context: Some(json!({"calendarEvent": "abc"}).as_object().unwrap().clone()),
        },
    )
    .unwrap();
    let status = MeetingControl::status(&control);
    assert_eq!(status.recording.map(|m| m.into_string()), Some(id.clone()));
    let meeting: Value = serde_json::from_slice(
        &std::fs::read(meeting_dir(&data, &id).join("meeting.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (meeting["startedBy"].as_str(), meeting["tracks"].clone()),
        (Some("rukoo"), json!(["mic"]))
    );
    assert_eq!(meeting["context"]["calendarEvent"], "abc");
    wait_clock(&c, 300);
    control.pause(&id).unwrap();
    assert_eq!(control.delete(&id).unwrap_err().code, ErrorCode::Busy);
    control.resume(&id).unwrap();
    assert_eq!(control.mark(&id, Some("x")).unwrap().0, "k-1");
    // Nothing follows this meeting here, so its snapshot is what meeting.json says.
    let snapshot = control.get(&id).unwrap();
    assert_eq!(snapshot["meeting"]["id"], id.as_str());
    assert_eq!(snapshot["meeting"]["title"], "From Rukoo");
    assert_eq!(snapshot["meeting"]["state"], "recording");
    assert_eq!(
        control.frame(&id, 0).unwrap_err().code,
        ErrorCode::NotAvailable
    );
    assert_eq!(
        control.get("m-20000101-000000-0000").unwrap_err().code,
        ErrorCode::NotFound
    );
    let list = control.list(10);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "From Rukoo");
    control.stop(&id).unwrap();
    assert!(
        matches!(wait_change(&changes, "the end", |c| matches!(c, Change::Ended { .. })), Change::Ended { reason, .. } if reason == "rukoo")
    );
    assert_eq!(control.stop(&id).unwrap_err().code, ErrorCode::NotRecording);
    assert_eq!(
        control.delete(&id).unwrap_err().code,
        ErrorCode::Busy,
        "processing data cannot be deleted while a pipeline may write it"
    );
    control
        .log
        .append(
            "meeting.state",
            Some(&id),
            json!({"state": "done", "at": 1000, "endedAt": 1793003605120_u64}),
        )
        .unwrap();
    control.delete(&id).unwrap();
    assert!(!meeting_dir(&data, &id).exists());
    assert!(!c.library().root().join(format!(".deleted-{id}")).exists());
    let (_, removed) = control.log.meeting_events(&id).unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].event, "meeting.removed");
    assert_eq!(removed[0].data["reason"], "user");
    assert_eq!(control.get(&id).unwrap_err().code, ErrorCode::NotFound);
    c.shutdown();
}

/// Starts a meeting the way `by` would, with `screen` asked for, and returns what its
/// `meeting.json` and `meeting.started` say it records of the screen.
fn screen_of_a_meeting(
    default: ScreenMode,
    available: bool,
    by: StartedBy,
    screen: Option<ScreenMode>,
) -> (String, Option<ScreenMode>) {
    let dir = TempDir::new("screen-choice");
    let data = dir.join("data");
    let (c, _) = controller(&data, timing());
    c.set_screen(default, available);
    let control = control(&c, &data);
    // What a start without `screen` gets, which is what Rukoo labels as Voice's default.
    let expected = if available { default } else { ScreenMode::Off };
    assert_eq!(c.screen_default(), expected);
    assert_eq!(
        MeetingControl::status(&control).screen_default,
        Some(expected)
    );
    let args = StartArgs {
        tracks: Some(vec![Track::Mic]),
        screen,
        ..StartArgs::default()
    };
    let id = match by {
        StartedBy::Rukoo => MeetingControl::start(&control, args).unwrap(),
        StartedBy::Voice => c.start(args, StartedBy::Voice).unwrap(),
    };
    let meeting: Value = serde_json::from_slice(
        &std::fs::read(meeting_dir(&data, &id).join("meeting.json")).unwrap(),
    )
    .unwrap();
    let file = c.library().dir(&id).unwrap().meeting().unwrap();
    assert_eq!(file.started()["screen"], meeting["screen"]);
    c.stop(Some(&id), "user").unwrap();
    c.shutdown();
    (
        meeting["screen"].as_str().unwrap().to_string(),
        MeetingControl::status(&control).screen_default,
    )
}

fn a_meeting_records_the_settings_default_screen() {
    let (screen, default) = screen_of_a_meeting(ScreenMode::Frames, true, StartedBy::Voice, None);
    assert_eq!(screen, "frames");
    assert_eq!(default, Some(ScreenMode::Frames), "status.screenDefault");
    let (screen, _) = screen_of_a_meeting(ScreenMode::Video, true, StartedBy::Rukoo, None);
    assert_eq!(
        screen, "video",
        "Rukoo's start without screen gets the default"
    );
}

fn rukoo_overrides_the_screen() {
    let (screen, _) = screen_of_a_meeting(
        ScreenMode::Frames,
        true,
        StartedBy::Rukoo,
        Some(ScreenMode::Video),
    );
    assert_eq!(screen, "video");
    let (screen, _) = screen_of_a_meeting(
        ScreenMode::Video,
        true,
        StartedBy::Rukoo,
        Some(ScreenMode::Off),
    );
    assert_eq!(screen, "off");
}

fn no_screen_process_means_off() {
    let (screen, default) = screen_of_a_meeting(
        ScreenMode::Video,
        false,
        StartedBy::Rukoo,
        Some(ScreenMode::Video),
    );
    assert_eq!(screen, "off");
    assert_eq!(
        default,
        Some(ScreenMode::Off),
        "Rukoo hears the default a meeting really gets"
    );
    let (screen, default) = screen_of_a_meeting(ScreenMode::Frames, false, StartedBy::Voice, None);
    assert_eq!(screen, "off");
    assert_eq!(default, Some(ScreenMode::Off));
}

fn engine_harness(name: &str) -> Harness {
    let mut h = Harness::new(name);
    h.recorder_exe = recorder_exe();
    h.timing = timing();
    h.test_sources = Some("mic=synthetic:tone:440,call=synthetic:idle".into());
    h.config(json!({}));
    h
}

fn stored(data: &Path) -> Vec<voice_bridge::Stored> {
    EventLog::open(data).unwrap().after(0).unwrap()
}

fn a_meeting_through_the_engine() {
    let h = engine_harness("engine-meeting");
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    let pill = h.sink.wait_phase(PillPhase::Meeting);
    let meeting = pill.meeting.unwrap();
    assert_eq!(
        (meeting.state.as_str(), meeting.stop_enabled),
        ("recording", true)
    );
    assert_eq!(engine.tray_state().meeting.as_deref(), Some("recording"));
    wait_for("meeting.started for Rukoo", || {
        stored(&h.data())
            .iter()
            .any(|s| s.event == "meeting.started" && s.meeting.as_deref() == Some(id.as_str()))
    });
    assert!(stored(&h.data())
        .iter()
        .any(|s| s.event == "status" && s.data["recording"] == id.as_str()));

    // Dictation during the meeting reads the recorder's tap, not a second capture.
    h.speech.say("noted");
    h.keys.down(Key::F13);
    wait_for("dictation to listen", || {
        h.sink
            .last_pill()
            .is_some_and(|p| p.phase == PillPhase::Listening)
    });
    assert!(
        !h.sink.last_pill().unwrap().meeting.unwrap().stop_enabled,
        "no stop button while dictating"
    );
    std::thread::sleep(Duration::from_millis(700));
    h.keys.up(Key::F13);
    wait_for("the insertion", || h.input.texts() == ["noted"]);
    assert_eq!(
        h.mic.opens.load(Ordering::SeqCst),
        0,
        "the microphone was captured once"
    );
    assert!(
        h.speech.heard()[0] >= 8000,
        "the tap delivered the tone: {:?}",
        h.speech.heard()
    );

    engine.pause_meeting().unwrap();
    wait_for("a paused pill", || {
        h.sink
            .last_pill()
            .and_then(|p| p.meeting)
            .is_some_and(|m| m.state == "paused")
    });
    engine.resume_meeting().unwrap();
    // Two clicks stop it; one only asks.
    engine.pill_stop_clicked();
    wait_for("Stop?", || {
        h.sink
            .last_pill()
            .and_then(|p| p.meeting)
            .is_some_and(|m| m.confirm_stop)
    });
    assert!(engine.tray_state().meeting.is_some());
    engine.pill_stop_clicked();
    wait_for("saving", || {
        h.sink
            .last_pill()
            .and_then(|p| p.meeting)
            .is_some_and(|m| m.state == "processing")
    });
    h.sink.wait_hidden_after(PillPhase::Meeting);
    assert_eq!(engine.tray_state().meeting, None);
    wait_for("meeting.state processing for Rukoo", || {
        stored(&h.data())
            .iter()
            .any(|s| s.event == "meeting.state" && s.data["state"] == "processing")
    });
    // voice-meetings transcribes what was recorded, then the meeting is done.
    wait_for("meeting.state done for Rukoo", || {
        stored(&h.data())
            .iter()
            .any(|s| s.event == "meeting.state" && s.data["state"] == "done")
    });
    let events = stored(&h.data());
    let passages: Vec<&voice_bridge::Stored> =
        events.iter().filter(|s| s.event == "passage").collect();
    assert!(!passages.is_empty(), "the tone was transcribed");
    assert!(passages
        .iter()
        .all(|p| p.data["text"] == "Meeting words." && p.meeting.as_deref() == Some(&id)));
    assert!(
        passages.iter().any(|p| p.data["track"] == "mic"),
        "the microphone has passages"
    );
    assert_eq!(
        h.speech.heard().len(),
        1,
        "dictation's recognizer heard only the dictation"
    );
    assert!(!lock(&h.speech.meeting_heard).is_empty());
    assert_eq!(lock(&h.speech.meeting_providers)[0], "orukeet");
    // Rukoo's meeting.get has every passage the log has.
    let control = control(
        &Controller::new(
            ControllerOptions {
                data_dir: h.data(),
                recorder_exe: recorder_exe(),
                sources: sources(),
                timing: timing(),
            },
            |_| {},
        )
        .unwrap(),
        &h.data(),
    );
    let snapshot = control.get(&id).unwrap();
    assert_eq!(snapshot["meeting"]["state"], "done");
    assert_eq!(
        snapshot["passages"].as_array().unwrap().len(),
        passages
            .iter()
            .map(|p| p.data["id"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len()
    );
    control.controller.shutdown();
    assert!(
        events
            .iter()
            .any(|s| s.event == "mark" && s.data["kind"] == "dictation"),
        "the dictation stretch is marked"
    );
    assert!(events
        .iter()
        .any(|s| s.event == "meeting.state" && s.data["state"] == "paused"));
    let lines = lines(&h.data(), &id);
    assert!(lines
        .iter()
        .any(|l| matches!(l, Line::Dictation { state, .. } if state == "start")));
    assert!(matches!(lines.last(), Some(Line::Stop { reason, .. }) if reason == "user"));
    engine.shutdown();
}

fn a_restarted_engine_reattaches_and_stops_the_meeting() {
    let h = engine_harness("engine-reattach");
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    wait_for("a recording", || {
        h.sink
            .last_pill()
            .is_some_and(|p| p.phase == PillPhase::Meeting)
    });
    let pid = recorder_pid(&h.data(), &id);
    // Quitting Voice never stops a recorder that records.
    engine.shutdown();
    drop(engine);
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(pid));
    assert!(read_recorder_file(&meeting_dir(&h.data(), &id)).is_some());

    let again = h.start();
    wait_for("the reattached meeting", || {
        again.tray_state().meeting.as_deref() == Some("recording")
    });
    again.stop_meeting(StopReason::User).unwrap();
    assert_eq!(again.tray_state().meeting, None);
    assert!(
        matches!(lines(&h.data(), &id).last(), Some(Line::Stop { reason, .. }) if reason == "user")
    );
    wait_for("the recorder to exit", || !alive(pid));
    // Two engines followed the meeting; Rukoo got each event once.
    wait_done(&h.data(), &id);
    assert_once(&h.data(), &id);
    again.shutdown();
}

fn wait_done(data: &Path, id: &str) {
    wait_for("meeting.state done", || {
        stored(data).iter().any(|s| {
            s.meeting.as_deref() == Some(id)
                && s.event == "meeting.state"
                && s.data["state"] == "done"
        })
    });
}

/// No passage revision, mark, speaker or state reached the log twice.
fn assert_once(data: &Path, id: &str) {
    let mut seen = std::collections::HashSet::new();
    for s in stored(data)
        .iter()
        .filter(|s| s.meeting.as_deref() == Some(id))
    {
        let key = match s.event.as_str() {
            "passage" => format!("passage {} {}", s.data["id"], s.data["revision"]),
            "mark" | "speaker" => format!("{} {}", s.event, s.data["id"]),
            "meeting.state" => format!("state {} {}", s.data["state"], s.data["at"]),
            other => other.to_string(),
        };
        assert!(seen.insert(key.clone()), "{key} reached Rukoo twice");
    }
}

fn a_cut_off_meeting_is_transcribed_at_the_next_start() {
    let h = engine_harness("engine-cut-off");
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    wait_for("1.5 s of audio", || {
        lines(&h.data(), &id)
            .iter()
            .any(|l| matches!(l, Line::Heartbeat { t, .. } if *t >= 1500))
    });
    let pid = recorder_pid(&h.data(), &id);
    // Voice quits, then the recorder dies while nothing watches it.
    engine.shutdown();
    drop(engine);
    kill(pid);

    let again = h.start();
    h.sink
        .wait_note(voice_engine::meetings::controller::NOTIFY_MEETING);
    assert!(
        matches!(lines(&h.data(), &id).last(), Some(Line::Stop { reason, .. }) if reason == "interrupted")
    );
    wait_done(&h.data(), &id);
    let events = stored(&h.data());
    assert!(
        events
            .iter()
            .any(|s| s.event == "passage" && s.meeting.as_deref() == Some(id.as_str())),
        "what was recorded before the cut got transcribed"
    );
    assert!(events.iter().any(|s| s.event == "meeting.state"
        && s.data["state"] == "processing"
        && s.data["reason"] == "interrupted"));
    assert_once(&h.data(), &id);
    again.shutdown();
}

fn transcription_waits_for_consent_and_starts_when_it_is_given() {
    let h = engine_harness("engine-consent");
    let engine = h.start();
    let mistral = |consent: bool| json!({"target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": "voxtral-mini-latest", "apiKey": "sk-test-not-a-real-key", "consent": consent});
    engine.settings_save_provider(mistral(false)).unwrap();
    let id = engine.start_meeting().unwrap();
    wait_for("1 s of audio", || {
        lines(&h.data(), &id)
            .iter()
            .any(|l| matches!(l, Line::Heartbeat { t, .. } if *t >= 1000))
    });
    engine.stop_meeting(StopReason::User).unwrap();
    wait_for("meeting.state processing", || {
        stored(&h.data())
            .iter()
            .any(|s| s.event == "meeting.state" && s.data["state"] == "processing")
    });
    // Decision 10: the meeting recorded, and no audio went to Mistral without consent.
    std::thread::sleep(Duration::from_millis(1500));
    let events = stored(&h.data());
    assert!(!events.iter().any(|s| s.event == "passage"));
    assert!(!events
        .iter()
        .any(|s| s.event == "meeting.state" && s.data["state"] == "done"));
    assert!(lock(&h.speech.meeting_heard).is_empty());

    engine.settings_save_provider(mistral(true)).unwrap();
    wait_done(&h.data(), &id);
    assert!(stored(&h.data()).iter().any(|s| s.event == "passage"));
    assert_eq!(
        lock(&h.speech.meeting_providers).last().map(String::as_str),
        Some("mistral")
    );
    engine.shutdown();
}
