//! The control port: newline-delimited JSON on 127.0.0.1, every request carrying the token.
//!
//! Connection threads only parse and authenticate; the recorder thread runs every command, so
//! the timeline keeps a single writer.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::record::Event;
use crate::token::same_token;

/// Longest request line; anything longer closes the connection.
const MAX_REQUEST: usize = 64 * 1024;
/// Longest mark label, in characters.
const MAX_LABEL: usize = 1000;
/// Simultaneous connections; more are closed at once.
const MAX_CONNECTIONS: usize = 16;
/// Tap blocks of 10 ms queued for a slow reader before it's disconnected.
const TAP_QUEUE: usize = 512;
/// A connection must send its first request with the token this soon, so connections that
/// never authenticate can't hold the slots.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
/// How often a tap looks whether its client is still there, also while the track sends nothing.
const PEER_CHECK: Duration = Duration::from_millis(250);
/// About the audio `TAP_QUEUE` holds.
const TAP_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// A command the recorder thread runs.
pub enum Command {
    Status,
    Stop {
        reason: String,
    },
    Pause,
    Resume,
    Mark {
        label: Option<String>,
    },
    Dictation {
        state: String,
    },
    Tap {
        track: String,
        pcm: SyncSender<Vec<u8>>,
    },
}

/// The recorder's answer. `written` is signalled once the reply is on the socket, so the
/// recorder doesn't exit before a `stop` reply reaches its client.
pub struct Reply {
    pub value: Value,
    pub written: Option<Sender<()>>,
}

impl Reply {
    pub fn ok(value: Value) -> Self {
        Self {
            value,
            written: None,
        }
    }

    pub fn error(code: &str) -> Self {
        Self::ok(error(code))
    }
}

fn error(code: &str) -> Value {
    json!({ "ok": false, "error": code })
}

/// Listens on `127.0.0.1:0`.
pub fn bind() -> io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
}

pub(crate) fn spawn(
    listener: TcpListener,
    token: String,
    events: SyncSender<Event>,
) -> io::Result<()> {
    let token: Arc<str> = token.into();
    std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue;
                }
                active.fetch_add(1, Ordering::Relaxed);
                let (token, events, done) = (token.clone(), events.clone(), active.clone());
                let spawned = std::thread::Builder::new()
                    .name("control-connection".into())
                    .spawn(move || {
                        let _ = serve(stream, &token, &events);
                        done.fetch_sub(1, Ordering::Relaxed);
                    });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::Relaxed);
                }
            }
        })?;
    Ok(())
}

fn serve(stream: TcpStream, token: &str, events: &SyncSender<Event>) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    // Until a request carries the token; after that a client may keep the connection idle.
    let mut deadline = Some(Instant::now() + AUTH_TIMEOUT);
    while let Some(line) = read_line(&mut reader, deadline)? {
        if line.trim().is_empty() {
            continue;
        }
        let Some(request) = serde_json::from_str::<Value>(&line)
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
        let (command, tap) = match parse(&request) {
            Ok(parsed) => parsed,
            Err(code) => {
                send(&mut writer, &error(code))?;
                continue;
            }
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        if events
            .send(Event::Command {
                command,
                reply: reply_tx,
                at: Instant::now(),
            })
            .is_err()
        {
            return Ok(());
        }
        let Ok(reply) = reply_rx.recv() else {
            return Ok(());
        };
        let sent = send(&mut writer, &reply.value);
        if let Some(written) = reply.written {
            let _ = written.send(());
        }
        sent?;
        if let Some(pcm) = tap {
            if reply.value["ok"] == true {
                return stream_pcm(writer, pcm);
            }
        }
    }
    Ok(())
}

/// A command, and for `tap` the receiving end of its PCM.
type Parsed = (Command, Option<Receiver<Vec<u8>>>);

fn parse(request: &Value) -> Result<Parsed, &'static str> {
    let text = |key: &str| request[key].as_str().map(str::to_string);
    let command = match request["cmd"].as_str().ok_or("bad_request")? {
        "status" => Command::Status,
        "stop" => {
            let reason = text("reason").unwrap_or_else(|| "user".into());
            if reason != "user" && reason != "rukoo" {
                return Err("bad_request");
            }
            Command::Stop { reason }
        }
        "pause" => Command::Pause,
        "resume" => Command::Resume,
        "mark" => {
            let label = match &request["label"] {
                Value::Null => None,
                Value::String(s) if s.chars().count() <= MAX_LABEL => Some(s.clone()),
                _ => return Err("bad_request"),
            };
            Command::Mark { label }
        }
        "dictation" => {
            let state = text("state").ok_or("bad_request")?;
            if state != "start" && state != "end" {
                return Err("bad_request");
            }
            Command::Dictation { state }
        }
        "tap" => {
            let track = text("track").ok_or("bad_request")?;
            let (pcm, rx) = mpsc::sync_channel(TAP_QUEUE);
            return Ok((Command::Tap { track, pcm }, Some(rx)));
        }
        _ => return Err("unknown_command"),
    };
    Ok((command, None))
}

/// Writes tap PCM until the client goes away or the recorder drops the tap. A track without
/// audio, such as a silent loopback, sends nothing for a long time, so the client is checked on
/// a timer too: its slot must come free when it closes.
fn stream_pcm(mut writer: TcpStream, pcm: Receiver<Vec<u8>>) -> io::Result<()> {
    // A client that stays connected but stops reading is as far behind as one the recorder
    // disconnects for its full queue.
    writer.set_write_timeout(Some(TAP_WRITE_TIMEOUT))?;
    let mut next_check = Instant::now() + PEER_CHECK;
    loop {
        match pcm.recv_timeout(PEER_CHECK) {
            Ok(block) => writer.write_all(&block)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if Instant::now() >= next_check {
            if peer_closed(&writer)? {
                return Ok(());
            }
            next_check = Instant::now() + PEER_CHECK;
        }
    }
}

/// Whether the client closed its end. It sends nothing after `tap`; whatever it does send is
/// read and dropped, so an end of stream behind it is still found.
fn peer_closed(stream: &TcpStream) -> io::Result<bool> {
    stream.set_nonblocking(true)?;
    let mut buf = [0u8; 512];
    let mut reader = stream;
    let closed = loop {
        match reader.read(&mut buf) {
            Ok(0) => break true,
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break false,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break true,
        }
    };
    stream.set_nonblocking(false)?;
    Ok(closed)
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
        if buf.len() > MAX_REQUEST || done {
            break;
        }
    }
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.len() > MAX_REQUEST {
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

/// A client for the control port, used by the live-recorder check and by tests.
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Client {
    pub fn connect(port: u16, timeout: Duration) -> io::Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_read_timeout(Some(timeout.max(Duration::from_secs(2))))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        })
    }

    /// Sends one request and reads its reply line.
    pub fn request(&mut self, request: &Value) -> io::Result<Value> {
        send(&mut self.writer, request)?;
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        serde_json::from_str(&line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// After a successful `tap`, the rest of the connection is raw PCM: s16le, 16 kHz, mono.
    pub fn into_pcm(self) -> BufReader<TcpStream> {
        self.reader
    }

    /// Sets how long a reply may take. Replies are read through a clone of the socket, and on
    /// Windows a clone has a read timeout of its own, so both handles get it.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.reader.get_ref().set_read_timeout(timeout)?;
        self.writer.set_read_timeout(timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(request: Value) -> Command {
        parse(&request).ok().unwrap().0
    }

    #[test]
    fn parses_commands_and_their_arguments() {
        assert!(matches!(
            parse_ok(json!({"cmd": "status"})),
            Command::Status
        ));
        assert!(
            matches!(parse_ok(json!({"cmd": "stop"})), Command::Stop { reason } if reason == "user")
        );
        assert!(
            matches!(parse_ok(json!({"cmd": "stop", "reason": "rukoo"})), Command::Stop { reason } if reason == "rukoo")
        );
        assert!(matches!(
            parse_ok(json!({"cmd": "mark", "label": "Decision"})),
            Command::Mark { label: Some(l) } if l == "Decision"
        ));
        assert!(matches!(
            parse_ok(json!({"cmd": "mark"})),
            Command::Mark { label: None }
        ));
        assert!(matches!(
            parse_ok(json!({"cmd": "dictation", "state": "end"})),
            Command::Dictation { state } if state == "end"
        ));
        let (command, rx) = parse(&json!({"cmd": "tap", "track": "mic"})).ok().unwrap();
        assert!(matches!(command, Command::Tap { track, .. } if track == "mic"));
        assert!(rx.is_some());
    }

    #[test]
    fn the_read_timeout_reaches_the_handle_replies_are_read_from() {
        let listener = bind().unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = Client::connect(port, Duration::from_secs(2)).unwrap();
        // A stop waits for the disk: the 2 s of `connect` must not stay on the reading handle.
        let long = Some(Duration::from_secs(15));
        client.set_read_timeout(long).unwrap();
        assert_eq!(client.reader.get_ref().read_timeout().unwrap(), long);
        assert_eq!(client.writer.read_timeout().unwrap(), long);
    }

    #[test]
    fn refuses_bad_arguments() {
        for (request, code) in [
            (json!({}), "bad_request"),
            (json!({"cmd": "stop", "reason": "limit"}), "bad_request"),
            (json!({"cmd": "dictation"}), "bad_request"),
            (json!({"cmd": "dictation", "state": "maybe"}), "bad_request"),
            (json!({"cmd": "mark", "label": 5}), "bad_request"),
            (
                json!({"cmd": "mark", "label": "x".repeat(1001)}),
                "bad_request",
            ),
            (json!({"cmd": "tap"}), "bad_request"),
            (json!({"cmd": "explode"}), "unknown_command"),
        ] {
            assert_eq!(parse(&request).err(), Some(code), "{request}");
        }
    }
}
