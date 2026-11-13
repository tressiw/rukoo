//! The two crash-safety tests every recorder change must pass (voice/AGENTS.md). They run the
//! real binary with synthetic sources, so they need no audio hardware and run on Windows and
//! Linux CI alike.

mod common;

use common::*;
use serde_json::json;

/// How late a sleeping thread may wake on the slowest host these tests must pass on. Hosted macOS
/// runners measured up to 76 ms (the CI host probe); the bounds below are derived for 100 ms at
/// every wake-up on a path, so a late wake-up of the source thread, one of the recorder's
/// loop and the next tick can all stack.
const HOST_LATE_MS: u64 = 100;
/// The recorder's loop wakes every 100 ms (`TICK` in record.rs), and a synthetic source polls every
/// 5 ms and delivers 10 ms blocks (`BLOCK` in source.rs).
const TICK_MS: u64 = 100;
const SOURCE_POLL_MS: u64 = 5;
const BLOCK_MS: u64 = 10;
/// How far the audio on disk lags the clock: a block is delivered at most a poll plus a late
/// wake-up after it was due, and the resampler holds back about a millisecond. A heartbeat and a
/// stop are stamped on the clock, so the audio before them can end this far short of their time.
const AUDIO_LAG_MS: u64 = BLOCK_MS + SOURCE_POLL_MS + HOST_LATE_MS + 1;
/// The first segment of a session may start this late: the device opens, then its first
/// 10 ms block arrives. GitHub's Windows runners open the second track's source and create its
/// first file late now and then (two cores and a virus scan of every new file), so the margin is
/// generous: wake-ups account for at most `AUDIO_LAG_MS` of it. What these tests guard is that
/// nothing recorded is lost after a kill or a lost device; the tolerances below for that stay tight.
const START_TOLERANCE_MS: u64 = 1_000;
/// Neighbouring segments of one stream may sit this far apart. A segment starts where the one
/// before ended, counted in samples, so no wake-up lateness enters: in practice they are exactly
/// contiguous, and a gap here is audio the recorder lost.
const JOIN_TOLERANCE_MS: u64 = 30;
/// A hard kill may lose the last append (100 to 200 ms) and audio not yet appended; the audio that
/// reached the recorder is flushed on every tick, before a heartbeat is written. Add the lag of
/// the audio itself.
const KILL_LOSS_MS: u64 = 300;
/// A failure is detected by the source's next poll after it is due, so `device_lost` is stamped at
/// most a poll and a late wake-up after the failure; the audio due before it, but not yet
/// delivered, is lost with a late wake-up too.
const DETECT_TOLERANCE_MS: u64 = 300;
/// A lost device is retried every second.
const RETRY_MS: u64 = 1_000;
/// The meeting clock starts after the audio backend, so a fault that fires at `fail-at` on the
/// backend's clock lands this much earlier on the meeting's: a descheduled process between the
/// two statements costs at most a late wake-up.
const SKEW_MS: u64 = HOST_LATE_MS;
/// Closing the lost track's segment syncs it to disk on the recorder's thread before the retries
/// are scheduled. A hosted macOS runner's `device` line came 1,375 ms after its `back-at`, so the
/// sync and the wake-ups below took that long together. The sync is the one term that isn't a
/// wake-up; 1.5 s is that whole 1,375 ms, rounded up.
const CLOSE_SYNC_MS: u64 = 1_500;
/// How long after its `back-at` the device may come back. The recorder handles the failure a poll
/// and a late wake-up of the source after `fail-at` and a late wake-up of its own loop after that,
/// closes the segment (`CLOSE_SYNC_MS`), and schedules its first retry a second later and the
/// next one a second after that (the first one fails when the close was quick, as `back-at` is
/// 2 s after `fail-at`). The attempt that finds the device back runs on the first tick after it
/// is due: a tick and a late wake-up. Every attempt after `back-at` opens it.
const BACK_LATE_MS: u64 = SOURCE_POLL_MS + 3 * HOST_LATE_MS + TICK_MS + CLOSE_SYNC_MS;

// The tolerances above hold what their derivations say on a host that wakes 100 ms late.
const _: () = assert!(KILL_LOSS_MS >= AUDIO_LAG_MS);
const _: () = assert!(DETECT_TOLERANCE_MS >= SOURCE_POLL_MS + BLOCK_MS + HOST_LATE_MS);

fn assert_contiguous(segs: &[Seg], what: &str) {
    for pair in segs.windows(2) {
        let gap = pair[1].t.abs_diff(pair[0].end());
        assert!(
            gap <= JOIN_TOLERANCE_MS,
            "{what}: segment {} ends at {} but {} starts at {}",
            pair[0].seq,
            pair[0].end(),
            pair[1].seq,
            pair[1].t
        );
    }
}

#[test]
fn hard_kill_keeps_everything_before_it() {
    let m = TempMeeting::new("hard-kill");
    let tracks = ["mic=synthetic:tone:440", "call=synthetic:tone:660"];
    let mut child = m.record(&tracks, &[]);
    m.wait_for_heartbeat(3_000);
    // TerminateProcess on Windows, SIGKILL elsewhere: nothing runs after this.
    child.kill().unwrap();
    child.wait().unwrap();

    let lines = m.lines();
    let last_heartbeat = of_kind(&lines, "heartbeat")
        .iter()
        .map(|l| t(l))
        .max()
        .unwrap();
    assert!(last_heartbeat >= 3_000);
    for (track, hz) in [("mic", 440.0), ("call", 660.0)] {
        let segs = segments(&m, &lines, track);
        assert!(!segs.is_empty(), "{track} has no segments");
        assert!(
            segs[0].t <= START_TOLERANCE_MS,
            "{track} starts at {}",
            segs[0].t
        );
        assert_contiguous(&segs, track);
        let end = segs.last().unwrap().end();
        assert!(
            end + KILL_LOSS_MS >= last_heartbeat,
            "{track} audio ends at {end}, last heartbeat at {last_heartbeat}"
        );
        for seg in &segs {
            assert_tone(&seg.pcm, hz, &format!("{track} segment {}", seg.seq));
        }
    }

    // The watchdog restarts the recorder into the same meeting.
    let mut child = m.record(&tracks, &["--resume"]);
    let (mut client, token) = m.connect(2);
    let lines = m.lines_so_far();
    let session = of_kind(&lines, "session")
        .into_iter()
        .find(|l| l["session"] == 2)
        .cloned()
        .unwrap();
    let resumed_at = t(&session);
    // The resumed session must record 1.5 s by this heartbeat (below). Its first segment may start
    // `START_TOLERANCE_MS` after `resumed_at` and the audio on disk lags the heartbeat by
    // `AUDIO_LAG_MS`, so waiting for 1.5 s + 1 s + 0.2 s, rounded up, is enough on a slow host too.
    m.wait_for_heartbeat(resumed_at + 3_000);
    let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    assert_eq!(session["resumed"], true);
    let crash = of_kind(&lines, "crash");
    assert_eq!(crash.len(), 1);
    let crash = crash[0];
    let last_seen = crash["lastSeen"].as_u64().unwrap();
    assert_eq!(t(crash), resumed_at);
    assert!(last_seen >= last_heartbeat, "lastSeen {last_seen}");
    assert!(
        last_seen <= resumed_at,
        "lastSeen {last_seen} after {resumed_at}"
    );
    assert_eq!(lines.last().unwrap()["kind"], "stop");
    assert!(
        m.recorder_file().is_none(),
        "recorder.json stays after stop"
    );

    for (track, hz) in [("mic", 440.0), ("call", 660.0)] {
        let segs = segments(&m, &lines, track);
        // The dead session's segment got its segment_end from the resumed session.
        for seg in &segs {
            let ended = lines
                .iter()
                .any(|l| l["kind"] == "segment_end" && l["track"] == track && l["seq"] == seg.seq);
            assert!(ended, "{track} segment {} never ended", seg.seq);
        }
        let after: Vec<&Seg> = segs.iter().filter(|s| s.t >= resumed_at).collect();
        assert!(
            !after.is_empty(),
            "{track} recorded nothing after the resume"
        );
        assert!(after[0].seq > segs[0].seq, "{track} numbering restarted");
        let recorded: usize = after.iter().map(|s| s.pcm.len()).sum();
        assert!(
            recorded >= 16_000 * 3 / 2,
            "{track} has {recorded} samples after the resume"
        );
        for seg in after {
            assert_tone(
                &seg.pcm,
                hz,
                &format!("{track} segment {} after resume", seg.seq),
            );
        }
    }
}

#[test]
fn microphone_removed_keeps_the_timeline_complete() {
    let m = TempMeeting::new("mic-removed");
    let mut child = m.record(
        &[
            "mic=synthetic:tone:440,fail-at=2000,back-at=4000",
            "call=synthetic:tone:660",
        ],
        &[],
    );
    let (mut client, token) = m.connect(1);
    // The device may come back as late as `back-at` + `BACK_LATE_MS` (5,905 ms), and the audio after
    // it has to be there before the stop: 7,000 ms leaves it a second.
    m.wait_for_heartbeat(7_000);
    let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let stop = lines.last().unwrap();
    assert_eq!(stop["kind"], "stop", "the timeline ends with stop");
    assert_eq!(stop["reason"], "user");
    let stop_t = t(stop);

    let lost: Vec<_> = of_kind(&lines, "device_lost")
        .into_iter()
        .filter(|l| l["track"] == "mic")
        .collect();
    assert_eq!(lost.len(), 1, "{lost:?}");
    let lost_t = t(lost[0]);
    assert!(
        (2_000 - SKEW_MS..=2_000 + DETECT_TOLERANCE_MS).contains(&lost_t),
        "device_lost at {lost_t}"
    );
    let back: Vec<_> = of_kind(&lines, "device")
        .into_iter()
        .filter(|l| l["track"] == "mic")
        .collect();
    assert_eq!(back.len(), 1, "{back:?}");
    assert_eq!(back[0]["reason"], "reconnected");
    let back_t = t(back[0]);
    assert!(
        (4_000 - SKEW_MS..=4_000 + BACK_LATE_MS).contains(&back_t),
        "device back at {back_t}"
    );
    // The first attempt comes a retry period after the recorder handled the loss, however late the
    // host is.
    assert!(
        back_t + JOIN_TOLERANCE_MS >= lost_t + RETRY_MS,
        "device back at {back_t}, lost at {lost_t}"
    );

    let mic = segments(&m, &lines, "mic");
    let (before, after): (Vec<&Seg>, Vec<&Seg>) = mic.iter().partition(|s| s.t < lost_t);
    assert!(!before.is_empty() && !after.is_empty(), "{mic:?}");
    assert!(
        before[0].t <= START_TOLERANCE_MS,
        "mic starts at {}",
        before[0].t
    );
    for pair in before.windows(2).chain(after.windows(2)) {
        assert!(pair[1].t.abs_diff(pair[0].end()) <= JOIN_TOLERANCE_MS);
    }
    let gone = before.last().unwrap().end();
    // The recorder places a segment by the arrival of its first block, and the synthetic source
    // fails by the clock but delivers its blocks when its thread wakes. A thread that wakes late
    // at the start puts the whole segment that much later, with all its audio, so the audio ends
    // at the failure time plus that lateness: 2,034 ms for a 2,000 ms failure on a hosted Mac whose
    // sleeps wake a median 64 ms late, 1,990 to 1,994 ms on an idle one. The audio may end up to a
    // detection tolerance after the failure, as the `device_lost` line may; it may not end after
    // that line (the loss is stamped at or after the audio's end), and it may not end early.
    assert!(
        (2_000 - DETECT_TOLERANCE_MS..=2_000 + DETECT_TOLERANCE_MS).contains(&gone),
        "mic audio ends at {gone}"
    );
    assert!(
        gone <= lost_t + JOIN_TOLERANCE_MS,
        "mic audio ends at {gone}, after the device was lost at {lost_t}"
    );
    // No samples while the device was gone.
    assert!(
        after[0].t >= 4_000 - SKEW_MS,
        "mic audio resumes at {}",
        after[0].t
    );
    assert!(after[0].t >= back_t.saturating_sub(JOIN_TOLERANCE_MS));
    let mic_end = after.last().unwrap().end();
    assert!(
        mic_end + KILL_LOSS_MS >= stop_t,
        "mic ends at {mic_end}, stop at {stop_t}"
    );
    for seg in &mic {
        assert_tone(&seg.pcm, 440.0, &format!("mic segment {}", seg.seq));
    }

    let call = segments(&m, &lines, "call");
    assert!(
        call[0].t <= START_TOLERANCE_MS,
        "call starts at {}",
        call[0].t
    );
    assert_contiguous(&call, "call");
    let call_end = call.last().unwrap().end();
    assert!(
        call_end + KILL_LOSS_MS >= stop_t,
        "call ends at {call_end}, stop at {stop_t}"
    );
    for seg in &call {
        assert_tone(&seg.pcm, 660.0, &format!("call segment {}", seg.seq));
    }
}
