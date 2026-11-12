//! Helpers shared by the integration tests: temporary folders, a fake `MeetingControl` and a
//! synchronous WebSocket client.

#![allow(dead_code)]

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message as Frame, WebSocket};
use voice_bridge::{
    BridgeServer, ControlError, EventLog, Handle, MediaKind, MediaOpenArgs, MeetingControl,
    MeetingSummary, Options, StartArgs, Status,
};
use voice_protocol::{
    media_read_result, validate_discovery, validate_incoming, MeetingId, MeetingState, Message,
    Side,
};

/// A meeting the fake knows, which isn't recording.
pub const DONE_MEETING: &str = "m-20261030-090000-00ff";
/// The id the fake gives a meeting it starts.
pub const NEW_MEETING: &str = "m-20261030-171203-4f2a";
/// The id the fake gives every audio or frame it opens.
pub const FAKE_MEDIA: &str = "md-a1B2c3D4e5F6g7H8i9J0kL";
/// The pid the test client's `hello` sends, as a development Rukoo does.
pub const RUKOO_PID: u32 = 4821;
/// The length of the fake's media: two full reads and a short third one.
pub const FAKE_MEDIA_SIZE: u64 = (2 << 20) + 100;

/// The fake's media from `offset`, at most `length` bytes: byte `i` is `i mod 251`.
pub fn fake_media(offset: u64, length: u64) -> Vec<u8> {
    let end = offset.saturating_add(length).min(FAKE_MEDIA_SIZE);
    (offset.min(end)..end).map(|i| (i % 251) as u8).collect()
}

/// No test waits longer than this for the server.
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// A folder under the system temp folder, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "voice-bridge-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A `MeetingControl` that records what it was asked and answers like a small Voice.
#[derive(Default)]
pub struct FakeControl {
    pub recording: Mutex<Option<String>>,
    pub paused: Mutex<bool>,
    pub started: Mutex<Vec<StartArgs>>,
    pub calls: Mutex<Vec<String>>,
    /// A mark labelled "wait" blocks until the test sends on this gate's other end.
    pub gate: Mutex<Option<mpsc::Receiver<()>>>,
    /// What `clip.get` answers for a known meeting; `None` answers `not_available`.
    pub clips: Mutex<Option<Value>>,
    /// The deadline of every `clip.get`, in the order they ran.
    pub deadlines: Mutex<Vec<Instant>>,
    /// `clip.get` takes until its deadline, like a cut that isn't finished in time.
    pub slow_clips: AtomicBool,
    /// What `screen.get` answers for the recording meeting; `None` answers `not_available`.
    /// `screen.select` knows the ids in its `monitors`.
    pub screen: Mutex<Option<Value>>,
    /// The deadline of every `screen.get` and `screen.select`, in the order they ran.
    pub screen_deadlines: Mutex<Vec<Instant>>,
}

impl FakeControl {
    fn call(&self, what: String) {
        self.calls.lock().unwrap().push(what);
    }

    fn known(&self, id: &str) -> Result<(), ControlError> {
        let recording = self.recording.lock().unwrap();
        if id == DONE_MEETING || recording.as_deref() == Some(id) {
            Ok(())
        } else {
            Err(ControlError::not_found(format!("No meeting {id}.")))
        }
    }

    fn recording(&self, id: &str) -> Result<(), ControlError> {
        self.known(id)?;
        if self.recording.lock().unwrap().as_deref() == Some(id) {
            Ok(())
        } else {
            Err(ControlError::not_recording("The meeting isn't recording."))
        }
    }
}

impl MeetingControl for FakeControl {
    fn start(&self, args: StartArgs) -> Result<String, ControlError> {
        self.call("start".into());
        let mut recording = self.recording.lock().unwrap();
        if let Some(meeting) = recording.as_ref() {
            return Err(ControlError::busy(
                meeting.clone(),
                "A meeting is already recording.",
            ));
        }
        self.started.lock().unwrap().push(args);
        *recording = Some(NEW_MEETING.into());
        Ok(NEW_MEETING.into())
    }

    fn stop(&self, id: &str) -> Result<(), ControlError> {
        self.call(format!("stop {id}"));
        self.recording(id)?;
        *self.recording.lock().unwrap() = None;
        Ok(())
    }

    fn pause(&self, id: &str) -> Result<(), ControlError> {
        self.call(format!("pause {id}"));
        self.recording(id)?;
        *self.paused.lock().unwrap() = true;
        Ok(())
    }

    fn resume(&self, id: &str) -> Result<(), ControlError> {
        self.call(format!("resume {id}"));
        self.recording(id)?;
        let mut paused = self.paused.lock().unwrap();
        if !*paused {
            return Err(ControlError::not_recording("The meeting isn't paused."));
        }
        *paused = false;
        Ok(())
    }

    fn mark(&self, id: &str, label: Option<&str>) -> Result<(String, u64), ControlError> {
        self.call(format!("mark {id} {label:?}"));
        if label == Some("panic") {
            panic!("the fake was asked to panic");
        }
        if label == Some("slow") {
            std::thread::sleep(Duration::from_millis(300));
        }
        if label == Some("wait") {
            let gate = self
                .gate
                .lock()
                .unwrap()
                .take()
                .expect("a gate for the wait");
            gate.recv_timeout(READ_TIMEOUT)
                .expect("the test opens the gate");
        }
        self.recording(id)?;
        Ok(("k-1".into(), 4_200))
    }

    fn list(&self, limit: usize) -> Vec<MeetingSummary> {
        self.call(format!("list {limit}"));
        let done = MeetingSummary {
            id: MeetingId::parse(DONE_MEETING).unwrap(),
            title: "Standup".into(),
            started_at: 1_792_998_000_000,
            ended_at: Some(1_792_998_900_000),
            state: MeetingState::Done,
            duration: 900_000,
        };
        vec![done; limit.min(2)]
    }

    fn get(&self, id: &str) -> Result<Value, ControlError> {
        self.call(format!("get {id}"));
        self.known(id)?;
        Ok(
            json!({ "meeting": { "id": id, "state": "done" }, "speakers": [], "passages": [], "marks": [], "head": 0 }),
        )
    }

    fn frame(&self, id: &str, at: u64) -> Result<Value, ControlError> {
        self.call(format!("frame {id} {at}"));
        self.known(id)?;
        Err(ControlError::not_available(
            "The meeting didn't record the screen.",
        ))
    }

    fn clip(
        &self,
        id: &str,
        start: u64,
        end: u64,
        deadline: Instant,
    ) -> Result<Value, ControlError> {
        self.call(format!("clip {id} {start} {end}"));
        self.deadlines.lock().unwrap().push(deadline);
        if self.slow_clips.load(Ordering::SeqCst) {
            // A loop, because a sleep may end a little early.
            loop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                std::thread::sleep(deadline - now);
            }
        }
        self.known(id)?;
        self.clips
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| ControlError::not_available("The meeting didn't record video."))
    }

    fn protected(&self, id: &str) -> Result<(), ControlError> {
        self.call(format!("protected {id}"));
        self.known(id)
    }

    fn screen(&self, id: &str, deadline: Instant) -> Result<Value, ControlError> {
        self.call(format!("screen {id}"));
        self.screen_deadlines.lock().unwrap().push(deadline);
        self.recording(id)?;
        self.screen
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| ControlError::not_available("The meeting records no screen."))
    }

    fn select_monitor(
        &self,
        id: &str,
        monitor: &str,
        deadline: Instant,
    ) -> Result<String, ControlError> {
        self.call(format!("select {id} {monitor}"));
        self.screen_deadlines.lock().unwrap().push(deadline);
        self.recording(id)?;
        let screen = self.screen.lock().unwrap().clone();
        let Some(screen) = screen else {
            return Err(ControlError::not_available(
                "The meeting records no screen.",
            ));
        };
        let known = screen["monitors"]
            .as_array()
            .is_some_and(|all| all.iter().any(|m| m["id"] == monitor));
        if known {
            Ok(monitor.to_owned())
        } else {
            Err(ControlError::not_found(format!("No monitor {monitor}.")))
        }
    }

    /// Audio and frames are ready at once as [`FAKE_MEDIA`]; the fake records no video.
    fn media_open(&self, args: &MediaOpenArgs) -> Result<Value, ControlError> {
        self.call(format!(
            "media.open {} {} {} {:?}",
            args.meeting, args.kind, args.start, args.end
        ));
        self.known(args.meeting.as_str())?;
        let kind = match args.kind {
            MediaKind::Audio => "audio/wav",
            MediaKind::Frame => "image/jpeg",
            MediaKind::Video => {
                return Err(ControlError::not_available(
                    "This meeting didn't record video.",
                ))
            }
        };
        Ok(json!({
            "media": FAKE_MEDIA,
            "ready": true,
            "type": kind,
            "size": FAKE_MEDIA_SIZE,
            "start": args.start,
            "end": args.end.unwrap_or(args.start),
        }))
    }

    fn media_read(&self, media: &str, offset: u64, length: u64) -> Result<Value, ControlError> {
        self.call(format!("media.read {media} {offset} {length}"));
        if media != FAKE_MEDIA {
            return Err(ControlError::not_found("There is no media with that id."));
        }
        Ok(media_read_result(offset, &fake_media(offset, length)))
    }

    fn status(&self) -> Status {
        let recording = self.recording.lock().unwrap();
        Status {
            recording: recording.as_deref().and_then(MeetingId::parse),
            dictating: false,
            screen_default: None,
        }
    }
}

/// The fake behind a Voice that predates the commands with default bodies: it passes the required
/// methods on and leaves the rest to the trait, so the server answers them as an older Voice would.
pub struct PlainControl(pub Arc<FakeControl>);

impl MeetingControl for PlainControl {
    fn start(&self, args: StartArgs) -> Result<String, ControlError> {
        self.0.start(args)
    }

    fn stop(&self, id: &str) -> Result<(), ControlError> {
        self.0.stop(id)
    }

    fn pause(&self, id: &str) -> Result<(), ControlError> {
        self.0.pause(id)
    }

    fn resume(&self, id: &str) -> Result<(), ControlError> {
        self.0.resume(id)
    }

    fn mark(&self, id: &str, label: Option<&str>) -> Result<(String, u64), ControlError> {
        self.0.mark(id, label)
    }

    fn list(&self, limit: usize) -> Vec<MeetingSummary> {
        self.0.list(limit)
    }

    fn get(&self, id: &str) -> Result<Value, ControlError> {
        self.0.get(id)
    }

    fn frame(&self, id: &str, at: u64) -> Result<Value, ControlError> {
        self.0.frame(id, at)
    }

    fn status(&self) -> Status {
        self.0.status()
    }
}

/// A started server with its log, fake control and discovery file.
pub struct Bridge {
    pub handle: Option<Handle>,
    pub log: EventLog,
    pub control: Arc<FakeControl>,
    pub discovery: voice_protocol::Discovery,
    pub discovery_path: PathBuf,
    pub dir: TempDir,
}

impl Bridge {
    pub fn start(tag: &str) -> Bridge {
        Bridge::start_with(tag, Options::default())
    }

    pub fn start_with(tag: &str, options: Options) -> Bridge {
        let dir = TempDir::new(tag);
        let log = EventLog::open(dir.path()).unwrap();
        Bridge::start_on(dir, log, options)
    }

    pub fn start_on(dir: TempDir, log: EventLog, options: Options) -> Bridge {
        Bridge::serve(dir, log, options, |fake| fake)
    }

    /// A server whose control keeps the trait's default bodies, as a Voice built before those
    /// commands existed. `control` still records the calls that reach the fake.
    pub fn start_plain(tag: &str) -> Bridge {
        let dir = TempDir::new(tag);
        let log = EventLog::open(dir.path()).unwrap();
        Bridge::serve(dir, log, Options::default(), |fake| {
            Arc::new(PlainControl(fake))
        })
    }

    fn serve(
        dir: TempDir,
        log: EventLog,
        options: Options,
        wrap: impl FnOnce(Arc<FakeControl>) -> Arc<dyn MeetingControl>,
    ) -> Bridge {
        let control = Arc::new(FakeControl::default());
        let discovery_path = dir.path().join("pair").join("bridge.json");
        let handle = BridgeServer::start_with(
            log.clone(),
            wrap(control.clone()),
            &discovery_path,
            "0.1.0",
            options,
        )
        .unwrap();
        let discovery = read_discovery(&discovery_path);
        assert_eq!(discovery.port, handle.port());
        Bridge {
            handle: Some(handle),
            log,
            control,
            discovery,
            discovery_path,
            dir,
        }
    }

    pub fn port(&self) -> u16 {
        self.discovery.port
    }

    pub fn token(&self) -> &str {
        &self.discovery.token
    }

    /// A client that sent `hello` and read `welcome`.
    pub fn client(&self, instance: Option<&str>, after: u64) -> (Client, Value) {
        let mut client = Client::connect(self.port());
        client.hello(self.token(), instance, after);
        let welcome = client.read();
        assert_eq!(welcome["type"], "welcome", "{welcome}");
        (client, welcome)
    }

    pub fn shutdown(&mut self) {
        self.handle.take().unwrap().shutdown();
    }
}

pub fn read_discovery(path: &Path) -> voice_protocol::Discovery {
    let text = fs::read_to_string(path).unwrap();
    validate_discovery(&serde_json::from_str(&text).unwrap()).unwrap()
}

/// A synchronous WebSocket client, as a test plays Rukoo.
pub struct Client {
    pub socket: WebSocket<MaybeTlsStream<TcpStream>>,
}

impl Client {
    pub fn connect(port: u16) -> Client {
        Client::try_connect(port, "/bridge", &[]).unwrap()
    }

    pub fn try_connect(
        port: u16,
        path: &str,
        headers: &[(&'static str, &str)],
    ) -> Result<Client, tungstenite::Error> {
        let mut request = format!("ws://127.0.0.1:{port}{path}")
            .into_client_request()
            .unwrap();
        for (name, value) in headers {
            request.headers_mut().insert(*name, value.parse().unwrap());
        }
        let (socket, _) = tungstenite::connect(request)?;
        if let MaybeTlsStream::Plain(tcp) = socket.get_ref() {
            tcp.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
        }
        Ok(Client { socket })
    }

    pub fn send(&mut self, message: Value) {
        self.send_text(message.to_string());
    }

    pub fn send_text(&mut self, text: String) {
        self.socket.send(Frame::text(text)).unwrap();
    }

    /// Sends without unwrapping, for a message the server may close the connection on while it is still written.
    pub fn try_send_text(&mut self, text: String) -> tungstenite::Result<()> {
        self.socket.send(Frame::text(text))
    }

    pub fn hello(&mut self, token: &str, instance: Option<&str>, after: u64) {
        self.send(json!({ "type": "hello", "protocol": voice_protocol::PROTOCOL, "token": token, "client": "rukoo", "clientVersion": "2.3.0", "pid": RUKOO_PID, "instance": instance, "after": after }));
    }

    pub fn command(&mut self, id: &str, command: &str, args: Value) {
        self.send(json!({ "type": "command", "id": id, "command": command, "args": args }));
    }

    /// The next message from Voice, which must pass Rukoo's checks.
    pub fn read(&mut self) -> Value {
        loop {
            match self.socket.read().expect("a message from Voice") {
                Frame::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if let Err(invalid) = validate_incoming(Side::Voice, &value) {
                        panic!("Voice sent an invalid message ({invalid}): {value}");
                    }
                    return value;
                }
                Frame::Close(frame) => panic!("closed while waiting for a message: {frame:?}"),
                _ => continue,
            }
        }
    }

    /// The next message from Voice, typed.
    pub fn read_message(&mut self) -> Message {
        validate_incoming(Side::Voice, &self.read()).unwrap()
    }

    /// The next reply, which must answer `id`.
    pub fn read_reply(&mut self, id: &str) -> Value {
        let reply = self.read();
        assert_eq!(reply["type"], "reply", "{reply}");
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }

    /// Reads events until `seq` arrives and returns every seq read.
    pub fn read_events_until(&mut self, seq: u64) -> Vec<u64> {
        let mut seqs = Vec::new();
        loop {
            let event = self.read();
            assert_eq!(event["type"], "event", "{event}");
            let got = event["seq"].as_u64().unwrap();
            seqs.push(got);
            if got >= seq {
                return seqs;
            }
        }
    }

    /// Reads until the server closes, and returns the close code, or `None` when the connection
    /// ended without a close frame.
    pub fn read_close(&mut self) -> Option<u16> {
        loop {
            match self.socket.read() {
                Ok(Frame::Close(frame)) => return frame.map(|f| u16::from(f.code)),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    /// Reads the `error` message and the close that follows it.
    pub fn read_refusal(&mut self) -> (String, Option<u16>) {
        let error = self.read();
        assert_eq!(error["type"], "error", "{error}");
        (
            error["code"].as_str().unwrap().to_owned(),
            self.read_close(),
        )
    }
}

/// The data of a valid passage on the microphone that starts at `start`.
pub fn passage(start: u64) -> Value {
    json!({ "id": format!("mic-{start:09}"), "track": "mic", "speaker": "mic", "start": start, "end": start + 500, "text": "hello", "revision": 1, "dictation": false, "language": "en" })
}
