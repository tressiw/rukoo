//! The screen process's control port: the recorder's line protocol (`voice/RECORDING.md`, "The
//! control port") on `127.0.0.1`, one JSON object per line, each request carrying the token.
//!
//! Commands: `status`, `monitors`, `monitor { id }`, `pause`, `resume`, `stop`, `mode`,
//! `exclude { pids }`.
//!
//! Connection threads only read, check and answer what is wrong with a request; the process's
//! main loop answers the rest, so the journal keeps a single writer.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};

pub const MAX_CONNECTIONS: usize = 8;
/// A connection that sends no complete request within this time is closed.
pub const FIRST_REQUEST: Duration = Duration::from_secs(5);
/// The longest request line, in bytes.
pub const MAX_LINE: usize = 64 * 1024;
/// The commands the port knows; anything else is `unknown_command`.
pub const COMMANDS: [&str; 8] = [
    "status", "monitors", "monitor", "pause", "resume", "stop", "mode", "exclude",
];
/// The most pids `exclude` takes.
pub const MAX_EXCLUDED: usize = 64;

/// The pids of an `exclude` request: an array of at most `MAX_EXCLUDED` pids from 1 up, sorted
/// and without repeats. `None` when `pids` isn't that.
pub fn excluded_pids(request: &Value) -> Option<Vec<u32>> {
    let list = request["pids"].as_array()?;
    if list.len() > MAX_EXCLUDED {
        return None;
    }
    let mut pids = list
        .iter()
        .map(|pid| {
            pid.as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .filter(|&pid| pid > 0)
        })
        .collect::<Option<Vec<u32>>>()?;
    pids.sort_unstable();
    pids.dedup();
    Some(pids)
}
/// The longest monitor id `monitor` takes, as `screen.select` in the bridge.
pub const MAX_MONITOR_ID: usize = 64;

/// 32 random bytes, base64url without padding.
pub fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(format!("no random bytes: {e}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// `127.0.0.1:0`.
pub fn bind() -> io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
}

/// A request with a good token, for the process's main loop. The answer goes back on `reply`.
/// `body` is the request without its token, so it can be logged.
#[derive(Debug)]
pub struct Request {
    pub body: Value,
    pub reply: mpsc::Sender<Value>,
}

/// `{"ok": false, "error": code}`.
pub fn error(code: &str) -> Value {
    json!({ "ok": false, "error": code })
}

/// Serves the port on its own thread: checks the token and the limits, answers bad requests
/// itself and hands the rest to `to_main`.
///
/// Errors it answers itself: `bad_token` and a line that isn't a JSON object close the
/// connection; `bad_request` for a missing `cmd` or a bad `monitor` id and `unknown_command`
/// keep it open. A line over `MAX_LINE` or no request within `FIRST_REQUEST` closes it without
/// an answer, and connections over `MAX_CONNECTIONS` are closed at once.
pub fn serve(
    listener: TcpListener,
    token: String,
    to_main: mpsc::SyncSender<Request>,
) -> io::Result<JoinHandle<()>> {
    let token: Arc<str> = token.into();
    std::thread::Builder::new()
        .name("screen-control".into())
        .spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if active.load(Ordering::Acquire) >= MAX_CONNECTIONS {
                    continue;
                }
                active.fetch_add(1, Ordering::AcqRel);
                let (token, to_main, done) = (token.clone(), to_main.clone(), active.clone());
                let spawned = std::thread::Builder::new()
                    .name("screen-control-connection".into())
                    .spawn(move || {
                        let _ = connection(stream, &token, &to_main);
                        done.fetch_sub(1, Ordering::AcqRel);
                    });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::AcqRel);
                }
            }
        })
}

fn connection(
    stream: TcpStream,
    token: &str,
    to_main: &mpsc::SyncSender<Request>,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    // Until a request carries the token; after that a client may keep the connection idle.
    let mut deadline = Some(Instant::now() + FIRST_REQUEST);
    while let Some(line) = read_line(&mut reader, deadline)? {
        if line.trim().is_empty() {
            continue;
        }
        let Some(mut request) = serde_json::from_str::<Value>(&line)
            .ok()
            .filter(Value::is_object)
        else {
            send(&mut writer, &error("bad_request"))?;
            return Ok(());
        };
        let authentic = request["token"]
            .as_str()
            .is_some_and(|t| same_token(t, token));
        if !authentic {
            send(&mut writer, &error("bad_token"))?;
            return Ok(());
        }
        if deadline.take().is_some() {
            reader.get_ref().set_read_timeout(None)?;
        }
        if let Some(object) = request.as_object_mut() {
            object.remove("token");
        }
        if let Err(code) = check(&request) {
            send(&mut writer, &error(code))?;
            continue;
        }
        let (reply, answer) = mpsc::channel();
        if to_main
            .send(Request {
                body: request,
                reply,
            })
            .is_err()
        {
            return Ok(());
        }
        let Ok(answer) = answer.recv() else {
            return Ok(());
        };
        send(&mut writer, &answer)?;
    }
    Ok(())
}

/// What the port refuses before the main loop sees a request.
fn check(request: &Value) -> Result<(), &'static str> {
    let cmd = request["cmd"].as_str().ok_or("bad_request")?;
    if !COMMANDS.contains(&cmd) {
        return Err("unknown_command");
    }
    if cmd == "monitor" {
        let id = request["id"].as_str().ok_or("bad_request")?;
        if id.is_empty() || id.chars().count() > MAX_MONITOR_ID {
            return Err("bad_request");
        }
    }
    if cmd == "exclude" && excluded_pids(request).is_none() {
        return Err("bad_request");
    }
    if cmd == "mode" && !matches!(request["mode"].as_str(), Some("off" | "frames" | "video")) {
        return Err("bad_request");
    }
    if cmd == "mode"
        && !request["monitor"].is_null()
        && request["monitor"]
            .as_str()
            .is_none_or(|id| id.is_empty() || id.chars().count() > MAX_MONITOR_ID)
    {
        return Err("bad_request");
    }
    Ok(())
}

/// Reads one request line. With a `deadline` the whole line must arrive before it passes, so a
/// client can't keep the connection by sending a byte now and then.
fn read_line(
    reader: &mut BufReader<TcpStream>,
    deadline: Option<Instant>,
) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    loop {
        if let Some(deadline) = deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            reader.get_ref().set_read_timeout(Some(left))?;
        }
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            break;
        }
        let (taken, done) = match available.iter().position(|&b| b == b'\n') {
            Some(end) => (end + 1, true),
            None => (available.len(), false),
        };
        buf.extend_from_slice(&available[..taken]);
        reader.consume(taken);
        if buf.len() > MAX_LINE + 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request too long",
            ));
        }
        if done {
            break;
        }
    }
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.len() > MAX_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request too long",
        ));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn send(writer: &mut TcpStream, value: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes)
}

/// Compares two tokens without stopping at the first difference.
fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Sends one request and reads its answer. `body` is a JSON object; the token is added to it.
/// `timeout` bounds the connection and each of the write and the read.
pub fn request(port: u16, token: &str, body: Value, timeout: Duration) -> io::Result<Value> {
    let Value::Object(mut object) = body else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a request is a JSON object",
        ));
    };
    object.insert("token".into(), Value::String(token.to_string()));
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    send(&mut stream, &Value::Object(object))?;
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line)? == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// [`request`], with the connection, the write and the whole answer done by `deadline`: a slow
/// or silent process makes it fail with `TimedOut` then, never later. The answer may be up to
/// `MAX_LINE` long, like a request; the port's answers are a few hundred bytes.
pub fn request_until(port: u16, token: &str, body: Value, deadline: Instant) -> io::Result<Value> {
    let left = || match deadline.saturating_duration_since(Instant::now()) {
        left if left.is_zero() => Err(io::Error::from(io::ErrorKind::TimedOut)),
        left => Ok(left),
    };
    let Value::Object(mut object) = body else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a request is a JSON object",
        ));
    };
    object.insert("token".into(), Value::String(token.to_string()));
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, left()?)?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(left()?))?;
    send(&mut stream, &Value::Object(object))?;
    let line = read_line(&mut BufReader::new(stream), Some(deadline))?
        .ok_or(io::ErrorKind::UnexpectedEof)?;
    serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    /// A port served by a main loop that answers `{ok, cmd}` and records what it got.
    fn served() -> (u16, String, mpsc::Receiver<Value>) {
        let listener = bind().unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = new_token().unwrap();
        let (to_main, requests) = mpsc::sync_channel::<Request>(4);
        serve(listener, token.clone(), to_main).unwrap();
        let (seen_tx, seen) = mpsc::channel();
        std::thread::spawn(move || {
            for request in requests {
                let answer = json!({ "ok": true, "cmd": request.body["cmd"] });
                seen_tx.send(request.body).unwrap();
                let _ = request.reply.send(answer);
            }
        });
        (port, token, seen)
    }

    fn connect(port: u16) -> TcpStream {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
    }

    fn exchange(stream: &mut TcpStream, line: &str) -> Option<Value> {
        stream.write_all(line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut answer = String::new();
        match reader.read_line(&mut answer) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&answer).unwrap()),
        }
    }

    /// Whether the server closed the connection: a read returns end of stream.
    fn closed(stream: &mut TcpStream) -> bool {
        let mut buf = [0u8; 16];
        matches!(stream.read(&mut buf), Ok(0) | Err(_))
    }

    #[test]
    fn a_request_until_a_deadline_ends_by_it() {
        let (port, token, _seen) = served();
        let answer = request_until(
            port,
            &token,
            json!({ "cmd": "status" }),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(answer, json!({ "ok": true, "cmd": "status" }));

        // A main loop that never answers: the request gives up at its deadline.
        let listener = bind().unwrap();
        let port = listener.local_addr().unwrap().port();
        let (to_main, requests) = mpsc::sync_channel::<Request>(4);
        serve(listener, token.clone(), to_main).unwrap();
        let asked = Instant::now();
        let error = request_until(
            port,
            &token,
            json!({ "cmd": "monitor", "id": "\\\\.\\DISPLAY2" }),
            asked + Duration::from_millis(300),
        )
        .unwrap_err();
        let took = asked.elapsed();
        assert!(
            took >= Duration::from_millis(250) && took < Duration::from_secs(2),
            "{took:?}"
        );
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ),
            "{error:?}"
        );
        // A deadline that passed: nothing is sent.
        let error =
            request_until(port, &token, json!({ "cmd": "status" }), Instant::now()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(requests);
    }

    #[test]
    fn tokens_are_43_characters_of_base64url_and_differ() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_eq!(a.len(), 43);
        assert!(a
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        assert_ne!(a, b);
        assert!(same_token(&a, &a.clone()));
        assert!(!same_token(&a, &b));
        assert!(!same_token("abc", "abcd"));
    }

    #[test]
    fn good_requests_reach_the_main_loop_without_their_token() {
        let (port, token, seen) = served();
        let answer = request(
            port,
            &token,
            json!({ "cmd": "status" }),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(answer, json!({ "ok": true, "cmd": "status" }));
        let body = seen.recv().unwrap();
        assert_eq!(body, json!({ "cmd": "status" }));
        // Several requests on one connection, also after a pause.
        let mut stream = connect(port);
        let pause = json!({ "token": token, "cmd": "pause" }).to_string();
        assert_eq!(exchange(&mut stream, &pause).unwrap()["cmd"], "pause");
        std::thread::sleep(Duration::from_millis(50));
        let monitor = json!({ "token": token, "cmd": "monitor", "id": r"\\.\DISPLAY2" });
        assert_eq!(
            exchange(&mut stream, &monitor.to_string()).unwrap()["cmd"],
            "monitor"
        );
        seen.recv().unwrap();
        assert_eq!(seen.recv().unwrap()["id"], r"\\.\DISPLAY2");
        assert!(request(port, &token, json!("status"), Duration::from_secs(1)).is_err());
    }

    #[test]
    fn a_wrong_or_missing_token_closes_the_connection() {
        let (port, token, seen) = served();
        for line in [
            json!({ "cmd": "status" }).to_string(),
            json!({ "token": "x", "cmd": "status" }).to_string(),
            json!({ "token": format!("{token}x"), "cmd": "stop" }).to_string(),
        ] {
            let mut stream = connect(port);
            assert_eq!(exchange(&mut stream, &line), Some(error("bad_token")));
            assert!(closed(&mut stream));
        }
        let answer = request(
            port,
            "wrong",
            json!({ "cmd": "stop" }),
            Duration::from_secs(5),
        );
        assert_eq!(answer.unwrap(), error("bad_token"));
        assert!(
            seen.try_recv().is_err(),
            "a bad token reached the main loop"
        );
    }

    #[test]
    fn bad_requests_are_answered_by_the_port() {
        let (port, token, seen) = served();
        let mut stream = connect(port);
        let with = |extra: Value| {
            let mut o = json!({ "token": token });
            o.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            o.to_string()
        };
        for (line, code) in [
            (with(json!({})), "bad_request"),
            (with(json!({ "cmd": 5 })), "bad_request"),
            (with(json!({ "cmd": "explode" })), "unknown_command"),
            (with(json!({ "cmd": "monitor" })), "bad_request"),
            (with(json!({ "cmd": "monitor", "id": "" })), "bad_request"),
            (
                with(json!({ "cmd": "monitor", "id": "x".repeat(MAX_MONITOR_ID + 1) })),
                "bad_request",
            ),
            (with(json!({ "cmd": "exclude" })), "bad_request"),
            (
                with(json!({ "cmd": "exclude", "pids": [0] })),
                "bad_request",
            ),
            (
                with(json!({ "cmd": "exclude", "pids": ["4821"] })),
                "bad_request",
            ),
            (
                with(json!({ "cmd": "exclude", "pids": [-1] })),
                "bad_request",
            ),
            (
                with(json!({ "cmd": "exclude", "pids": vec![7; MAX_EXCLUDED + 1] })),
                "bad_request",
            ),
        ] {
            assert_eq!(exchange(&mut stream, &line), Some(error(code)), "{line}");
        }
        // The connection stays open after those.
        let status = with(json!({ "cmd": "status" }));
        assert_eq!(exchange(&mut stream, &status).unwrap()["ok"], true);
        assert_eq!(seen.recv().unwrap()["cmd"], "status");
        assert!(seen.try_recv().is_err());
        // A line that isn't a JSON object is answered and closes it.
        assert_eq!(exchange(&mut stream, "[1, 2]"), Some(error("bad_request")));
        assert!(closed(&mut stream));
    }

    #[test]
    fn exclude_takes_a_short_list_of_pids() {
        assert_eq!(
            excluded_pids(&json!({ "pids": [4821, 77, 4821] })),
            Some(vec![77, 4821])
        );
        assert_eq!(excluded_pids(&json!({ "pids": [] })), Some(vec![]));
        assert_eq!(
            excluded_pids(&json!({ "pids": [u64::from(u32::MAX) + 1] })),
            None
        );
        assert_eq!(excluded_pids(&json!({ "pids": 4821 })), None);
        assert_eq!(excluded_pids(&json!({ "pids": [1.5] })), None);
        assert_eq!(
            excluded_pids(&json!({ "pids": vec![9; MAX_EXCLUDED] })),
            Some(vec![9])
        );
    }

    #[test]
    fn lines_over_64_kib_close_the_connection() {
        let (port, token, seen) = served();
        let mut stream = connect(port);
        // Just under the limit is a request like any other.
        let mut fits = json!({ "token": token, "cmd": "status" }).to_string();
        fits.insert_str(1, &" ".repeat(MAX_LINE - fits.len()));
        assert_eq!(fits.len(), MAX_LINE);
        assert_eq!(exchange(&mut stream, &fits).unwrap()["ok"], true);
        seen.recv().unwrap();
        // One byte more closes it unanswered.
        let mut over = json!({ "token": token, "cmd": "status" }).to_string();
        over.insert_str(1, &" ".repeat(MAX_LINE + 1 - over.len()));
        assert_eq!(exchange(&mut stream, &over), None);
        assert!(seen.try_recv().is_err());
    }

    #[test]
    fn a_silent_connection_is_closed_after_the_first_request_time() {
        let (port, _token, _seen) = served();
        let mut stream = connect(port);
        // Half a request doesn't keep it.
        stream.write_all(b"{").unwrap();
        let started = Instant::now();
        assert!(closed(&mut stream));
        let waited = started.elapsed();
        assert!(
            waited >= FIRST_REQUEST - Duration::from_millis(100) && waited < FIRST_REQUEST * 2,
            "{waited:?}"
        );
    }

    #[test]
    fn connections_beyond_the_limit_are_closed() {
        let (port, token, _seen) = served();
        let mut open: Vec<TcpStream> = (0..MAX_CONNECTIONS).map(|_| connect(port)).collect();
        let status = json!({ "token": token, "cmd": "status" }).to_string();
        for stream in &mut open {
            assert_eq!(exchange(stream, &status).unwrap()["ok"], true);
        }
        let mut extra = connect(port);
        assert!(closed(&mut extra));
        drop(open);
        // Slots come free once their clients close.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut again = connect(port);
            if exchange(&mut again, &status).is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "no slot came free");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
