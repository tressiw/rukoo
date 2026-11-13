//! Meetings through the whole engine against the real recorder: the pill across two meetings,
//! dictation while a meeting records, and each meeting's audio retention.
//!
//! Like tests/controller.rs, this binary has its own main (`harness = false`) so it can also be
//! the recorder: run with `VOICE_ENGINE_TEST_ROLE=recorder` it calls `voice_recorder::run`.

mod support;

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::{lock, wait_for, Harness};
use voice_bridge::EventLog;
use voice_engine::meetings::controller::ControllerTiming;
use voice_engine::{PillPhase, StopReason};
use voice_input::Key;
use voice_recorder::meeting::read_recorder_file;
use voice_recorder::timeline::{self, Line};

const ROLE: &str = "VOICE_ENGINE_TEST_ROLE";

fn main() {
    if std::env::var(ROLE).as_deref() == Ok("recorder") {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Err(e) = voice_recorder::run(&args) {
            eprintln!("rukoo-voice-recorder: {e}");
            std::process::exit(e.code);
        }
        return;
    }
    // Single-threaded here, so setting the variable is safe; every recorder started from now on
    // inherits it.
    std::env::set_var(ROLE, "recorder");
    let tests: Vec<(&str, fn())> = vec![
        (
            "an_earlier_meeting_s_outcome_leaves_the_recording_one_on_the_pill",
            an_earlier_meeting_s_outcome_leaves_the_recording_one_on_the_pill,
        ),
        (
            "a_dictation_held_across_the_meeting_s_stop_keeps_its_audio",
            a_dictation_held_across_the_meeting_s_stop_keeps_its_audio,
        ),
        (
            "a_dictation_while_the_recorder_restarts_is_still_marked",
            a_dictation_while_the_recorder_restarts_is_still_marked,
        ),
        (
            "a_dictation_whose_recorder_dies_twice_is_marked_on_each_next_one",
            a_dictation_whose_recorder_dies_twice_is_marked_on_each_next_one,
        ),
        (
            "each_meeting_keeps_the_retention_it_started_with",
            each_meeting_keeps_the_retention_it_started_with,
        ),
        (
            "a_silent_call_says_so_once_and_settings_learns_it",
            a_silent_call_says_so_once_and_settings_learns_it,
        ),
    ];
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

fn harness(name: &str, config: Value) -> Harness {
    let mut h = Harness::new(name);
    h.recorder_exe = std::env::current_exe().unwrap();
    h.timing = ControllerTiming {
        poll: Duration::from_millis(100),
        start_timeout: Duration::from_secs(20),
        backoff: vec![
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ],
        stable_after: Duration::from_secs(60),
        ping_failures: 3,
        recorder_args: vec!["--segment-ms".into(), "1000".into()],
    };
    h.test_sources = Some("mic=synthetic:tone:440,call=synthetic:idle".into());
    h.config(config);
    h
}

fn meeting_dir(data: &Path, id: &str) -> PathBuf {
    data.join("meetings").join(id)
}

fn lines(data: &Path, id: &str) -> Vec<Line> {
    timeline::read(&meeting_dir(data, id).join("timeline.jsonl")).unwrap_or_default()
}

fn wait_recorded(data: &Path, id: &str, at_least: u64) {
    wait_for(&format!("{at_least} ms of audio"), || {
        lines(data, id)
            .iter()
            .any(|l| matches!(l, Line::Heartbeat { t, .. } if *t >= at_least))
    });
}

fn wait_done(data: &Path, id: &str) {
    wait_for("meeting.state done", || {
        EventLog::open(data)
            .unwrap()
            .after(0)
            .unwrap()
            .iter()
            .any(|s| {
                s.meeting.as_deref() == Some(id)
                    && s.event == "meeting.state"
                    && s.data["state"] == "done"
            })
    });
}

fn meeting_json(data: &Path, id: &str) -> Value {
    serde_json::from_slice(&std::fs::read(meeting_dir(data, id).join("meeting.json")).unwrap())
        .unwrap()
}

/// The dictation lines the recorder wrote since its newest session began.
fn dictation_since_restart(data: &Path, id: &str) -> Vec<String> {
    let lines = lines(data, id);
    let session = lines
        .iter()
        .rposition(|l| matches!(l, Line::Session { .. }))
        .unwrap_or(0);
    lines[session..]
        .iter()
        .filter_map(|l| match l {
            Line::Dictation { state, .. } => Some(state.clone()),
            _ => None,
        })
        .collect()
}

fn sessions(data: &Path, id: &str) -> usize {
    lines(data, id)
        .iter()
        .filter(|l| matches!(l, Line::Session { .. }))
        .count()
}

/// Ends a recorder this test started, by its process id.
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

fn recorder_pid(data: &Path, id: &str) -> u32 {
    read_recorder_file(&meeting_dir(data, id))
        .expect("recorder.json")
        .pid
}

fn pill_meeting_state(h: &Harness) -> Option<String> {
    h.sink
        .last_pill()
        .filter(|p| p.visible)
        .and_then(|p| p.meeting)
        .map(|m| m.state)
}

// The tests ----------------------------------------------------------------------------------

fn an_earlier_meeting_s_outcome_leaves_the_recording_one_on_the_pill() {
    let h = harness("pill-two-meetings", json!({}));
    let engine = h.start();
    let mistral = |consent: bool| json!({"target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": "voxtral-mini-latest", "apiKey": "sk-test-not-a-real-key", "consent": consent});
    engine.settings_save_provider(mistral(false)).unwrap();
    let a = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &a, 1000);
    engine.stop_meeting(StopReason::User).unwrap();
    // A waits for consent, so it still transcribes while B records.
    let b = engine.start_meeting().unwrap();
    assert_ne!(a, b);
    wait_for("B on the pill", || {
        pill_meeting_state(&h).as_deref() == Some("recording")
    });
    engine.settings_save_provider(mistral(true)).unwrap();
    wait_done(&h.data(), &a);
    // The engine reads meeting outcomes from the log once a second.
    std::thread::sleep(Duration::from_millis(2500));
    let pill = h.sink.last_pill().unwrap();
    assert_eq!(
        (pill.visible, pill.phase),
        (true, PillPhase::Meeting),
        "B is still on the pill"
    );
    let meeting = pill.meeting.unwrap();
    assert_eq!(
        (meeting.state.as_str(), meeting.stop_enabled),
        ("recording", true)
    );
    assert_eq!(engine.tray_state().meeting.as_deref(), Some("recording"));
    // The stop button still works: two clicks stop B.
    engine.pill_stop_clicked();
    engine.pill_stop_clicked();
    wait_for("B to stop", || engine.tray_state().meeting.is_none());
    engine.shutdown();
}

fn a_dictation_held_across_the_meeting_s_stop_keeps_its_audio() {
    let h = harness("dictation-across-stop", json!({}));
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &id, 500);
    h.speech.say("all of it");
    h.keys.down(Key::F13);
    wait_for("dictation to listen", || {
        h.sink
            .last_pill()
            .is_some_and(|p| p.phase == PillPhase::Listening)
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(h.mic.opens.load(Ordering::SeqCst), 0, "the tap carries it");
    // The meeting stops while the key is still down; the rest comes from Voice's own capture.
    engine.stop_meeting(StopReason::User).unwrap();
    wait_for("Voice's own microphone", || {
        h.mic.opens.load(Ordering::SeqCst) == 1
    });
    wait_for("its audio", || {
        h.mic.delivered.load(Ordering::SeqCst) >= 6400
    });
    h.keys.up(Key::F13);
    wait_for("the insertion", || h.input.texts() == ["all of it"]);
    let heard = h.speech.heard()[0];
    assert!(
        heard >= 4000 + 6400,
        "the tap's audio and the microphone's after the stop: {heard} samples"
    );
    engine.shutdown();
}

fn a_dictation_while_the_recorder_restarts_is_still_marked() {
    let mut h = harness("dictation-restart", json!({}));
    // The first restart waits 1.5 s, so the press below lands while it restarts.
    h.timing.backoff[0] = Duration::from_millis(1500);
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &id, 500);
    // The recorder dies; the dictation starts while the watchdog brings it back, so the tap
    // isn't there and dictation reads Voice's own capture.
    kill(recorder_pid(&h.data(), &id));
    std::thread::sleep(Duration::from_millis(400));
    h.speech.say("not said in the call");
    h.keys.down(Key::F13);
    wait_for("Voice's own microphone", || {
        h.mic.opens.load(Ordering::SeqCst) == 1
    });
    assert_eq!(sessions(&h.data(), &id), 1, "pressed before the restart");
    wait_for("the restarted recorder", || sessions(&h.data(), &id) == 2);
    wait_for("the dictation marked on it", || {
        dictation_since_restart(&h.data(), &id) == ["start"]
    });
    std::thread::sleep(Duration::from_millis(300));
    h.keys.up(Key::F13);
    wait_for("the stretch to end", || {
        dictation_since_restart(&h.data(), &id) == ["start", "end"]
    });
    wait_for("the insertion", || {
        h.input.texts() == ["not said in the call"]
    });
    engine.stop_meeting(StopReason::User).unwrap();
    engine.shutdown();
}

fn a_dictation_whose_recorder_dies_twice_is_marked_on_each_next_one() {
    let h = harness("dictation-recorder-dies", json!({}));
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    wait_recorded(&h.data(), &id, 500);
    h.speech.say("keeps going");
    h.keys.down(Key::F13);
    wait_for("the dictation marked", || {
        dictation_since_restart(&h.data(), &id) == ["start"]
    });
    // The recorder dies mid-dictation: its tap ends, Voice's own capture takes over, and the
    // restarted recorder hears about the dictation too.
    kill(recorder_pid(&h.data(), &id));
    wait_for("Voice's own microphone", || {
        h.mic.opens.load(Ordering::SeqCst) == 1
    });
    wait_for("the restarted recorder", || sessions(&h.data(), &id) == 2);
    wait_for("the dictation marked on it", || {
        dictation_since_restart(&h.data(), &id) == ["start"]
    });
    // It dies again with the key still down. Voice's own capture has no tap to end, so only the
    // restart itself can tell the next recorder.
    kill(recorder_pid(&h.data(), &id));
    wait_for("the second restarted recorder", || {
        sessions(&h.data(), &id) == 3
    });
    wait_for("the dictation marked on that one too", || {
        dictation_since_restart(&h.data(), &id) == ["start"]
    });
    assert_eq!(
        h.mic.opens.load(Ordering::SeqCst),
        1,
        "Voice's own capture carries on"
    );
    h.keys.up(Key::F13);
    wait_for("the stretch to end", || {
        dictation_since_restart(&h.data(), &id) == ["start", "end"]
    });
    wait_for("the insertion", || h.input.texts() == ["keeps going"]);
    engine.stop_meeting(StopReason::User).unwrap();
    engine.shutdown();
}

fn each_meeting_keeps_the_retention_it_started_with() {
    let h = harness("retention", json!({"meetings_retention": "keep"}));
    let engine = h.start();
    let kept = engine.start_meeting().unwrap();
    assert_eq!(meeting_json(&h.data(), &kept)["retention"], "keep");
    wait_recorded(&h.data(), &kept, 1000);
    // The setting changes while it records; this meeting keeps what it started with.
    engine
        .settings_set("meetings.retention", json!("after_transcription"))
        .unwrap();
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &kept);
    // The engine sweeps right after it reads `done`, within a second or two.
    std::thread::sleep(Duration::from_millis(2500));
    assert!(
        meeting_dir(&h.data(), &kept).join("audio").is_dir(),
        "the earlier meeting kept its audio"
    );

    let next = engine.start_meeting().unwrap();
    assert_eq!(
        meeting_json(&h.data(), &next)["retention"],
        "after_transcription"
    );
    wait_recorded(&h.data(), &next, 1000);
    engine.stop_meeting(StopReason::User).unwrap();
    wait_done(&h.data(), &next);
    wait_for("the new meeting's audio to go", || {
        !meeting_dir(&h.data(), &next).join("audio").exists()
    });
    assert!(meeting_dir(&h.data(), &kept).join("audio").is_dir());
    engine.shutdown();
}

/// A call track of exact zeros while another app plays, as a Mac gives without the System Audio
/// Recording grant: the pill and a notification say so once, the recording goes on, and Settings'
/// System Audio Recording row shows what the meeting learned.
fn a_silent_call_says_so_once_and_settings_learns_it() {
    let mut h = harness("silent-call", json!({}));
    h.test_sources = Some("mic=synthetic:tone:440,call=synthetic:tap,playing".into());
    h.timing
        .recorder_args
        .extend(["--silence-ms".to_string(), "1000".to_string()]);
    let engine = h.start();
    let id = engine.start_meeting().unwrap();
    wait_for("the pill's call notice", || {
        lock(&h.sink.pills).iter().any(|p| {
            p.notice
                .as_ref()
                .is_some_and(|n| n.key == "pill.callSilent")
        })
    });
    let note = h.sink.wait_note("app.notify.meeting");
    assert_eq!(
        note.body_key.as_deref(),
        Some("app.notify.callSilent"),
        "{note:?}"
    );
    assert!(
        note.body.contains("System Audio Recording"),
        "{}",
        note.body
    );
    assert!(
        lines(&h.data(), &id)
            .iter()
            .any(|l| matches!(l, Line::Silent { track, .. } if track == "call")),
        "the timeline has no silent line"
    );
    // Told once: the next status rounds keep it silent without another notification.
    std::thread::sleep(Duration::from_millis(1500));
    let told = h
        .sink
        .notes()
        .iter()
        .filter(|n| n.body_key.as_deref() == Some("app.notify.callSilent"))
        .count();
    assert_eq!(told, 1);
    assert_eq!(pill_meeting_state(&h).as_deref(), Some("recording"));
    let state = engine.settings_state();
    if cfg!(target_os = "macos") {
        // Only a hint, so the row doesn't claim macOS refused.
        assert_eq!(
            state["permissions"]["systemAudio"], "unknown",
            "{}",
            state["permissions"]
        );
    } else {
        assert_eq!(state["permissions"], Value::Null);
    }
    engine.stop_meeting(StopReason::User).unwrap();
    engine.shutdown();
}
