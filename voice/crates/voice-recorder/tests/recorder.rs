//! Behaviour of the recorder binary beyond the two crash tests: finalize, resume numbering,
//! pause and marks, the tap and the connections it holds, dictation stretches, control
//! authentication, the file source, call-track padding, a slow device open, slow disk syncs, the
//! length limit and refused starts. All with synthetic or file sources.

mod common;

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use common::*;
use serde_json::json;
use voice_recorder::control::Client;
use voice_recorder::exit;

/// Times the recorder reports may trail the moment a test acted by a tick or two, more on CI.
const SLACK_MS: u64 = 300;
/// A loopback track is padded once it trails the clock by 0.5 s.
const PAD_THRESHOLD_MS: u64 = 500;

#[test]
fn finalize_repairs_headers_and_is_idempotent() {
    let m = TempMeeting::new("finalize");
    let mut child = m.record(&["mic=synthetic:tone:440", "call=synthetic:idle"], &[]);
    m.wait_for_heartbeat(1_500);
    child.kill().unwrap();
    child.wait().unwrap();

    // What a kill between writing samples and patching the sizes leaves: samples the header
    // doesn't count yet, and half a sample.
    let lines = m.lines();
    let mic = segments(&m, &lines, "mic");
    let path = m.segment_path(&mic.last().unwrap().file);
    let counted = mic.last().unwrap().pcm.len();
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    for _ in 0..100 {
        file.write_all(&1000i16.to_le_bytes()).unwrap();
    }
    file.write_all(&[7]).unwrap();
    drop(file);
    // And a timeline line the kill cut in half.
    let mut timeline = OpenOptions::new()
        .append(true)
        .open(m.dir.join("timeline.jsonl"))
        .unwrap();
    timeline
        .write_all(br#"{"kind":"heartbeat","t":99"#)
        .unwrap();
    drop(timeline);
    assert!(
        m.recorder_file().is_some(),
        "a killed recorder leaves recorder.json"
    );

    let dir = m.arg();
    let (status, out) = m.run(&["finalize", "--meeting", &dir]);
    assert!(status.success(), "{out}");
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["ended"], 2, "{report}");
    assert_eq!(report["stopped"], true);

    let text = m.timeline_text();
    assert!(text.ends_with('\n'), "the partial line is gone");
    let lines = m.lines();
    let stop = lines.last().unwrap();
    assert_eq!(stop["kind"], "stop");
    assert_eq!(stop["reason"], "interrupted");
    assert_eq!(report["t"], stop["t"]);

    let mic = segments(&m, &lines, "mic");
    let repaired = mic.last().unwrap();
    assert_eq!(
        repaired.pcm.len(),
        counted + 100,
        "the uncounted samples are recovered"
    );
    assert!(repaired.pcm[counted..].iter().all(|&s| s == 1000));
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        44 + 2 * (counted as u64 + 100)
    );
    let end = lines
        .iter()
        .find(|l| l["kind"] == "segment_end" && l["track"] == "mic" && l["seq"] == repaired.seq)
        .unwrap();
    assert_eq!(end["samples"], (counted + 100) as u64);
    assert_eq!(t(end), repaired.end());
    assert!(
        t(stop) >= repaired.end(),
        "stop at {} before the audio's end",
        t(stop)
    );
    assert!(
        m.recorder_file().is_none(),
        "finalize removes the stale recorder.json"
    );

    // Again: nothing to do, nothing written.
    let before = m.timeline_text();
    let (status, out) = m.run(&["finalize", "--meeting", &dir]);
    assert!(status.success(), "{out}");
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["ended"], 0);
    assert_eq!(report["stopped"], false);
    assert_eq!(m.timeline_text(), before);
}

#[test]
fn resume_continues_numbering_and_stays_paused() {
    let m = TempMeeting::new("resume-paused");
    let tracks = ["mic=synthetic:tone:440", "call=synthetic:tone:660"];
    let mut child = m.record(&tracks, &[]);
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(1_000);
    let paused = command(&mut client, &token, json!({ "cmd": "pause" }));
    assert_eq!(paused["ok"], true, "{paused}");
    let mark = command(
        &mut client,
        &token,
        json!({ "cmd": "mark", "label": "Before" }),
    );
    assert_eq!(mark["id"], "k-1");
    m.wait_for_heartbeat(t(&paused) + 1_000);
    child.kill().unwrap();
    child.wait().unwrap();

    // A file the dead session created but never named in the timeline.
    let orphan = m.dir.join("audio").join("mic").join("000002.wav");
    fs::write(&orphan, voice_audio::segment_header(0)).unwrap();

    let mut child = m.record(&tracks, &["--resume"]);
    let (mut client, token) = m.connect(2);
    let status = command(&mut client, &token, json!({ "cmd": "status" }));
    assert_eq!(
        status["state"], "paused",
        "a restart must not record a paused user"
    );
    let lines = m.lines_so_far();
    let session_at = lines
        .iter()
        .position(|l| l["kind"] == "session" && l["session"] == 2)
        .unwrap();
    let kinds: Vec<&str> = lines[session_at..]
        .iter()
        .map(|l| l["kind"].as_str().unwrap())
        .filter(|k| *k != "track" && *k != "heartbeat")
        .collect();
    assert_eq!(kinds, vec!["session", "crash", "pause"]);
    let resumed_at = t(&lines[session_at]);
    m.wait_for_heartbeat(resumed_at + 1_000);
    assert!(
        !m.lines_so_far()[session_at..]
            .iter()
            .any(|l| l["kind"] == "segment"),
        "audio was recorded while paused"
    );

    let resume = command(&mut client, &token, json!({ "cmd": "resume" }));
    assert_eq!(resume["ok"], true, "{resume}");
    let mark = command(&mut client, &token, json!({ "cmd": "mark" }));
    assert_eq!(mark["id"], "k-2", "mark ids stay unique across sessions");
    m.wait_for_heartbeat(t(&resume) + 1_000);
    let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true);
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let mic = segments(&m, &lines, "mic");
    let seqs: Vec<u64> = mic.iter().map(|s| s.seq).collect();
    assert_eq!(seqs, vec![1, 3], "numbering skips the orphan file");
    assert_eq!(mic[1].file, "audio/mic/000003.wav");
    assert!(mic[1].t >= t(&resume));
    assert_eq!(fs::read(&orphan).unwrap(), voice_audio::segment_header(0));
    let call: Vec<u64> = segments(&m, &lines, "call").iter().map(|s| s.seq).collect();
    assert_eq!(call, vec![1, 2]);
}

#[test]
fn pause_resume_mark_and_dictation_lines() {
    if skip_on_slow_host("pause_resume_mark_and_dictation_lines") {
        return;
    }
    let m = TempMeeting::new("pause");
    let mut child = m.record(&["mic=synthetic:tone:440", "call=synthetic:idle"], &[]);
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(1_000);

    let mark = command(
        &mut client,
        &token,
        json!({ "cmd": "mark", "label": "Decision" }),
    );
    assert_eq!(
        (mark["ok"].clone(), mark["id"].clone()),
        (json!(true), json!("k-1"))
    );
    let start = command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "start" }),
    );
    let end = command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "end" }),
    );
    assert_eq!(
        (start["ok"].clone(), end["ok"].clone()),
        (json!(true), json!(true))
    );
    let pause = command(&mut client, &token, json!({ "cmd": "pause" }));
    let p = t(&pause);
    let again = command(&mut client, &token, json!({ "cmd": "pause" }));
    assert_eq!(again, json!({ "ok": false, "error": "not_recording" }));
    let status = command(&mut client, &token, json!({ "cmd": "status" }));
    assert_eq!(status["state"], "paused");
    let paused_mark = command(&mut client, &token, json!({ "cmd": "mark" }));
    assert_eq!(paused_mark["id"], "k-2");
    m.wait_for_heartbeat(p + 1_000);
    let resume = command(&mut client, &token, json!({ "cmd": "resume" }));
    let r = t(&resume);
    let again = command(&mut client, &token, json!({ "cmd": "resume" }));
    assert_eq!(again, json!({ "ok": false, "error": "not_recording" }));
    m.wait_for_heartbeat(r + 1_000);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let marks = of_kind(&lines, "mark");
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0]["label"], "Decision");
    assert_eq!(t(marks[0]), t(&mark));
    assert_eq!(marks[1]["label"], serde_json::Value::Null);
    let dictation: Vec<_> = of_kind(&lines, "dictation")
        .iter()
        .map(|l| (l["state"].as_str().unwrap(), t(l)))
        .collect();
    assert_eq!(dictation, vec![("start", t(&start)), ("end", t(&end))]);
    assert_eq!(
        of_kind(&lines, "pause")
            .iter()
            .map(|l| t(l))
            .collect::<Vec<_>>(),
        vec![p]
    );
    assert_eq!(
        of_kind(&lines, "resume")
            .iter()
            .map(|l| t(l))
            .collect::<Vec<_>>(),
        vec![r]
    );

    // Both segments end by the pause, and nothing covers the pause. By `t`: a `segment_end` line
    // comes once its segment is on disk, which can be after the pause line.
    for track in ["mic", "call"] {
        let segs = segments(&m, &lines, track);
        assert_eq!(segs.len(), 2, "{track}: {segs:?}");
        assert!(segs[0].end() <= p, "{track} records past the pause");
        assert!(segs[1].t >= r, "{track} records before the resume");
        let ended = of_kind(&lines, "segment_end")
            .into_iter()
            .find(|l| l["track"] == track && l["seq"] == segs[0].seq)
            .map(t);
        assert!(
            ended.is_some_and(|end| end <= p),
            "{track} segment ends at {ended:?}, the pause at {p}"
        );
    }
    // The idle loopback is padded right up to the pause and again from the resume.
    let call = segments(&m, &lines, "call");
    assert!(
        call[0].end() + 20 >= p,
        "call padded to {} of {p}",
        call[0].end()
    );
    assert_eq!(call[1].t, r);
    assert!(call.iter().all(|s| s.pcm.iter().all(|&v| v == 0)));
}

#[test]
fn tap_streams_the_tone() {
    let m = TempMeeting::new("tap");
    let mut child = m.record(&["mic=synthetic:tone:440"], &[]);
    let (mut client, token) = m.connect(1);
    let port = m.recorder_file().unwrap().port;

    let mut tap = Client::connect(port, Duration::from_secs(2)).unwrap();
    let reply = command(&mut tap, &token, json!({ "cmd": "tap", "track": "mic" }));
    assert_eq!(reply, json!({ "ok": true, "rate": 16000 }));
    let mut pcm = tap.into_pcm();
    let mut bytes = vec![0u8; 16_000 * 2];
    pcm.read_exact(&mut bytes).unwrap();
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    // The tap may begin mid-block; the tone itself must be intact.
    assert_tone(&samples, 440.0, "tap");

    let mut other = Client::connect(port, Duration::from_secs(2)).unwrap();
    let reply = command(&mut other, &token, json!({ "cmd": "tap", "track": "call" }));
    assert_eq!(reply, json!({ "ok": false, "error": "unknown_track" }));

    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());
    // The tap ends with the recording.
    let mut rest = Vec::new();
    let _ = pcm.read_to_end(&mut rest);
}

#[test]
fn control_refuses_a_bad_token() {
    let m = TempMeeting::new("auth");
    let mut child = m.record(&["mic=synthetic:tone:440"], &[]);
    let (mut client, token) = m.connect(1);
    let port = m.recorder_file().unwrap().port;

    for request in [
        json!({ "token": "wrong", "cmd": "stop" }),
        json!({ "cmd": "stop" }),
        json!({ "token": format!("{token}x"), "cmd": "tap", "track": "mic" }),
    ] {
        let mut intruder = Client::connect(port, Duration::from_secs(2)).unwrap();
        let reply = intruder.request(&request).unwrap();
        assert_eq!(reply, json!({ "ok": false, "error": "bad_token" }));
        assert!(
            intruder
                .request(&json!({ "token": token, "cmd": "status" }))
                .is_err(),
            "the connection stays open after a bad token"
        );
    }
    let mut raw = TcpStream::connect(("127.0.0.1", port)).unwrap();
    raw.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    raw.write_all(b"not json\n").unwrap();
    let mut reply = String::new();
    raw.read_to_string(&mut reply).unwrap();
    assert_eq!(reply, "{\"error\":\"bad_request\",\"ok\":false}\n");

    let unknown = command(&mut client, &token, json!({ "cmd": "explode" }));
    assert_eq!(unknown, json!({ "ok": false, "error": "unknown_command" }));
    let status = command(&mut client, &token, json!({ "cmd": "status" }));
    assert_eq!(status["state"], "recording");
    assert_eq!(status["tracks"]["mic"]["ok"], true);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());
    let lines = m.lines();
    assert_eq!(of_kind(&lines, "stop").len(), 1);
}

/// The control port serves 16 connections at a time.
const CONNECTIONS: usize = 16;

/// Opens a tap the way the app's dictation does and returns the reply and the connection.
fn open_tap(port: u16, token: &str, track: &str) -> (serde_json::Value, Client) {
    let mut tap = Client::connect(port, Duration::from_secs(2)).unwrap();
    let reply = command(&mut tap, token, json!({ "cmd": "tap", "track": track }));
    (reply, tap)
}

/// Closes a connection as the app's tap does on drop: both directions, then the socket.
fn close(client: Client) {
    let pcm = client.into_pcm();
    let _ = pcm.get_ref().shutdown(Shutdown::Both);
}

/// Waits until the recorder answers `status` again on a new connection.
fn wait_for_a_free_connection(port: u16, token: &str) {
    wait_until(WAIT_TIMEOUT, || {
        let mut client = Client::connect(port, Duration::from_secs(2)).ok()?;
        client
            .request(&json!({ "token": token, "cmd": "status" }))
            .ok()
            .filter(|reply| reply["ok"] == true)
    });
}

/// Whether the server closed `stream` within `timeout`.
fn closed_within(stream: &mut TcpStream, timeout: Duration) -> bool {
    stream.set_read_timeout(Some(timeout)).unwrap();
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => !matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
    }
}

fn dictation_lines(lines: &[serde_json::Value]) -> Vec<(String, u64)> {
    of_kind(lines, "dictation")
        .iter()
        .map(|l| (l["state"].as_str().unwrap().to_string(), t(l)))
        .collect()
}

#[test]
fn taps_on_a_lost_microphone_never_lock_out_stop() {
    let m = TempMeeting::new("tap-lost-mic");
    let mut child = m.record(
        &[
            "mic=synthetic:tone:440,fail-at=0",
            "call=synthetic:tone:660",
        ],
        &[],
    );
    // The app's own connection stays open throughout.
    let (_client, token) = m.connect(1);
    let port = m.recorder_file().unwrap().port;
    m.wait_for_heartbeat(1_000);

    // The headset is unplugged and the user presses the dictation key again and again.
    for n in 0..CONNECTIONS {
        let (reply, tap) = open_tap(port, &token, "mic");
        assert_eq!(
            reply,
            json!({ "ok": false, "error": "no_device" }),
            "tap {n}"
        );
        close(tap);
    }
    // Every slot is free again: a new connection gets an answer and the meeting stops.
    let mut fresh = Client::connect(port, Duration::from_secs(2)).unwrap();
    let status = command(&mut fresh, &token, json!({ "cmd": "status" }));
    assert_eq!(status["tracks"]["mic"]["ok"], false, "{status}");
    let reply = command(&mut fresh, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(wait_exit(&mut child).success());
    assert_eq!(m.lines().last().unwrap()["kind"], "stop");
}

#[test]
fn a_tap_ends_when_its_track_loses_the_device() {
    let m = TempMeeting::new("tap-then-lost");
    let mut child = m.record(&["mic=synthetic:tone:440,fail-at=1500"], &[]);
    let (mut client, token) = m.connect(1);
    let port = m.recorder_file().unwrap().port;
    let (reply, tap) = open_tap(port, &token, "mic");
    assert_eq!(reply["ok"], true, "{reply}");
    // The tap carries audio until the device goes, then the recorder closes it.
    let mut pcm = tap.into_pcm();
    pcm.get_ref()
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut rest = Vec::new();
    let read = pcm.read_to_end(&mut rest);
    assert!(read.is_ok(), "the tap ended with {read:?}");
    assert!(
        rest.len() >= 16_000,
        "only {} bytes before the loss",
        rest.len()
    );
    let status = command(&mut client, &token, json!({ "cmd": "status" }));
    assert_eq!(status["tracks"]["mic"]["ok"], false, "{status}");
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());
}

#[test]
fn closed_taps_on_a_silent_track_free_their_connections() {
    let m = TempMeeting::new("tap-silent");
    let mut child = m.record(&["mic=synthetic:tone:440", "call=synthetic:idle"], &[]);
    let (mut client, token) = m.connect(1);
    let port = m.recorder_file().unwrap().port;
    // Nothing plays, so these taps never get a byte to write; only their clients closing ends
    // them. With the app's connection they fill every slot, twice over.
    for n in 0..2 * (CONNECTIONS - 1) {
        let (reply, tap) = wait_until(WAIT_TIMEOUT, || {
            let mut tap = Client::connect(port, Duration::from_secs(2)).ok()?;
            let reply = tap
                .request(&json!({ "token": token, "cmd": "tap", "track": "call" }))
                .ok()?;
            Some((reply, tap))
        });
        assert_eq!(reply["ok"], true, "tap {n}: {reply}");
        close(tap);
    }
    wait_for_a_free_connection(port, &token);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());
}

#[test]
fn connections_without_the_token_are_closed_after_five_seconds() {
    let m = TempMeeting::new("auth-timeout");
    let mut child = m.record(&["mic=synthetic:tone:440"], &[]);
    let (mut client, token) = m.connect(1);
    let port = m.recorder_file().unwrap().port;
    drop(client);
    // A local program fills every slot and never says anything.
    let mut silent: Vec<TcpStream> = (0..CONNECTIONS)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    for (n, stream) in silent.iter_mut().enumerate() {
        assert!(
            closed_within(stream, Duration::from_secs(15)),
            "silent connection {n} stayed open"
        );
    }
    wait_for_a_free_connection(port, &token);
    client = Client::connect(port, Duration::from_secs(2)).unwrap();
    let status = command(&mut client, &token, json!({ "cmd": "status" }));
    assert_eq!(status["ok"], true);
    // A client that authenticated may keep its connection idle.
    std::thread::sleep(Duration::from_secs(6));
    let status = command(&mut client, &token, json!({ "cmd": "status" }));
    assert_eq!(status["ok"], true);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());
}

#[test]
fn a_dictation_ends_when_its_tap_closes() {
    let m = TempMeeting::new("dictation-tap");
    let mut child = m.record(&["mic=synthetic:tone:440", "call=synthetic:idle"], &[]);
    let (mut client, token) = m.connect(1);
    let port = m.recorder_file().unwrap().port;
    m.wait_for_heartbeat(500);

    // The app opens the tap, then says it dictates, then crashes: no `end` ever comes.
    let (reply, tap) = open_tap(port, &token, "mic");
    assert_eq!(reply["ok"], true, "{reply}");
    let start = command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "start" }),
    );
    std::thread::sleep(Duration::from_millis(500));
    close(tap);
    let gone = t(&command(&mut client, &token, json!({ "cmd": "status" })));
    let lines = wait_until(WAIT_TIMEOUT, || {
        let lines = m.lines_so_far();
        (dictation_lines(&lines).len() == 2).then_some(lines)
    });
    let end = dictation_lines(&lines)[1].1;
    // The recorder notices a closed tap within its 250 ms peer check and the next 100 ms tick; an
    // idle Mac measures 10 to 25 ms. With no `end` from the app, a dictation that doesn't end on the
    // tap's close would wait for the stop and fail the wait above, so the upper bound only has to
    // say "about when the tap closed". It was 1 s, and two of five hosted Windows runs of the port
    // answered 1.1 s late ("ended at 2759, the tap closed at 1573"), at the start of the binary,
    // when the first tests start their recorders side by side. The code on that path hasn't changed.
    assert!(
        end + SLACK_MS >= gone && end <= gone + 3_000,
        "the dictation ended at {end}, the tap closed at {gone}"
    );
    // The app's own `end` after that changes nothing.
    let late = command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "end" }),
    );
    assert_eq!(late["ok"], true);

    // The usual order: close the tap, then `end`. One `end` either way.
    let (_, tap) = open_tap(port, &token, "mic");
    let again = command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "start" }),
    );
    std::thread::sleep(Duration::from_millis(300));
    close(tap);
    command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "end" }),
    );
    m.wait_for_heartbeat(t(&again) + 1_500);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let states: Vec<String> = dictation_lines(&lines).into_iter().map(|d| d.0).collect();
    assert_eq!(states, ["start", "end", "start", "end"]);
    assert_eq!(dictation_lines(&lines)[0].1, t(&start));
    assert_eq!(dictation_lines(&lines)[2].1, t(&again));
}

#[test]
fn a_dead_sessions_dictation_ends_where_it_was_last_seen() {
    let m = TempMeeting::new("dictation-crash");
    let tracks = ["mic=synthetic:tone:440"];
    let mut child = m.record(&tracks, &[]);
    let (mut client, token) = m.connect(1);
    let start = command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "start" }),
    );
    m.wait_for_heartbeat(t(&start) + 1_000);
    child.kill().unwrap();
    child.wait().unwrap();

    // The watchdog's restart closes it before its session line.
    let mut child = m.record(&tracks, &["--resume"]);
    let (mut client, token) = m.connect(2);
    let lines = m.lines_so_far();
    let session = lines
        .iter()
        .position(|l| l["kind"] == "session" && l["session"] == 2)
        .unwrap();
    let crash = of_kind(&lines, "crash")[0];
    let ended = &lines[session - 1];
    assert_eq!(
        (ended["kind"].as_str(), ended["state"].as_str()),
        (Some("dictation"), Some("end")),
        "{ended}"
    );
    assert_eq!(ended["t"], crash["lastSeen"]);

    // And `finalize` closes one before its stop line.
    let start = command(
        &mut client,
        &token,
        json!({ "cmd": "dictation", "state": "start" }),
    );
    m.wait_for_heartbeat(t(&start) + 1_000);
    child.kill().unwrap();
    child.wait().unwrap();
    let dir = m.arg();
    let (status, out) = m.run(&["finalize", "--meeting", &dir]);
    assert!(status.success(), "{out}");
    let lines = m.lines();
    let stop = &lines[lines.len() - 1];
    let ended = &lines[lines.len() - 2];
    assert_eq!(stop["kind"], "stop");
    assert_eq!(
        (ended["kind"].as_str(), ended["state"].as_str()),
        (Some("dictation"), Some("end")),
        "{ended}"
    );
    assert_eq!(ended["t"], stop["t"]);
    let states: Vec<String> = dictation_lines(&lines).into_iter().map(|d| d.0).collect();
    assert_eq!(states, ["start", "end", "start", "end"]);
}

#[test]
fn a_slow_device_open_is_not_a_sleep() {
    let m = TempMeeting::new("slow-open");
    // Opening the microphone blocks the recorder thread for 5 s, as a Bluetooth headset
    // switching to Hands-Free can. A wall-clock check took that for a sleep, dropped both
    // tracks and reopened the slow device again and again.
    let mut child = m.record(
        &[
            "mic=synthetic:tone:440,open-ms=5000",
            "call=synthetic:tone:660",
        ],
        &[],
    );
    // The device opens on a thread of its own, so the recorder answers meanwhile.
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(8_000);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    assert!(
        of_kind(&lines, "suspend").is_empty(),
        "{:?}",
        of_kind(&lines, "suspend")
    );
    assert_eq!(of_kind(&lines, "track").len(), 2, "a device was reopened");
    for (track, hz) in [("mic", 440.0), ("call", 660.0)] {
        let segs = segments(&m, &lines, track);
        assert_eq!(segs.len(), 1, "{track}: {segs:?}");
        assert_tone(&segs[0].pcm, hz, track);
    }
}

fn write_tone(path: &std::path::Path, rate: u32, hz: f64, seconds: f64) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for n in 0..(rate as f64 * seconds) as u64 {
        let v = 0.5 * (std::f64::consts::TAU * hz * n as f64 / rate as f64).sin();
        w.write_sample((v * 32_767.0).round() as i16).unwrap();
    }
    w.finalize().unwrap();
}

#[test]
fn file_source_plays_the_file_then_silence_or_loops() {
    let once = TempMeeting::new("file-once");
    let looped = TempMeeting::new("file-loop");
    // 523 whole cycles in one second, so the loop joins without a click.
    let wav = once.root.join("tone.wav");
    write_tone(&wav, 22_050, 523.0, 1.0);
    let source = format!("mic=file:{}", wav.display());
    let looped_source = format!("{source},loop");
    let mut a = once.record(&[&source], &[]);
    let mut b = looped.record(&[&looped_source], &[]);
    let (mut client_a, token_a) = once.connect(1);
    let (mut client_b, token_b) = looped.connect(1);
    once.wait_for_heartbeat(2_500);
    looped.wait_for_heartbeat(2_500);
    command(&mut client_a, &token_a, json!({ "cmd": "stop" }));
    command(&mut client_b, &token_b, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut a).success());
    assert!(wait_exit(&mut b).success());

    let lines = once.lines();
    let track = of_kind(&lines, "track")[0];
    assert_eq!(track["device"], "File tone.wav");
    assert_eq!(track["nativeRate"], 22_050);
    assert_eq!(track["nativeChannels"], 1);
    let pcm: Vec<i16> = segments(&once, &lines, "mic")
        .into_iter()
        .flat_map(|s| s.pcm)
        .collect();
    assert!(pcm.len() >= 16_000 * 2, "only {} samples", pcm.len());
    assert_tone(&pcm[..16_000], 523.0, "file");
    assert!(
        pcm[16_000 + 160..].iter().all(|&s| s.abs() < 50),
        "silence after the file ends"
    );

    let lines = looped.lines();
    let pcm: Vec<i16> = segments(&looped, &lines, "mic")
        .into_iter()
        .flat_map(|s| s.pcm)
        .collect();
    assert_tone(&pcm[..16_000 * 2], 523.0, "looped file");
}

#[test]
fn segments_rotate_without_gaps() {
    let m = TempMeeting::new("rotate");
    let mut child = m.record(
        &["mic=synthetic:tone:440", "call=synthetic:idle"],
        &["--segment-ms", "1000"],
    );
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(3_500);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let ended = |track: &str, seq: u64| {
        of_kind(&lines, "segment_end")
            .into_iter()
            .find(|l| l["track"] == track && l["seq"] == seq)
            .map(t)
    };
    for track in ["mic", "call"] {
        let segs = segments(&m, &lines, track);
        assert!(segs.len() >= 4, "{track}: {} segments", segs.len());
        let seqs: Vec<u64> = segs.iter().map(|s| s.seq).collect();
        assert_eq!(seqs, (1..=segs.len() as u64).collect::<Vec<_>>());
        for pair in segs.windows(2) {
            assert_eq!(
                pair[0].pcm.len(),
                16_000,
                "{track} {} isn't full",
                pair[0].seq
            );
            assert_eq!(
                pair[1].t,
                pair[0].t + 1_000,
                "{track} {} moved",
                pair[1].seq
            );
            // By `t`: the line comes once the segment is on disk, after the next one's.
            assert_eq!(
                ended(track, pair[0].seq),
                Some(pair[1].t),
                "{track} {} doesn't end where the next one starts",
                pair[0].seq
            );
        }
    }
    for seg in segments(&m, &lines, "mic") {
        if seg.pcm.len() > 2_000 {
            assert_tone(&seg.pcm, 440.0, &format!("mic segment {}", seg.seq));
        }
    }
}

#[test]
fn a_new_default_device_moves_the_track() {
    if skip_on_slow_host("a_new_default_device_moves_the_track") {
        return;
    }
    let m = TempMeeting::new("default-change");
    // Every sync takes 800 ms. Closing the old segment must not hold up the new device: its
    // stream would drop the old one's queued audio for that long, and the new lines would come
    // late.
    let mut child = m.record(
        &["mic=synthetic:tone:440,switch-at=1000"],
        &["--test-sync-delay-ms", "800"],
    );
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(4_500);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    assert!(of_kind(&lines, "device_lost").is_empty());
    let moved = of_kind(&lines, "device");
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(moved[0]["reason"], "default");
    assert_eq!(moved[0]["device"], "Synthetic 440 Hz tone 2");
    let at = t(moved[0]);
    let opened = of_kind(&lines, "track");
    assert_eq!(opened.len(), 2);
    assert!(opened[1]["deviceId"].as_str().unwrap().ends_with("#2"));
    let segs = segments(&m, &lines, "mic");
    assert_eq!(segs.len(), 2, "{segs:?}");
    // The default is polled every 2 s; the old device's audio ends at the poll.
    let gone = segs[0].end();
    assert!(
        (1_000..=1_000 + 2_000 + SLACK_MS).contains(&gone),
        "the old device ends at {gone}"
    );
    assert!(gone <= at, "the old device ends at {gone}, moved at {at}");
    assert!(segs[1].t >= at);
    assert!(
        segs[1].t - gone <= SLACK_MS,
        "the old device ends at {gone}, the new one starts at {}",
        segs[1].t
    );
    for seg in &segs {
        assert_tone(&seg.pcm, 440.0, &format!("mic segment {}", seg.seq));
    }
}

#[test]
fn idle_loopback_is_padded_only_while_healthy() {
    let m = TempMeeting::new("padding");
    let mut child = m.record(&["call=synthetic:idle,fail-at=1500,back-at=3000"], &[]);
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(4_500);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let lost = t(of_kind(&lines, "device_lost")[0]);
    assert!((1_500..=1_500 + SLACK_MS).contains(&lost), "lost at {lost}");
    let back = of_kind(&lines, "device")[0];
    assert_eq!(back["reason"], "reconnected");
    let back = t(back);
    let stop = t(lines.last().unwrap());
    let call = segments(&m, &lines, "call");
    assert_eq!(call.len(), 2, "{call:?}");
    // Silence from the clock up to the failure, never after it.
    assert!(call[0].t <= SLACK_MS);
    assert!(call[0].end() <= lost, "padded past the failure");
    assert!(call[0].end() + PAD_THRESHOLD_MS + SLACK_MS >= lost);
    // From the moment the device is back, up to the stop.
    assert_eq!(call[1].t, back);
    assert!(call[1].end() + 20 >= stop);
    assert!(call.iter().all(|s| s.pcm.iter().all(|&v| v == 0)));
    // By position, not by time: a heartbeat written between the failure and its handling has a
    // later `t` than `device_lost` and still lists the segment.
    let lost_at = lines
        .iter()
        .position(|l| l["kind"] == "device_lost")
        .unwrap();
    let back_at = lines.iter().position(|l| l["kind"] == "device").unwrap();
    let heartbeat_while_lost = lines[lost_at..back_at]
        .iter()
        .find(|l| l["kind"] == "heartbeat")
        .unwrap();
    assert_eq!(heartbeat_while_lost["samples"], json!({}));
}

#[test]
fn padding_catches_up_after_a_long_stall() {
    let m = TempMeeting::new("pad-stall");
    // Moving the microphone to the new default blocks the recorder thread for 1.5 s, longer than
    // a call segment. The idle call track then trails the clock by more than a segment, and its
    // padding must still continue each segment where the previous one ended. The first open
    // takes 1.5 s too, so the switch comes after it.
    let mut child = m.record(
        &[
            "mic=synthetic:tone:440,switch-at=2500,open-ms=1500",
            "call=synthetic:idle",
        ],
        &["--segment-ms", "1000"],
    );
    let (mut client, token) = m.connect(1);
    let moved = wait_until(WAIT_TIMEOUT, || {
        of_kind(&m.lines_so_far(), "device").first().map(|l| t(l))
    });
    m.wait_for_heartbeat(moved + 2_000);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let stop = t(lines.last().unwrap());
    let call = segments(&m, &lines, "call");
    assert!(call.len() >= 4, "{call:?}");
    for pair in call.windows(2) {
        assert_eq!(pair[0].pcm.len(), 16_000, "call {} isn't full", pair[0].seq);
        assert_eq!(
            pair[1].t,
            pair[0].end(),
            "call {} doesn't continue {}",
            pair[1].seq,
            pair[0].seq
        );
    }
    // The call track is padded up to the moment the stop arrived, and the stop line is that moment
    // or the latest track end. A microphone whose thread woke late delivers its backlog in one
    // burst, so its track runs ahead of the clock by that delay and can end past the stop's
    // arrival; the stop line then follows the microphone and the call ends that much before it.
    let call_end = call.last().unwrap().end();
    let mic_end = segments(&m, &lines, "mic").last().unwrap().end();
    let allowed = if mic_end + 1 >= stop { SLACK_MS } else { 20 };
    assert!(
        call_end + allowed >= stop,
        "call ends at {call_end}, mic at {mic_end}, stop at {stop}"
    );
    assert!(call.iter().all(|s| s.pcm.iter().all(|&v| v == 0)));
}

#[test]
fn slow_syncs_at_the_start_are_not_recording_time() {
    let m = TempMeeting::new("slow-start");
    // The timeline and recorder.json are synced before the tracks open; at 400 ms a sync, those
    // syncs used to count as recording time before the first segment of every track.
    let mut child = m.record(
        &["mic=synthetic:tone:440", "call=synthetic:idle"],
        &["--test-sync-delay-ms", "400"],
    );
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(1_500);
    command(&mut client, &token, json!({ "cmd": "stop" }));
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    for opened in of_kind(&lines, "track") {
        assert!(t(opened) <= SLACK_MS, "{opened}");
    }
    for track in ["mic", "call"] {
        let first = &segments(&m, &lines, track)[0];
        assert!(first.t <= SLACK_MS, "{track} starts at {}", first.t);
    }
}

#[test]
fn a_stop_during_a_slow_sync_ends_where_it_arrived() {
    let m = TempMeeting::new("slow-stop");
    let mut child = m.record(
        &["mic=synthetic:tone:440", "call=synthetic:idle"],
        &["--test-sync-delay-ms", "1000"],
    );
    let (mut client, token) = m.connect(1);
    // The fifth heartbeat comes with the first periodic sync, three syncs of 1 s each here. The
    // stop is sent while it runs.
    let fifth = wait_until(WAIT_TIMEOUT, || {
        of_kind(&m.lines_so_far(), "heartbeat").get(4).map(|l| t(l))
    });
    let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let stop = lines.last().unwrap();
    assert_eq!(stop["kind"], "stop");
    assert_eq!(reply["t"], stop["t"]);
    let stop = t(stop);
    // The reply waits for the disk; the stop itself arrived right after the heartbeat.
    assert!(
        stop <= fifth + SLACK_MS,
        "stopped at {stop}, the heartbeat before it at {fifth}"
    );
    for line in &lines {
        assert!(t(line) <= stop, "{line} after the stop at {stop}");
    }
    // Every track reaches the stop: no tail of audio the timeline doesn't explain.
    for track in ["mic", "call"] {
        let end = segments(&m, &lines, track).last().unwrap().end();
        assert!(
            end + SLACK_MS >= stop,
            "{track} ends at {end}, stop at {stop}"
        );
    }
}

#[test]
fn a_slow_disk_never_holds_up_the_heartbeats() {
    let m = TempMeeting::new("slow-disk");
    // Every sync takes 1.5 s, longer than a heartbeat period: the segments that close every 3 s,
    // the periodic sync and the timeline. None of it may hold up the recording.
    let mut child = m.record(
        &["mic=synthetic:tone:440", "call=synthetic:idle"],
        &["--segment-ms", "3000", "--test-sync-delay-ms", "1500"],
    );
    let (mut client, token) = m.connect(1);
    m.wait_for_heartbeat(7_000);
    let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(wait_exit(&mut child).success());

    let lines = m.lines();
    let heartbeats: Vec<u64> = of_kind(&lines, "heartbeat").iter().map(|l| t(l)).collect();
    assert!(heartbeats.len() >= 6, "{heartbeats:?}");
    for pair in heartbeats.windows(2) {
        assert!(
            pair[1] - pair[0] <= 1_300,
            "heartbeats {} and {}",
            pair[0],
            pair[1]
        );
    }
    // The stop replied once every file was on disk; the segments are whole and follow each
    // other.
    assert_eq!(lines.last().unwrap()["kind"], "stop");
    for track in ["mic", "call"] {
        let segs = segments(&m, &lines, track);
        assert!(segs.len() >= 3, "{track}: {segs:?}");
        for pair in segs.windows(2) {
            assert_eq!(
                pair[0].pcm.len(),
                48_000,
                "{track} {} isn't full",
                pair[0].seq
            );
            assert_eq!(pair[1].t, pair[0].end(), "{track} {} moved", pair[1].seq);
        }
    }
    for seg in segments(&m, &lines, "mic") {
        assert_tone(&seg.pcm, 440.0, &format!("mic segment {}", seg.seq));
    }
    // The slow syncs are logged, so a stalled recording can be told from a slow disk.
    let log = fs::read_to_string(m.root.join("stderr-1.log")).unwrap();
    assert!(log.contains("took"), "{log}");
}

/// A recorder that a failing test kills, instead of leaving it to record for 8 hours.
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn a_segment_end_reaches_the_timeline_only_after_its_audio_is_on_disk() {
    let m = TempMeeting::new("end-after-sync");
    // Every sync takes 1.5 s and a segment closes every 0.3 s. The session's first sync of the
    // timeline is still running when the first segment closes, and the closed segments queue up
    // behind it. A `segment_end` written at the close would go to disk with that sync of the
    // timeline, ahead of its audio: after a power cut the timeline would say the segment is
    // complete, and a resume would trust it instead of repairing the segment.
    let mut child = Running(m.record(
        &["mic=synthetic:tone:440"],
        &["--segment-ms", "300", "--test-sync-delay-ms", "1500"],
    ));
    // The syncer logs every sync this slow once it is done, before it hands the `segment_end`
    // back to the recorder thread. So when a `segment_end` shows up in the timeline, a read of
    // the log after that read of the timeline must already have its segment synced.
    let log_path = m.root.join("stderr-1.log");
    let read_log = || fs::read_to_string(&log_path).unwrap_or_default();
    let synced = |log: &str, seq: u64| log.contains(&format!("syncing mic segment {seq} took"));
    let (client, token) = m.connect(1);
    let mut client = Some(client);
    let mut stopping = None;
    let mut checked = Vec::new();
    let mut overlapped = false;
    let deadline = std::time::Instant::now() + WAIT_TIMEOUT;
    while child.0.try_wait().unwrap().is_none() {
        let lines = m.lines_so_far();
        let log = read_log();
        for end in of_kind(&lines, "segment_end") {
            let seq = end["seq"].as_u64().unwrap();
            if !checked.contains(&seq) {
                assert!(
                    synced(&log, seq),
                    "mic segment {seq} ended in the timeline before it was on disk:\n{log}"
                );
                checked.push(seq);
            }
        }
        let started = of_kind(&lines, "segment").len();
        // Segment 2 starts where segment 1 closed.
        if !overlapped && started >= 2 {
            assert!(
                !log.contains("syncing the timeline took"),
                "segment 1 closed after the first sync of the timeline:\n{log}"
            );
            overlapped = true;
        }
        // With a few segments waiting for the disk, stop: the stop waits for them too.
        if started >= 3 {
            if let Some(mut client) = client.take() {
                let token = token.clone();
                stopping = Some(std::thread::spawn(move || {
                    command(&mut client, &token, json!({ "cmd": "stop" }))
                }));
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the recorder never stopped"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let reply = stopping.expect("the stop was sent").join().unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(child.0.wait().unwrap().success());

    let log = read_log();
    // The segments waited behind that sync of the timeline.
    let first_timeline = log.find("syncing the timeline took").unwrap();
    let first_segment = log.find("syncing mic segment 1 took").unwrap();
    assert!(first_timeline < first_segment, "{log}");
    let lines = m.lines();
    let stop = lines.last().unwrap();
    assert_eq!(stop["kind"], "stop");
    assert_eq!(reply["t"], stop["t"]);
    let segs = segments(&m, &lines, "mic");
    assert!(segs.len() >= 3, "{segs:?}");
    // Every segment ended once, with its samples, and the stop came after all of them; each was
    // checked against the log when it showed up.
    for seg in &segs {
        let ends: Vec<_> = of_kind(&lines, "segment_end")
            .into_iter()
            .filter(|l| l["seq"] == seg.seq)
            .collect();
        assert_eq!(ends.len(), 1, "mic segment {}: {ends:?}", seg.seq);
        assert_eq!(ends[0]["samples"], seg.pcm.len() as u64);
        assert_eq!(t(ends[0]), seg.end());
        assert!(
            checked.contains(&seg.seq),
            "mic segment {} unchecked",
            seg.seq
        );
    }
    for pair in segs.windows(2) {
        assert_eq!(pair[0].pcm.len(), 4_800, "mic {} isn't full", pair[0].seq);
        assert_eq!(pair[1].t, pair[0].end(), "mic {} moved", pair[1].seq);
    }
}

#[test]
fn resume_ends_segments_still_waiting_for_the_disk() {
    let m = TempMeeting::new("kill-waiting");
    // Every sync takes 1.5 s and a segment closes every 0.3 s, so the recorder dies with closed
    // segments that have no `segment_end` yet, next to the open one.
    let tracks = ["mic=synthetic:tone:440"];
    let mut child = Running(m.record(
        &tracks,
        &["--segment-ms", "300", "--test-sync-delay-ms", "1500"],
    ));
    wait_until(WAIT_TIMEOUT, || {
        (of_kind(&m.lines_so_far(), "segment").len() >= 4).then_some(())
    });
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let lines = m.lines();
    let started = of_kind(&lines, "segment").len();
    let ended = of_kind(&lines, "segment_end").len();
    assert!(started >= ended + 2, "{started} segments, {ended} ended");

    let mut child = Running(m.record(&tracks, &["--resume"]));
    let (mut client, token) = m.connect(2);
    let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(wait_exit(&mut child.0).success());

    let lines = m.lines();
    let session = lines
        .iter()
        .position(|l| l["kind"] == "session" && l["session"] == 2)
        .unwrap();
    let resumed_at = t(&lines[session]);
    let segs = segments(&m, &lines, "mic");
    let before: Vec<&Seg> = segs.iter().filter(|s| s.t < resumed_at).collect();
    assert_eq!(before.len(), started, "{segs:?}");
    // Every segment of the dead session ended once, with what its file holds. The resumed
    // session ended the ones it found without an end, before its own session line.
    for seg in &before {
        let ends: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l["kind"] == "segment_end" && l["seq"] == seg.seq)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            ends.len(),
            1,
            "mic segment {} ends {} times",
            seg.seq,
            ends.len()
        );
        assert_eq!(lines[ends[0]]["samples"], seg.pcm.len() as u64);
        assert!(
            ends[0] < session,
            "mic segment {} ended after the resume",
            seg.seq
        );
    }
    for pair in before.windows(2) {
        assert_eq!(pair[0].pcm.len(), 4_800, "mic {} isn't full", pair[0].seq);
        assert_eq!(pair[1].t, pair[0].end(), "mic {} moved", pair[1].seq);
    }
}

#[test]
fn length_limit_stops_the_recording() {
    let m = TempMeeting::new("limit");
    let mut child = m.record(&["mic=synthetic:tone:440"], &["--limit-ms", "1500"]);
    assert!(wait_exit(&mut child).success());
    let lines = m.lines();
    let stop = lines.last().unwrap();
    assert_eq!(stop["kind"], "stop");
    assert_eq!(stop["reason"], "limit");
    assert!((1_500..=1_500 + SLACK_MS).contains(&t(stop)), "{stop}");
    assert!(m.recorder_file().is_none());
}

#[test]
fn refuses_conflicting_starts() {
    let m = TempMeeting::new("conflict");
    let dir = m.arg();
    let mut child = m.record(&["mic=synthetic:tone:440"], &[]);
    let (mut client, token) = m.connect(1);
    let track = ["--track", "mic=synthetic:tone:440"];
    let mut resume = vec!["record", "--meeting", dir.as_str(), "--resume"];
    resume.extend(track);
    let (status, _) = m.run(&resume);
    assert_eq!(
        status.code(),
        Some(exit::BUSY),
        "a second recorder into a live meeting"
    );
    let (status, _) = m.run(&["finalize", "--meeting", &dir]);
    assert_eq!(
        status.code(),
        Some(exit::BUSY),
        "finalize of a live meeting"
    );
    command(
        &mut client,
        &token,
        json!({ "cmd": "stop", "reason": "rukoo" }),
    );
    assert!(wait_exit(&mut child).success());
    assert_eq!(m.lines().last().unwrap()["reason"], "rukoo");

    let (status, _) = m.run(&resume);
    assert_eq!(status.code(), Some(exit::CONFLICT), "nothing follows stop");
    let mut fresh = vec!["record", "--meeting", dir.as_str()];
    fresh.extend(track);
    let (status, _) = m.run(&fresh);
    assert_eq!(
        status.code(),
        Some(exit::CONFLICT),
        "a new recording over an old one"
    );
    let (status, _) = m.run(&["record", "--meeting", &dir, "--track", "mic=nothing"]);
    assert_eq!(status.code(), Some(exit::USAGE));
    assert_eq!(of_kind(&m.lines(), "session").len(), 1);
}
