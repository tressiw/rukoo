//! The bridge server against a synchronous client playing Rukoo.

mod common;

use std::io::Read;
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use common::{
    fake_media, passage, Bridge, Client, TempDir, DONE_MEETING, FAKE_MEDIA, FAKE_MEDIA_SIZE,
    NEW_MEETING, RUKOO_PID,
};
use serde_json::{json, Value};
use voice_bridge::{EventLog, Options};
use voice_protocol::{validate_discovery, CLIP_WAIT, MAX_RUKOO_MESSAGE};

const A: &str = "m-20261030-171203-4f2a";
const B: &str = "m-20261030-180000-0b0b";
const OTHER_INSTANCE: &str = "AAAAAAAAAAAAAAAAAAAAAA";
/// The ping test drops a silent client after 150 + 300 ms; this leaves room for a slow machine.
const PING_DROP_WAIT: Duration = Duration::from_millis(1_200);

fn fill(log: &EventLog, meeting: &str, count: u64) {
    for i in 0..count {
        log.append("passage", Some(meeting), passage(i * 1000))
            .unwrap();
    }
}

fn seqs(client: &mut Client, count: usize) -> Vec<u64> {
    (0..count)
        .map(|_| {
            let event = client.read();
            assert_eq!(event["type"], "event", "{event}");
            event["seq"].as_u64().unwrap()
        })
        .collect()
}

/// Sends `status.get` and returns the messages that came before its reply.
fn read_until_status_reply(client: &mut Client, id: &str) -> Vec<Value> {
    client.command(id, "status.get", json!({}));
    let mut before = Vec::new();
    loop {
        let message = client.read();
        if message["type"] == "reply" && message["id"] == id {
            return before;
        }
        before.push(message);
    }
}

#[test]
fn protection_waits_for_rukoo_windows_but_not_diagnostic_clients() {
    let bridge = Bridge::start("server-protection-clients");
    let handle = bridge.handle.as_ref().unwrap();
    let (mut rukoo, _) = bridge.client(None, 0);
    assert!(read_until_status_reply(&mut rukoo, "rukoo-ready").is_empty());
    let mut probe = Client::connect(bridge.port());
    probe.send(json!({"type":"hello", "protocol":voice_protocol::PROTOCOL, "token":bridge.token(), "client":"probe", "instance":null, "after":0}));
    assert_eq!(probe.read()["type"], "welcome");
    assert!(read_until_status_reply(&mut probe, "probe-ready").is_empty());
    assert_eq!(handle.clients().connected.len(), 2);
    assert_eq!(handle.protection_clients().len(), 1);
}

#[test]
fn the_pids_are_the_welcomed_rukoo_clients_that_sent_one() {
    let bridge = Bridge::start("server-rukoo-pids");
    let handle = bridge.handle.as_ref().unwrap();
    assert!(handle.rukoo_pids().is_empty());
    let (mut rukoo, _) = bridge.client(None, 0);
    assert!(read_until_status_reply(&mut rukoo, "rukoo-ready").is_empty());
    assert_eq!(handle.rukoo_pids(), [RUKOO_PID]);

    // A client with another name, or one without a pid, isn't a Rukoo window to leave out.
    let mut probe = Client::connect(bridge.port());
    probe.send(json!({"type":"hello", "protocol":voice_protocol::PROTOCOL, "token":bridge.token(), "client":"probe", "pid": 7, "instance":null, "after":0}));
    assert_eq!(probe.read()["type"], "welcome");
    let mut older = Client::connect(bridge.port());
    older.send(json!({"type":"hello", "protocol":voice_protocol::PROTOCOL, "token":bridge.token(), "client":"rukoo", "clientVersion":"2.1.0", "instance":null, "after":0}));
    assert_eq!(older.read()["type"], "welcome");
    assert_eq!(handle.rukoo_pids(), [RUKOO_PID]);

    // A second Rukoo on the same pid is one process: it is named once.
    let (mut again, _) = bridge.client(None, 0);
    assert!(read_until_status_reply(&mut again, "rukoo-again").is_empty());
    assert_eq!(handle.rukoo_pids(), [RUKOO_PID]);

    // A client that leaves takes its pid with it.
    drop(rukoo);
    drop(again);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.rukoo_pids().is_empty() {
        assert!(Instant::now() < deadline, "the pid stayed in the list");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_discovery_file_describes_the_server_and_goes_on_shutdown() {
    let mut bridge = Bridge::start("server-discovery");
    let text = std::fs::read_to_string(&bridge.discovery_path).unwrap();
    let file = validate_discovery(&serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(file.port, bridge.handle.as_ref().unwrap().port());
    assert_eq!(file.instance, bridge.log.instance());
    assert_eq!(file.pid, std::process::id());
    assert_eq!(
        (file.protocol, file.min_protocol),
        (voice_protocol::PROTOCOL, voice_protocol::MIN_PROTOCOL)
    );
    assert_eq!(file.voice_version, "0.1.0");
    assert_eq!(
        bridge.handle.as_ref().unwrap().discovery_path(),
        bridge.discovery_path
    );

    bridge.shutdown();
    assert!(!bridge.discovery_path.exists());
    assert!(Client::try_connect(bridge.port(), "/bridge", &[]).is_err());
}

#[test]
fn every_start_makes_a_fresh_token() {
    let dir = TempDir::new("server-tokens");
    let log = EventLog::open(dir.path()).unwrap();
    let first = Bridge::start_on(
        TempDir::new("server-tokens-a"),
        log.clone(),
        Options::default(),
    );
    let second = Bridge::start_on(TempDir::new("server-tokens-b"), log, Options::default());
    assert_ne!(first.token(), second.token());
    assert_eq!(first.discovery.instance, second.discovery.instance);
}

#[test]
fn a_first_connection_gets_every_stored_event() {
    let bridge = Bridge::start("server-full");
    fill(&bridge.log, A, 3);
    let (mut client, welcome) = bridge.client(None, 0);
    assert_eq!(welcome["replay"], "full");
    assert_eq!(welcome["head"], 3);
    assert_eq!(welcome["oldest"], 1);
    assert_eq!(welcome["protocol"], voice_protocol::PROTOCOL);
    assert_eq!(welcome["voiceVersion"], "0.1.0");
    assert_eq!(welcome["instance"], bridge.log.instance());
    assert_eq!(
        welcome["status"],
        json!({ "recording": null, "dictating": false })
    );
    assert_eq!(seqs(&mut client, 3), [1, 2, 3]);
    let event = &bridge.log.after(0).unwrap()[0];
    assert!(read_until_status_reply(&mut client, "c-1").is_empty());
    assert_eq!(event.data, passage(0));
}

#[test]
fn a_known_cursor_gets_the_events_after_it() {
    let bridge = Bridge::start("server-partial");
    fill(&bridge.log, A, 5);
    let instance = bridge.log.instance().to_owned();
    let (mut client, welcome) = bridge.client(Some(&instance), 2);
    assert_eq!(welcome["replay"], "partial");
    assert_eq!(seqs(&mut client, 3), [3, 4, 5]);
    assert!(read_until_status_reply(&mut client, "c-1").is_empty());

    let (mut client, welcome) = bridge.client(Some(&instance), 5);
    assert_eq!(welcome["replay"], "partial");
    assert!(read_until_status_reply(&mut client, "c-1").is_empty());
}

#[test]
fn another_instance_or_a_cursor_above_the_head_gets_everything() {
    let bridge = Bridge::start("server-full-again");
    fill(&bridge.log, A, 2);
    let (mut client, welcome) = bridge.client(Some(OTHER_INSTANCE), 1);
    assert_eq!(welcome["replay"], "full");
    assert_eq!(seqs(&mut client, 2), [1, 2]);

    let instance = bridge.log.instance().to_owned();
    let (mut client, welcome) = bridge.client(Some(&instance), 99);
    assert_eq!(welcome["replay"], "full");
    assert_eq!(seqs(&mut client, 2), [1, 2]);
}

#[test]
fn a_cursor_below_removed_events_gets_a_gap() {
    let bridge = Bridge::start("server-gap");
    fill(&bridge.log, A, 3); // 1..3
    bridge.log.append("passage", Some(B), passage(0)).unwrap(); // 4
    bridge.log.remove_meeting(A).unwrap(); // 5; 1..3 go
    let instance = bridge.log.instance().to_owned();
    let (mut client, welcome) = bridge.client(Some(&instance), 1);
    assert_eq!(welcome["replay"], "gap");
    assert_eq!(welcome["oldest"], 4);
    assert_eq!(welcome["head"], 5);
    let events: Vec<Value> = (0..2).map(|_| client.read()).collect();
    assert_eq!(events[0]["seq"], 4);
    assert_eq!(events[1]["seq"], 5);
    assert_eq!(events[1]["event"], "meeting.removed");
    assert_eq!(events[1]["meeting"], A);

    // Just below the oldest stored event is still a partial replay.
    let (mut client, welcome) = bridge.client(Some(&instance), 3);
    assert_eq!(welcome["replay"], "partial");
    assert_eq!(seqs(&mut client, 2), [4, 5]);
}

#[test]
fn an_empty_log_welcomes_with_oldest_above_head() {
    let bridge = Bridge::start("server-empty");
    let (mut client, welcome) = bridge.client(None, 0);
    assert_eq!(welcome["replay"], "full");
    assert_eq!(
        (welcome["head"].as_u64(), welcome["oldest"].as_u64()),
        (Some(0), Some(1))
    );
    assert!(read_until_status_reply(&mut client, "c-1").is_empty());
}

#[test]
fn live_events_follow_the_replay() {
    let bridge = Bridge::start("server-live");
    fill(&bridge.log, A, 2);
    let (mut client, _) = bridge.client(None, 0);
    assert_eq!(seqs(&mut client, 2), [1, 2]);
    bridge
        .log
        .append("passage", Some(A), passage(5000))
        .unwrap();
    bridge.log.remove_meeting(A).unwrap();
    let live: Vec<Value> = (0..2).map(|_| client.read()).collect();
    assert_eq!(live[0]["seq"], 3);
    assert_eq!(live[0]["data"], passage(5000));
    assert_eq!(live[1]["seq"], 4);
    assert_eq!(live[1]["event"], "meeting.removed");
}

/// Appends run on another thread while the client replays, so events land before, during and
/// after the replay. Every number must arrive once, in order.
fn replay_while_appending(bridge: &Bridge, stored: u64, appended: u64) {
    fill(&bridge.log, A, stored);
    let start = Arc::new(Barrier::new(2));
    let writer = {
        let log = bridge.log.clone();
        let start = start.clone();
        thread::spawn(move || {
            start.wait();
            for i in 0..appended {
                log.append("passage", Some(B), passage(i * 1000)).unwrap();
                if i % 50 == 0 {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        })
    };
    let mut client = Client::connect(bridge.port());
    start.wait();
    client.hello(bridge.token(), None, 0);
    let welcome = client.read();
    assert_eq!(welcome["replay"], "full");
    let total = stored + appended;
    let received = client.read_events_until(total);
    writer.join().unwrap();
    assert_eq!(received, (1..=total).collect::<Vec<_>>());
    assert!(
        welcome["head"].as_u64().unwrap() < total,
        "the writer finished before the hello, so nothing ran concurrently: {welcome}"
    );
    assert!(read_until_status_reply(&mut client, "c-end").is_empty());
}

#[test]
fn events_appended_during_the_replay_arrive_once_in_order() {
    let bridge = Bridge::start("server-concurrent");
    replay_while_appending(&bridge, 2_000, 1_500);
}

#[test]
fn a_subscriber_that_lags_catches_up_from_the_log() {
    // A live buffer of 4 events lags on every burst, so the catch-up path carries the stream.
    let dir = TempDir::new("server-lag");
    let log = EventLog::open_with_live_capacity(dir.path(), 4).unwrap();
    let bridge = Bridge::start_on(dir, log, Options::default());
    replay_while_appending(&bridge, 500, 1_500);
}

#[test]
fn a_wrong_token_is_refused_with_4001() {
    let bridge = Bridge::start("server-token");
    let mut client = Client::connect(bridge.port());
    client.hello("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", None, 0);
    assert_eq!(client.read_refusal(), ("bad_token".into(), Some(4001)));

    // A token of the wrong shape is a wrong token too.
    let mut client = Client::connect(bridge.port());
    client.hello("abc", None, 0);
    assert_eq!(client.read_refusal(), ("bad_token".into(), Some(4001)));
}

#[test]
fn an_unsupported_protocol_is_refused_with_4002() {
    let bridge = Bridge::start("server-protocol");
    let mut client = Client::connect(bridge.port());
    client.send(json!({ "type": "hello", "protocol": 1, "token": bridge.token(), "client": "rukoo", "instance": null, "after": 0 }));
    assert_eq!(
        client.read_refusal(),
        ("unsupported_protocol".into(), Some(4002))
    );
}

#[test]
fn no_hello_in_time_is_refused_with_4003() {
    let options = Options {
        hello_timeout: Duration::from_millis(300),
        ..Options::default()
    };
    let bridge = Bridge::start_with("server-hello-timeout", options);
    let started = Instant::now();
    let mut client = Client::connect(bridge.port());
    assert_eq!(client.read_refusal(), ("hello_timeout".into(), Some(4003)));
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(250), "{waited:?}");
    assert!(waited < Duration::from_secs(5), "{waited:?}");
}

#[test]
fn another_message_before_hello_is_refused_with_4003() {
    let bridge = Bridge::start("server-hello-first");
    let mut client = Client::connect(bridge.port());
    client.command("c-1", "status.get", json!({}));
    assert_eq!(client.read_refusal(), ("hello_timeout".into(), Some(4003)));

    let mut client = Client::connect(bridge.port());
    client.send_text("not json".into());
    assert_eq!(client.read_refusal(), ("hello_timeout".into(), Some(4003)));
}

#[test]
fn a_fifth_client_is_refused_with_4004() {
    let bridge = Bridge::start("server-clients");
    let mut clients: Vec<Client> = (0..4).map(|_| bridge.client(None, 0).0).collect();
    let mut fifth = Client::connect(bridge.port());
    fifth.hello(bridge.token(), None, 0);
    assert_eq!(
        fifth.read_refusal(),
        ("too_many_clients".into(), Some(4004))
    );

    // A wrong token is still refused as a wrong token while the slots are full.
    let mut stranger = Client::connect(bridge.port());
    stranger.hello("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", None, 0);
    assert_eq!(stranger.read_refusal(), ("bad_token".into(), Some(4001)));

    // A slot frees up when a client leaves.
    let mut leaving = clients.pop().unwrap();
    let normal = tungstenite::protocol::CloseFrame {
        code: tungstenite::protocol::frame::coding::CloseCode::Normal,
        reason: "".into(),
    };
    leaving.socket.close(Some(normal)).unwrap();
    assert_eq!(
        leaving.read_close(),
        Some(1000),
        "the server answers the close"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut next = Client::connect(bridge.port());
        next.hello(bridge.token(), None, 0);
        let message = next.read();
        if message["type"] == "welcome" {
            break;
        }
        assert_eq!(message["code"], "too_many_clients");
        assert!(Instant::now() < deadline, "the slot never came back");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn every_origin_is_refused_with_403() {
    let bridge = Bridge::start("server-origin");
    // Browsers send `null` from sandboxed frames, data: frames and files, and Chrome sends
    // `file://` for a local file. Rukoo sends no Origin at all.
    for origin in [
        "https://evil.example",
        "http://localhost:3000",
        "null",
        "file://",
        "chrome-extension://abcdefghijklmnop",
        "",
    ] {
        match Client::try_connect(bridge.port(), "/bridge", &[("origin", origin)]) {
            Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 403),
            Err(e) => panic!("{origin}: expected HTTP 403, got {e}"),
            Ok(_) => panic!("{origin}: the upgrade succeeded"),
        }
    }
    match Client::try_connect(bridge.port(), "/v1", &[]) {
        Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 404),
        Err(e) => panic!("expected HTTP 404, got {e}"),
        Ok(_) => panic!("the upgrade on another path succeeded"),
    }
    // Rukoo's main process sends no Origin; that and the default client both connect.
    bridge.client(None, 0);
}

/// Whether the server ended `stream` within the read timeout.
fn ended(stream: &mut TcpStream) -> bool {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
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

#[test]
fn connections_that_never_say_hello_cant_keep_rukoo_out() {
    // Long enough that only the server's room for new connections closes the silent ones.
    let options = Options {
        hello_timeout: Duration::from_secs(60),
        ..Options::default()
    };
    let bridge = Bridge::start_with("server-pending", options);
    // A program without the token fills the waiting room: sockets that never finish their
    // upgrade, and upgraded ones that never say hello.
    let mut raw: Vec<TcpStream> = (0..8)
        .map(|_| TcpStream::connect(("127.0.0.1", bridge.port())).unwrap())
        .collect();
    let mut silent: Vec<Client> = (0..8).map(|_| Client::connect(bridge.port())).collect();
    // Rukoo still gets in, also while new silent sockets keep coming.
    for _ in 0..3 {
        let (mut rukoo, _) = bridge.client(None, 0);
        assert!(read_until_status_reply(&mut rukoo, "c-1").is_empty());
        silent.push(Client::connect(bridge.port()));
    }
    // The oldest waited longest and made room.
    assert!(ended(&mut raw[0]), "the oldest silent socket is still open");
}

#[test]
fn a_message_over_one_mebibyte_closes_the_connection() {
    let bridge = Bridge::start("server-size");
    let (mut client, _) = bridge.client(None, 0);

    // Exactly the limit is fine.
    let envelope =
        json!({ "type": "command", "id": "c-1", "command": "status.get", "args": { "pad": "" } })
            .to_string();
    let pad = "x".repeat(MAX_RUKOO_MESSAGE - envelope.len());
    let at_limit = envelope.replace("\"pad\":\"\"", &format!("\"pad\":\"{pad}\""));
    assert_eq!(at_limit.len(), MAX_RUKOO_MESSAGE);
    client.send_text(at_limit);
    client.read_reply("c-1");

    // The server stops reading at the limit and closes. On Linux the rest of the write can meet that close as a reset
    // or a broken pipe before the client reads the close frame; either way the connection is gone.
    let over = format!("{}x", " ".repeat(MAX_RUKOO_MESSAGE));
    match client.try_send_text(over) {
        Ok(()) => {
            let code = client.read_close();
            assert!(matches!(code, Some(1009) | None), "{code:?}");
        }
        Err(tungstenite::Error::Io(e)) => assert!(
            matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionAborted
            ),
            "{e:?}"
        ),
        Err(e) => panic!("{e:?}"),
    }

    // The server carries on for others.
    bridge.client(None, 0);
}

#[test]
fn the_server_pings_and_drops_a_client_that_never_answers() {
    let options = Options {
        ping_interval: Duration::from_millis(150),
        pong_timeout: Duration::from_millis(300),
        ..Options::default()
    };
    let bridge = Bridge::start_with("server-ping", options);
    let (mut client, _) = bridge.client(None, 0);
    match client.socket.read().unwrap() {
        tungstenite::Message::Ping(_) => {}
        other => panic!("expected a ping, got {other:?}"),
    }
    // Reading answered that ping; a client that keeps reading stays connected.
    for _ in 0..4 {
        assert!(matches!(
            client.socket.read().unwrap(),
            tungstenite::Message::Ping(_)
        ));
    }
    assert!(read_until_status_reply(&mut client, "c-1")
        .iter()
        .all(|m| m["type"] == "event"));

    // A client that stops reading never answers the next ping. Once it reads again it finds
    // the pings sent before the drop, then the end; a live connection would ping forever.
    let (mut silent, _) = bridge.client(None, 0);
    thread::sleep(PING_DROP_WAIT);
    let mut ended = false;
    for _ in 0..20 {
        match silent.socket.read() {
            Ok(tungstenite::Message::Text(text)) => panic!("the silent client got {text}"),
            Ok(tungstenite::Message::Close(_)) | Err(_) => {
                ended = true;
                break;
            }
            Ok(_) => continue,
        }
    }
    assert!(ended, "the silent client was never dropped");
}

#[test]
fn shutdown_closes_clients_with_1001() {
    let mut bridge = Bridge::start("server-quit");
    let (mut client, _) = bridge.client(None, 0);
    bridge.shutdown();
    assert_eq!(client.read_close(), Some(1001));
    assert!(!bridge.discovery_path.exists());
}

#[test]
fn commands_reach_the_meeting_control() {
    let bridge = Bridge::start("server-commands");
    let (mut client, _) = bridge.client(None, 0);

    client.command(
        "c-1",
        "meeting.start",
        json!({ "title": "Weekly with Anna", "tracks": ["mic"], "context": { "calendarEvent": "abc" } }),
    );
    let reply = client.read_reply("c-1");
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["result"], json!({ "meeting": NEW_MEETING }));
    let started = bridge.control.started.lock().unwrap()[0].clone();
    assert_eq!(started.title.as_deref(), Some("Weekly with Anna"));
    assert_eq!(started.tracks, Some(vec![voice_protocol::Track::Mic]));
    assert_eq!(started.screen, None);
    assert_eq!(started.context.unwrap()["calendarEvent"], "abc");

    client.command("c-2", "meeting.start", json!({}));
    let reply = client.read_reply("c-2");
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "busy");
    assert_eq!(reply["error"]["meeting"], NEW_MEETING);

    client.command("c-3", "status.get", json!({}));
    assert_eq!(
        client.read_reply("c-3")["result"],
        json!({ "recording": NEW_MEETING, "dictating": false })
    );

    client.command(
        "c-4",
        "meeting.mark",
        json!({ "meeting": NEW_MEETING, "label": "Decision" }),
    );
    assert_eq!(
        client.read_reply("c-4")["result"],
        json!({ "mark": "k-1", "at": 4200 })
    );

    client.command("c-5", "meeting.resume", json!({ "meeting": NEW_MEETING }));
    assert_eq!(client.read_reply("c-5")["error"]["code"], "not_recording");
    client.command("c-6", "meeting.pause", json!({ "meeting": NEW_MEETING }));
    assert_eq!(client.read_reply("c-6")["result"], json!({}));
    client.command("c-7", "meeting.resume", json!({ "meeting": NEW_MEETING }));
    assert_eq!(client.read_reply("c-7")["result"], json!({}));

    client.command(
        "c-8",
        "meeting.stop",
        json!({ "meeting": A.replace("4f2a", "0000") }),
    );
    assert_eq!(client.read_reply("c-8")["error"]["code"], "not_found");
    client.command("c-9", "meeting.stop", json!({ "meeting": NEW_MEETING }));
    assert_eq!(client.read_reply("c-9")["result"], json!({}));
    client.command("c-10", "meeting.stop", json!({ "meeting": DONE_MEETING }));
    assert_eq!(client.read_reply("c-10")["error"]["code"], "not_recording");

    client.command("c-11", "meeting.list", json!({ "limit": 1 }));
    let meetings = client.read_reply("c-11")["result"]["meetings"].clone();
    assert_eq!(
        meetings,
        json!([{ "id": DONE_MEETING, "title": "Standup", "startedAt": 1_792_998_000_000u64, "endedAt": 1_792_998_900_000u64, "state": "done", "duration": 900_000 }])
    );
    client.command("c-12", "meeting.list", json!({}));
    client.read_reply("c-12");

    client.command("c-13", "meeting.get", json!({ "meeting": DONE_MEETING }));
    assert_eq!(client.read_reply("c-13")["result"]["head"], 0);
    client.command(
        "c-14",
        "frame.get",
        json!({ "meeting": DONE_MEETING, "at": 412_500 }),
    );
    assert_eq!(client.read_reply("c-14")["error"]["code"], "not_available");

    // clip.get gets a deadline CLIP_WAIT after the server read it.
    let clip = json!({ "meeting": DONE_MEETING, "start": 60_000, "end": 75_000 });
    let sent = Instant::now();
    client.command("c-15", "clip.get", clip.clone());
    assert_eq!(client.read_reply("c-15")["error"]["code"], "not_available");
    let answered = Instant::now();
    let deadline = bridge.control.deadlines.lock().unwrap()[0];
    assert!(
        deadline >= sent + CLIP_WAIT && deadline <= answered + CLIP_WAIT,
        "the deadline isn't CLIP_WAIT after the command arrived"
    );
    let clips = json!({ "clips": [{ "path": "C:\\clips\\c-1.mp4", "start": 59_850, "end": 75_120, "width": 1920, "height": 1080, "bytes": 2_481_152 }] });
    *bridge.control.clips.lock().unwrap() = Some(clips.clone());
    client.command("c-16", "clip.get", clip);
    assert_eq!(client.read_reply("c-16")["result"], clips);

    client.command(
        "c-17",
        "meeting.protected",
        json!({ "meeting": DONE_MEETING }),
    );
    assert_eq!(client.read_reply("c-17")["result"], json!({}));
    client.command(
        "c-18",
        "meeting.protected",
        json!({ "meeting": A.replace("4f2a", "0000") }),
    );
    assert_eq!(client.read_reply("c-18")["error"]["code"], "not_found");

    // screen.get and screen.select need a recording meeting that records the screen.
    client.command("c-19", "screen.get", json!({ "meeting": DONE_MEETING }));
    assert_eq!(client.read_reply("c-19")["error"]["code"], "not_recording");
    client.command("c-20", "meeting.start", json!({ "screen": "frames" }));
    assert_eq!(client.read_reply("c-20")["result"]["meeting"], NEW_MEETING);
    assert_eq!(
        bridge.control.started.lock().unwrap()[1].screen,
        Some(voice_protocol::ScreenMode::Frames)
    );
    client.command("c-21", "screen.get", json!({ "meeting": NEW_MEETING }));
    assert_eq!(client.read_reply("c-21")["error"]["code"], "not_available");
    let screen = json!({ "screen": "frames", "state": "recording", "video": false, "frames": true, "monitor": "\\\\.\\DISPLAY1",
        "monitors": [{ "id": "\\\\.\\DISPLAY1", "name": "DELL U2720Q", "width": 2560, "height": 1440, "primary": true },
                     { "id": "\\\\.\\DISPLAY2", "name": "Display 2", "width": 1440, "height": 2560, "primary": false }] });
    *bridge.control.screen.lock().unwrap() = Some(screen.clone());
    client.command("c-22", "screen.get", json!({ "meeting": NEW_MEETING }));
    assert_eq!(client.read_reply("c-22")["result"], screen);
    client.command(
        "c-23",
        "screen.select",
        json!({ "meeting": NEW_MEETING, "monitor": "\\\\.\\DISPLAY2" }),
    );
    assert_eq!(
        client.read_reply("c-23")["result"],
        json!({ "monitor": "\\\\.\\DISPLAY2" })
    );
    client.command(
        "c-24",
        "screen.select",
        json!({ "meeting": NEW_MEETING, "monitor": "\\\\.\\DISPLAY9" }),
    );
    assert_eq!(client.read_reply("c-24")["error"]["code"], "not_found");

    let calls = bridge.control.calls.lock().unwrap().clone();
    assert!(calls.contains(&"list 1".to_owned()), "{calls:?}");
    assert!(calls.contains(&"list 50".to_owned()), "{calls:?}");
    assert!(
        calls.contains(&format!("frame {DONE_MEETING} 412500")),
        "{calls:?}"
    );
    assert!(
        calls.contains(&format!("mark {NEW_MEETING} Some(\"Decision\")")),
        "{calls:?}"
    );
    for call in [
        format!("clip {DONE_MEETING} 60000 75000"),
        format!("protected {DONE_MEETING}"),
        format!("screen {NEW_MEETING}"),
        format!("select {NEW_MEETING} \\\\.\\DISPLAY2"),
    ] {
        assert!(calls.contains(&call), "{call} is missing from {calls:?}");
    }
}

#[test]
fn queued_commands_keep_their_deadline() {
    let bridge = Bridge::start("server-clip-deadline");
    bridge.control.slow_clips.store(true, Ordering::SeqCst);
    let (mut client, _) = bridge.client(None, 0);
    // Two clips that each take until their deadline and a heartbeat, in one burst. Voice runs
    // them one after another; counted from dispatch, the second clip would end 8 s after it
    // was sent and the heartbeat after that.
    let clip = json!({ "meeting": DONE_MEETING, "start": 0, "end": 300_000 });
    let sent = Instant::now();
    client.command("c-1", "clip.get", clip.clone());
    client.command("c-2", "clip.get", clip);
    client.command("c-3", "status.get", json!({}));
    let limit = CLIP_WAIT + Duration::from_secs(1);
    for id in ["c-1", "c-2", "c-3"] {
        let reply = client.read_reply(id);
        let took = sent.elapsed();
        assert!(took < limit, "{id} answered after {took:?}: {reply}");
        if id == "c-1" {
            assert!(took >= CLIP_WAIT, "the first clip didn't wait: {took:?}");
        }
    }
    let deadlines = bridge.control.deadlines.lock().unwrap().clone();
    assert_eq!(deadlines.len(), 2);
    assert!(deadlines[1] - deadlines[0] < Duration::from_secs(1));
}

#[test]
fn screen_commands_queued_behind_a_clip_keep_their_deadline() {
    let bridge = Bridge::start("server-screen-deadline");
    let (mut client, _) = bridge.client(None, 0);
    client.command("c-1", "meeting.start", json!({ "screen": "frames" }));
    assert_eq!(client.read_reply("c-1")["result"]["meeting"], NEW_MEETING);
    *bridge.control.screen.lock().unwrap() = Some(
        json!({ "screen": "frames", "state": "recording", "video": false, "frames": true, "monitor": "\\\\.\\DISPLAY1",
        "monitors": [{ "id": "\\\\.\\DISPLAY2", "name": "Display 2", "width": 1440, "height": 2560, "primary": false }] }),
    );
    *bridge.control.clips.lock().unwrap() = Some(json!({ "clips": [] }));
    bridge.control.slow_clips.store(true, Ordering::SeqCst);
    // screen.get and screen.select wait for the screen recorder, which can be slow to start or
    // answer: behind a clip that took its 4 s, they get what is left of their own 4 s.
    let sent = Instant::now();
    client.command(
        "c-2",
        "clip.get",
        json!({ "meeting": DONE_MEETING, "start": 0, "end": 300_000 }),
    );
    client.command("c-3", "screen.get", json!({ "meeting": NEW_MEETING }));
    client.command(
        "c-4",
        "screen.select",
        json!({ "meeting": NEW_MEETING, "monitor": "\\\\.\\DISPLAY2" }),
    );
    for id in ["c-2", "c-3", "c-4"] {
        assert_eq!(client.read_reply(id)["ok"], true, "{id}");
    }
    let deadlines = bridge.control.screen_deadlines.lock().unwrap().clone();
    assert_eq!(deadlines.len(), 2);
    for deadline in deadlines {
        assert!(
            deadline >= sent + CLIP_WAIT && deadline < sent + CLIP_WAIT + Duration::from_secs(1),
            "the deadline isn't CLIP_WAIT after the command arrived: {:?} after it was sent",
            deadline - sent
        );
    }
}

#[test]
fn media_commands_reach_the_meeting_control() {
    let bridge = Bridge::start("server-media");
    let (mut client, _) = bridge.client(None, 0);

    client.command(
        "c-50",
        "media.open",
        json!({ "meeting": DONE_MEETING, "kind": "audio", "start": 600_000, "end": 1_200_000 }),
    );
    assert_eq!(
        client.read_reply("c-50")["result"],
        json!({ "media": FAKE_MEDIA, "ready": true, "type": "audio/wav", "size": FAKE_MEDIA_SIZE, "start": 600_000, "end": 1_200_000 })
    );
    // A null end counts as left out.
    client.command(
        "c-51",
        "media.open",
        json!({ "meeting": DONE_MEETING, "kind": "frame", "start": 412_500, "end": null }),
    );
    assert_eq!(client.read_reply("c-51")["result"]["type"], "image/jpeg");
    client.command(
        "c-52",
        "media.open",
        json!({ "meeting": DONE_MEETING, "kind": "video", "start": 0, "end": 3_600_000 }),
    );
    assert_eq!(client.read_reply("c-52")["error"]["code"], "not_available");
    client.command(
        "c-53",
        "media.open",
        json!({ "meeting": B, "kind": "audio", "start": 0, "end": 1000 }),
    );
    assert_eq!(client.read_reply("c-53")["error"]["code"], "not_found");

    client.command(
        "c-54",
        "media.read",
        json!({ "media": FAKE_MEDIA, "offset": 1_048_576, "length": 1_048_576 }),
    );
    let reply = client.read_reply("c-54");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(
        reply["result"],
        voice_protocol::media_read_result(1_048_576, &fake_media(1_048_576, 1_048_576))
    );
    // The end of the file gives fewer bytes, and past it none.
    client.command(
        "c-55",
        "media.read",
        json!({ "media": FAKE_MEDIA, "offset": 2 << 20, "length": 1_048_576 }),
    );
    assert_eq!(
        client.read_reply("c-55")["result"],
        voice_protocol::media_read_result(2 << 20, &fake_media(2 << 20, 100))
    );
    client.command(
        "c-56",
        "media.read",
        json!({ "media": FAKE_MEDIA, "offset": FAKE_MEDIA_SIZE + 5, "length": 10 }),
    );
    assert_eq!(
        client.read_reply("c-56")["result"],
        json!({ "offset": FAKE_MEDIA_SIZE + 5, "data": "" })
    );
    client.command(
        "c-57",
        "media.read",
        json!({ "media": "md-AAAAAAAAAAAAAAAAAAAAAA", "offset": 0, "length": 1 }),
    );
    assert_eq!(client.read_reply("c-57")["error"]["code"], "not_found");

    // Arguments that break the protocol's rules never reach the control.
    client.command(
        "c-58",
        "media.open",
        json!({ "meeting": DONE_MEETING, "kind": "audio", "start": 0, "end": 3_600_001 }),
    );
    assert_eq!(client.read_reply("c-58")["error"]["code"], "bad_request");
    client.command(
        "c-59",
        "media.read",
        json!({ "media": FAKE_MEDIA, "offset": 0, "length": 1_048_577 }),
    );
    assert_eq!(client.read_reply("c-59")["error"]["code"], "bad_request");

    let calls = bridge.control.calls.lock().unwrap().clone();
    let media: Vec<&str> = calls
        .iter()
        .map(String::as_str)
        .filter(|call| call.starts_with("media."))
        .collect();
    assert_eq!(
        media,
        [
            format!("media.open {DONE_MEETING} audio 600000 Some(1200000)"),
            format!("media.open {DONE_MEETING} frame 412500 None"),
            format!("media.open {DONE_MEETING} video 0 Some(3600000)"),
            format!("media.open {B} audio 0 Some(1000)"),
            format!("media.read {FAKE_MEDIA} 1048576 1048576"),
            format!("media.read {FAKE_MEDIA} 2097152 1048576"),
            format!("media.read {FAKE_MEDIA} {} 10", FAKE_MEDIA_SIZE + 5),
            "media.read md-AAAAAAAAAAAAAAAAAAAAAA 0 1".to_owned(),
        ]
    );
}

#[test]
fn a_full_media_read_fits_in_one_reply() {
    let bridge = Bridge::start("server-media-limit");
    let (mut client, _) = bridge.client(None, 0);
    client.command(
        "c-1",
        "media.read",
        json!({ "media": FAKE_MEDIA, "offset": 0, "length": voice_protocol::MAX_MEDIA_READ }),
    );
    let reply = client.read_reply("c-1");
    assert_eq!(
        reply["ok"], true,
        "the reply was refused: {}",
        reply["error"]
    );
    let data = reply["result"]["data"].as_str().unwrap();
    // 1 MiB in base64 is 1,398,104 characters, far below the 8 MiB a message from Voice may hold.
    assert_eq!(data.len(), 1_398_104);
    assert!(reply.to_string().len() < voice_protocol::MAX_VOICE_MESSAGE / 4);
    assert_eq!(
        reply["result"],
        voice_protocol::media_read_result(0, &fake_media(0, 1 << 20))
    );
}

#[test]
fn a_voice_without_media_answers_with_the_defaults() {
    let bridge = Bridge::start_plain("server-media-defaults");
    let (mut client, _) = bridge.client(None, 0);
    client.command(
        "c-1",
        "media.open",
        json!({ "meeting": DONE_MEETING, "kind": "audio", "start": 0, "end": 1000 }),
    );
    let reply = client.read_reply("c-1");
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "not_available");
    client.command(
        "c-2",
        "media.read",
        json!({ "media": FAKE_MEDIA, "offset": 0, "length": 1024 }),
    );
    assert_eq!(client.read_reply("c-2")["error"]["code"], "not_found");
    // The other commands still reach the fake behind it.
    client.command("c-3", "meeting.get", json!({ "meeting": DONE_MEETING }));
    assert_eq!(client.read_reply("c-3")["ok"], true);
    let calls = bridge.control.calls.lock().unwrap().clone();
    assert_eq!(calls, [format!("get {DONE_MEETING}")]);
}

#[test]
fn bad_and_unknown_commands_get_error_replies() {
    let bridge = Bridge::start("server-bad-commands");
    let (mut client, _) = bridge.client(None, 0);

    client.command("c-1", "meeting.stop", json!({}));
    let reply = client.read_reply("c-1");
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "bad_request");

    client.command("c-2", "meeting.start", json!({ "tracks": ["mic", "room"] }));
    assert_eq!(client.read_reply("c-2")["error"]["code"], "bad_request");

    client.command(
        "c-3",
        "example.unknown",
        json!({ "meeting": A, "from": 1000, "to": 5000 }),
    );
    let reply = client.read_reply("c-3");
    assert_eq!(reply["error"]["code"], "unknown_command");

    // Without a readable id there is nothing to answer; the next reply is for c-4.
    client.send(json!({ "type": "command", "command": "status.get" }));
    client.command(&"x".repeat(65), "status.get", json!({}));
    client.send(json!({ "type": "hello", "protocol": 1 }));
    client.send_text("not json".into());
    client
        .socket
        .send(tungstenite::Message::binary(vec![1, 2, 3]))
        .unwrap();
    client.command("c-4", "status.get", json!({}));
    client.read_reply("c-4");

    // A control that panics answers `failed`, and the connection carries on.
    client.command(
        "c-5",
        "meeting.mark",
        json!({ "meeting": A, "label": "panic" }),
    );
    assert_eq!(client.read_reply("c-5")["error"]["code"], "failed");
    client.command("c-6", "status.get", json!({}));
    client.read_reply("c-6");
}

#[test]
fn replies_keep_the_order_of_their_commands() {
    let bridge = Bridge::start("server-command-order");
    let (mut client, _) = bridge.client(None, 0);
    // A slow command, then a bad one that is answered without the control, then a fast one.
    client.command(
        "c-1",
        "meeting.mark",
        json!({ "meeting": DONE_MEETING, "label": "slow" }),
    );
    client.command("c-2", "meeting.stop", json!({}));
    client.command("c-3", "status.get", json!({}));
    client.read_reply("c-1");
    client.read_reply("c-2");
    client.read_reply("c-3");
}

#[test]
fn events_keep_flowing_while_a_command_runs() {
    let bridge = Bridge::start("server-command-events");
    let (open, gate) = std::sync::mpsc::channel();
    *bridge.control.gate.lock().unwrap() = Some(gate);
    let (mut client, _) = bridge.client(None, 0);
    // The command blocks until the event has arrived; if events waited behind commands, this
    // read would time out.
    client.command(
        "c-1",
        "meeting.mark",
        json!({ "meeting": DONE_MEETING, "label": "wait" }),
    );
    bridge.log.append("passage", Some(A), passage(0)).unwrap();
    let first = client.read();
    assert_eq!(first["type"], "event", "{first}");
    client.command("c-2", "status.get", json!({}));
    open.send(()).unwrap();
    client.read_reply("c-1");
    client.read_reply("c-2");
}

#[test]
fn the_handle_reports_its_clients_and_the_last_refused_hello() {
    let bridge = Bridge::start("server-clients-info");
    let handle = bridge.handle.as_ref().unwrap();
    assert_eq!(handle.clients(), voice_bridge::Clients::default());
    fill(&bridge.log, A, 3);
    let instance = bridge.log.instance().to_owned();
    let (mut client, _) = bridge.client(Some(&instance), 1);
    assert_eq!(seqs(&mut client, 2), [2, 3]);
    // The server counts an event as sent once it has written it; the status reply comes after.
    assert!(read_until_status_reply(&mut client, "c-1").is_empty());
    let clients = handle.clients();
    assert_eq!(clients.connected.len(), 1);
    let rukoo = &clients.connected[0];
    assert_eq!(rukoo.pid, Some(u64::from(RUKOO_PID)));
    assert_eq!(
        (
            rukoo.client.as_str(),
            rukoo.client_version.as_deref(),
            rukoo.protocol,
            rukoo.after,
            rukoo.sent,
            rukoo.welcomed
        ),
        ("rukoo", Some("2.3.0"), voice_protocol::PROTOCOL, 1, 3, true)
    );
    bridge
        .log
        .append("passage", Some(A), passage(9000))
        .unwrap();
    assert_eq!(seqs(&mut client, 1), [4]);
    assert!(read_until_status_reply(&mut client, "c-2").is_empty());
    assert_eq!(handle.clients().connected[0].sent, 4);
    assert_eq!(handle.clients().last_admitted_at, Some(rukoo.connected_at));

    // A wrong token and a version Voice doesn't speak are remembered; a client that never said
    // hello isn't.
    let mut stranger = Client::connect(bridge.port());
    stranger.hello("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", None, 0);
    assert_eq!(stranger.read_refusal().0, "bad_token");
    let refused = handle.clients().refused.unwrap();
    assert_eq!(
        (
            refused.refusal,
            refused.protocol,
            refused.client.as_deref(),
            refused.client_version.as_deref()
        ),
        (
            voice_protocol::Refusal::BadToken,
            Some(u64::from(voice_protocol::PROTOCOL)),
            Some("rukoo"),
            Some("2.3.0")
        )
    );
    let mut newer = Client::connect(bridge.port());
    newer.send(json!({ "type": "hello", "protocol": 3, "token": bridge.token(), "client": "rukoo", "clientVersion": "9.0.0", "instance": null, "after": 0 }));
    assert_eq!(newer.read_refusal().0, "unsupported_protocol");
    let refused = handle.clients().refused.unwrap();
    assert_eq!(
        (refused.refusal, refused.protocol),
        (voice_protocol::Refusal::UnsupportedProtocol, Some(3))
    );
    let mut silent = Client::connect(bridge.port());
    silent.command("c-1", "status.get", json!({}));
    assert_eq!(silent.read_refusal().0, "hello_timeout");
    assert_eq!(
        handle.clients().refused.unwrap().refusal,
        voice_protocol::Refusal::UnsupportedProtocol
    );

    // A client that leaves is gone from the list.
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.clients().connected.is_empty() {
        assert!(Instant::now() < deadline, "the client stayed in the list");
        thread::sleep(Duration::from_millis(20));
    }
}
