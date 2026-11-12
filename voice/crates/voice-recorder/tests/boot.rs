//! The `boot` field of `session` and `heartbeat` lines: a clock that counts sleep, read next to
//! `t`, so the screen process can map its own moments onto the meeting clock
//! (`voice/RECORDING.md`). On Windows that is the interrupt time, on macOS `mach_continuous_time`;
//! Linux has no such shared counter, so the recorder leaves the field out there.

mod common;

use common::*;
use serde_json::{json, Value};

/// `t` is whole milliseconds read from `Instant` and `boot` is read right after it, so two
/// lines may disagree by the two truncations plus the moment between the reads.
const BOOT_TOLERANCE_MS: f64 = 2.0;

fn boot(line: &Value) -> Option<u64> {
    line["boot"].as_u64()
}

/// The `session` line and the heartbeats after it, up to the next `session` line.
fn session_lines(lines: &[Value], session: u64) -> Vec<&Value> {
    let start = lines
        .iter()
        .position(|l| l["kind"] == "session" && l["session"] == session)
        .unwrap_or_else(|| panic!("no session {session}"));
    let mut out = vec![&lines[start]];
    out.extend(
        lines[start + 1..]
            .iter()
            .take_while(|l| l["kind"] != "session")
            .filter(|l| l["kind"] == "heartbeat"),
    );
    out
}

fn assert_boot_follows_t(anchors: &[&Value], what: &str) {
    assert!(
        anchors.len() >= 3,
        "{what}: too few heartbeats: {anchors:?}"
    );
    for pair in anchors.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let dt = t(b) as f64 - t(a) as f64;
        let dboot = (boot(b).unwrap() as f64 - boot(a).unwrap() as f64) / 10_000.0;
        assert!(
            (dt - dboot).abs() <= BOOT_TOLERANCE_MS,
            "{what}: t moved {dt} ms but boot {dboot} ms between {a} and {b}"
        );
    }
}

#[test]
fn session_and_heartbeats_carry_boot_where_the_clock_counts_sleep() {
    let m = TempMeeting::new("boot");
    let tracks = ["mic=synthetic:tone:440", "call=synthetic:tone:660"];
    let mut child = m.record(&tracks, &[]);
    m.wait_for_heartbeat(3_000);
    // A hard kill and a resume: the resumed session's lines carry it too.
    child.kill().unwrap();
    child.wait().unwrap();
    let mut child = m.record(&tracks, &["--resume"]);
    let (mut client, token) = m.connect(2);
    let resumed_at = t(of_kind(&m.lines_so_far(), "session")
        .into_iter()
        .find(|l| l["session"] == 2)
        .unwrap());
    m.wait_for_heartbeat(resumed_at + 3_000);
    let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let anchors: Vec<&Value> = lines
        .iter()
        .filter(|l| l["kind"] == "session" || l["kind"] == "heartbeat")
        .collect();
    assert!(anchors.len() >= 8, "{anchors:?}");
    if cfg!(windows) || cfg!(target_os = "macos") {
        for line in &anchors {
            assert!(boot(line).is_some(), "no boot in {line}");
        }
        // The counter keeps counting across processes, so both sessions sit on one line.
        assert_boot_follows_t(&session_lines(&lines, 1), "session 1");
        assert_boot_follows_t(&session_lines(&lines, 2), "session 2");
        let first = boot(session_lines(&lines, 1)[0]).unwrap();
        let second = boot(session_lines(&lines, 2)[0]).unwrap();
        assert!(second > first, "boot went back from {first} to {second}");
    } else {
        for line in &anchors {
            assert!(line.get("boot").is_none(), "boot in {line}");
        }
    }
}
