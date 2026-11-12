//! What real devices do that the synthetic ones play here: a device that takes long to open, as
//! macOS' first process tap does while its permission prompt waits for an answer; a process tap
//! that records exact zeros because the system won't let Voice record; and a device that
//! delivers less audio than time passed, around a crash and resume.

mod common;

use std::time::{Duration, Instant};

use common::*;
use serde_json::json;

/// Times the recorder reports may trail the moment a test acted by a tick or two, more on CI.
const SLACK_MS: u64 = 300;

fn track_status(
    client: &mut voice_recorder::control::Client,
    token: &str,
    track: &str,
) -> serde_json::Value {
    let status = command(client, token, json!({ "cmd": "status" }));
    status["tracks"][track].clone()
}

#[test]
fn a_slow_open_holds_up_only_its_own_track() {
    if skip_on_slow_host("a_slow_open_holds_up_only_its_own_track") {
        return;
    }
    let m = TempMeeting::new("slow-tap");
    // The call's open blocks for 7 s, as macOS holds the first process tap until the user
    // answers the System Audio Recording prompt. Voice gives a recorder 20 s to answer.
    let begun = Instant::now();
    let mut child = m.record(
        &[
            "mic=synthetic:tone:440",
            "call=synthetic:tap,sound-at=0,open-ms=7000",
        ],
        &[],
    );
    let (mut client, token) = m.connect(1);
    assert!(
        begun.elapsed() < Duration::from_secs(3),
        "the recorder answered after {:?}",
        begun.elapsed()
    );
    let call = track_status(&mut client, &token, "call");
    assert_eq!(call["opening"], true, "{call}");
    assert_eq!(call["ok"], true, "a short open is no problem yet: {call}");
    assert_eq!(track_status(&mut client, &token, "mic")["ok"], true);

    m.wait_for_heartbeat(6_000);
    let call = track_status(&mut client, &token, "call");
    assert_eq!(call["ok"], false, "{call}");
    assert_eq!(call["error"], "The device is taking long to open.");

    m.wait_for_heartbeat(9_500);
    let call = track_status(&mut client, &token, "call");
    assert_eq!(call["ok"], true, "{call}");
    assert!(call.get("opening").is_none(), "{call}");
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    // The microphone recorded from the start, while the call waited.
    let mic = segments(&m, &lines, "mic");
    assert!(mic[0].t <= 1_000, "mic starts at {}", mic[0].t);
    for seg in &mic {
        if seg.pcm.len() > 2_000 {
            assert_tone(&seg.pcm, 440.0, &format!("mic segment {}", seg.seq));
        }
    }
    // The call is lost from the start of its open until it opened, so the gap is marked.
    let lost = of_kind(&lines, "device_lost");
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert_eq!(lost[0]["track"], "call");
    assert!(t(lost[0]) <= SLACK_MS, "lost at {}", t(lost[0]));
    let back = of_kind(&lines, "device");
    assert_eq!(back.len(), 1, "{back:?}");
    assert_eq!(back[0]["reason"], "reconnected");
    let back = t(back[0]);
    assert!((7_000..=7_000 + 1_000).contains(&back), "back at {back}");
    let call = segments(&m, &lines, "call");
    assert!(call[0].t >= back, "the call starts at {}", call[0].t);
    assert_eq!(of_kind(&lines, "track").len(), 2);
    assert!(of_kind(&lines, "suspend").is_empty());
}

#[test]
fn a_call_of_exact_zeros_is_reported_while_another_app_plays() {
    if skip_on_slow_host("a_call_of_exact_zeros_is_reported_while_another_app_plays") {
        return;
    }
    // Zeros from the start, as a denied tap gives them.
    let denied = TempMeeting::new("zeros-denied");
    // Zeros, then sound: the call was quiet at first.
    let late = TempMeeting::new("zeros-then-sound");
    // Sound, then zeros: the grant works and the call went quiet.
    let quiet = TempMeeting::new("sound-then-zeros");
    let extra = ["--silence-ms", "1500"];
    let mut a = denied.record(&["call=synthetic:tap,playing"], &extra);
    let mut b = late.record(&["call=synthetic:tap,sound-at=3000,playing"], &extra);
    let mut c = quiet.record(
        &["call=synthetic:tap,sound-at=0,sound-until=500,playing"],
        &extra,
    );
    let (mut ca, ta) = denied.connect(1);
    let (mut cb, tb) = late.connect(1);
    let (mut cc, tc) = quiet.connect(1);

    denied.wait_for_heartbeat(2_500);
    let status = track_status(&mut ca, &ta, "call");
    assert_eq!(status["silent"], true, "{status}");
    assert_eq!(status["ok"], true, "the recording goes on: {status}");
    late.wait_for_heartbeat(4_500);
    quiet.wait_for_heartbeat(4_500);
    let status = track_status(&mut cb, &tb, "call");
    assert!(status.get("silent").is_none(), "{status}");
    assert_eq!(status["heard"], true, "{status}");
    let status = track_status(&mut cc, &tc, "call");
    assert!(status.get("silent").is_none(), "{status}");
    assert_eq!(status["heard"], true, "{status}");
    for (client, token, child) in [
        (&mut ca, &ta, &mut a),
        (&mut cb, &tb, &mut b),
        (&mut cc, &tc, &mut c),
    ] {
        command(client, token, json!({ "cmd": "stop" }));
        assert!(wait_exit(child).success());
    }

    let lines = denied.lines();
    let silent = of_kind(&lines, "silent");
    assert_eq!(silent.len(), 1, "{silent:?}");
    assert_eq!(silent[0]["track"], "call");
    assert!(
        silent[0]["since"].as_u64().unwrap() <= SLACK_MS,
        "{}",
        silent[0]
    );
    assert!(
        (1_500..=1_500 + 2 * SLACK_MS).contains(&t(silent[0])),
        "{}",
        silent[0]
    );
    assert!(of_kind(&lines, "sound").is_empty());
    // The zeros are recorded all the same.
    let call = segments(&denied, &lines, "call");
    assert!(call.iter().map(|s| s.pcm.len()).sum::<usize>() > 16_000 * 2);
    assert!(call.iter().all(|s| s.pcm.iter().all(|&v| v == 0)));

    let lines = late.lines();
    assert_eq!(of_kind(&lines, "silent").len(), 1);
    let sound = of_kind(&lines, "sound");
    assert_eq!(sound.len(), 1, "{sound:?}");
    assert!(
        (3_000..=3_000 + SLACK_MS).contains(&t(sound[0])),
        "{}",
        sound[0]
    );

    let lines = quiet.lines();
    assert!(of_kind(&lines, "silent").is_empty());
    assert!(of_kind(&lines, "sound").is_empty());
}

/// Spike S2 saw a `segment_end` 418 ms before the next segment's start, after a crash and resume,
/// where other boundaries share the millisecond. The timeline does that when a device delivers
/// less audio than the clock says passed, by more than 100 ms but less than the 0.5 s after which
/// a call track is padded: the segment holds its 30 s of samples, and the next one starts where
/// the audio after them was heard. Nothing is lost at the boundary; the shortfall happened
/// inside the segment.
#[test]
fn a_shortfall_after_a_resume_moves_the_next_segment() {
    if skip_on_slow_host("a_shortfall_after_a_resume_moves_the_next_segment") {
        return;
    }
    let m = TempMeeting::new("shortfall");
    let extra = ["--segment-ms", "2000"];
    let mut child = m.record(
        &["mic=synthetic:tone:440", "call=synthetic:tap,sound-at=0"],
        &extra,
    );
    m.wait_for_heartbeat(1_500);
    child.kill().unwrap();
    child.wait().unwrap();

    // The resumed call delivers nothing for 300 ms, 800 ms after its recorder started.
    let mut child = m.record(
        &[
            "mic=synthetic:tone:440",
            "call=synthetic:tap,sound-at=0,skip-at=800,skip-ms=300",
        ],
        &["--resume", "--segment-ms", "2000"],
    );
    let (mut client, token) = m.connect(2);
    let resumed_at = t(of_kind(&m.lines_so_far(), "session")
        .into_iter()
        .find(|l| l["session"] == 2)
        .unwrap());
    m.wait_for_heartbeat(resumed_at + 5_000);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    assert_eq!(of_kind(&lines, "crash").len(), 1);
    let call: Vec<Seg> = segments(&m, &lines, "call")
        .into_iter()
        .filter(|s| s.t >= resumed_at)
        .collect();
    assert!(call.len() >= 3, "{} call segments", call.len());
    let (first, second) = (&call[0], &call[1]);
    assert_eq!(first.pcm.len(), 32_000, "the first segment is full");
    let ended = lines
        .iter()
        .find(|l| l["kind"] == "segment_end" && l["track"] == "call" && l["seq"] == first.seq)
        .unwrap();
    assert_eq!(t(ended), first.t + 2_000);
    let jump = second.t - first.end();
    assert!(
        (200..=400).contains(&jump),
        "segment {} ends at {} and {} starts at {}",
        first.seq,
        first.end(),
        second.seq,
        second.t
    );
    // Later boundaries share the millisecond again.
    for pair in call[1..].windows(2) {
        assert!(
            pair[1].t.abs_diff(pair[0].end()) <= 30,
            "{} {}",
            pair[0].end(),
            pair[1].t
        );
    }
    // The microphone had no shortfall, so its boundaries stay together.
    let mic: Vec<Seg> = segments(&m, &lines, "mic")
        .into_iter()
        .filter(|s| s.t >= resumed_at)
        .collect();
    for pair in mic.windows(2) {
        assert!(
            pair[1].t.abs_diff(pair[0].end()) <= 30,
            "{} {}",
            pair[0].end(),
            pair[1].t
        );
    }
}
