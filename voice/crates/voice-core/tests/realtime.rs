//! Mistral's realtime client against a loopback WebSocket server that plays Mistral. No real key
//! and no provider is ever contacted.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use tungstenite::{Message, WebSocket};
use voice_core::providers::stream::{
    streams, SpeechStream, StreamEvent, StreamSession, StreamingSpeech,
};
use voice_core::providers::{
    parse_realtime_event, realtime_url, streaming_delay_ms, streaming_speech, wav_bytes, ErrorKind,
    MistralRealtime, ProviderConfig, ProviderError, Purpose,
};

const KEY: &str = "test-key-1234567890";
const MODEL: &str = "voxtral-mini-transcribe-realtime-3";
const WAIT: Duration = Duration::from_secs(5);

fn cfg(endpoint: &str, model: &str, key: &str) -> ProviderConfig {
    ProviderConfig {
        provider: "mistral".into(),
        api_key: key.into(),
        endpoint: endpoint.into(),
        model: model.into(),
        timeout: Duration::from_secs(5),
        purpose: Purpose::Meetings,
    }
}

/// What the fake server saw during the handshake, and every message the client sent.
#[derive(Debug, Default)]
struct Seen {
    uri: String,
    authorization: Option<String>,
    messages: Vec<Value>,
}

/// The server side of one session, for the scripts below.
struct Server {
    ws: WebSocket<TcpStream>,
    seen: Seen,
}

impl Server {
    /// The next text message from the client.
    fn recv(&mut self) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            assert!(Instant::now() < deadline, "the client sent nothing");
            match self.ws.read() {
                Ok(Message::Text(text)) => {
                    let message: Value = serde_json::from_str(text.as_str()).unwrap();
                    self.seen.messages.push(message.clone());
                    return message;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => panic!("the client connection failed: {e}"),
            }
        }
    }

    fn recv_type(&mut self, kind: &str) -> Value {
        let message = self.recv();
        assert_eq!(message["type"], kind, "{message}");
        message
    }

    fn send(&mut self, message: Value) {
        self.ws.send(Message::text(message.to_string())).unwrap();
    }

    /// `session.created` and `session.updated`, as Mistral answers a good `session.update`.
    fn accept_session(&mut self) -> Value {
        let update = self.recv_type("session.update");
        self.send(json!({"type": "session.created", "session": {}}));
        self.send(json!({"type": "session.updated", "session": update["session"]}));
        update
    }

    /// Waits until the client has closed or dropped the socket.
    fn wait_for_close(&mut self) {
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            match self.ws.read() {
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Ok(Message::Close(_)) | Err(_) => return,
                Ok(_) => {}
            }
        }
    }
}

struct Fake {
    endpoint: String,
    thread: JoinHandle<Seen>,
}

impl Fake {
    /// A server that accepts one connection and runs `script` on it.
    fn start(script: impl FnOnce(&mut Server) + Send + 'static) -> Fake {
        Fake::start_with(None, script)
    }

    /// A server that refuses the handshake with this status and body.
    fn refusing(status: u16, body: impl Into<String>) -> Fake {
        Fake::start_with(Some((status, body.into())), |_| {})
    }

    fn start_with(
        refuse: Option<(u16, String)>,
        script: impl FnOnce(&mut Server) + Send + 'static,
    ) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let thread = thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut seen = Seen::default();
            // tungstenite's handshake callback fixes the error type to its `ErrorResponse`.
            #[allow(clippy::result_large_err)]
            let callback = |request: &Request, response: Response| {
                seen.uri = request.uri().to_string();
                seen.authorization = request
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                match refuse {
                    Some((status, body)) => {
                        let mut refusal = ErrorResponse::new(Some(body));
                        *refusal.status_mut() = StatusCode::from_u16(status).unwrap();
                        Err(refusal)
                    }
                    None => Ok(response),
                }
            };
            let ws = match tungstenite::accept_hdr(tcp, callback) {
                Ok(ws) => ws,
                Err(_) => return seen,
            };
            ws.get_ref()
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let mut server = Server { ws, seen };
            script(&mut server);
            server.seen
        });
        Fake {
            endpoint: format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
            thread,
        }
    }

    fn client(&self) -> MistralRealtime {
        MistralRealtime::new(&cfg(&self.endpoint, MODEL, KEY)).unwrap()
    }

    fn finish(self) -> Seen {
        self.thread.join().expect("the fake server panicked")
    }
}

/// A port nothing listens on, as in `support::closed_port`: a port freed after binding port 0
/// could go to a fake server another test starts at the same moment (#313).
fn closed_port() -> u16 {
    1
}

fn pcm(len: usize, seed: i16) -> Vec<i16> {
    (0..len)
        .map(|i| (i as i16 % 97 - 48).wrapping_mul(seed))
        .collect()
}

fn decode_audio(message: &Value) -> Vec<i16> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(message["audio"].as_str().unwrap())
        .unwrap();
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// Polls until `Done`, an error or the timeout; returns the events and the error, if any.
fn poll_all(stream: &mut dyn SpeechStream) -> (Vec<StreamEvent>, Option<ProviderError>) {
    let deadline = Instant::now() + WAIT;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match stream.poll(Duration::from_millis(100)) {
            Ok(batch) => {
                let done = batch.contains(&StreamEvent::Done);
                events.extend(batch);
                if done {
                    return (events, None);
                }
            }
            Err(error) => return (events, Some(error)),
        }
    }
    panic!("no transcription.done and no error within {WAIT:?}: {events:?}");
}

fn assert_no_key(error: &ProviderError) {
    assert!(!error.message.contains(KEY), "{error:?}");
    assert!(!format!("{error:?}").contains(KEY));
}

#[test]
fn realtime_url_maps_the_meeting_endpoint() {
    assert_eq!(
        realtime_url("https://api.mistral.ai/v1/audio/transcriptions", MODEL).unwrap(),
        "wss://api.mistral.ai/v1/audio/transcriptions/realtime?model=voxtral-mini-transcribe-realtime-3"
    );
    assert_eq!(
        realtime_url(" https://api.eu.mistral.ai/v1/audio/transcriptions/ ", " voxtral-mini-transcribe-realtime-3 ").unwrap(),
        "wss://api.eu.mistral.ai/v1/audio/transcriptions/realtime?model=voxtral-mini-transcribe-realtime-3"
    );
    // A path that already ends in /realtime keeps it once; a port stays; wss stays wss.
    assert_eq!(
        realtime_url("wss://example.com:8443/v1/audio/transcriptions/realtime", MODEL).unwrap(),
        "wss://example.com:8443/v1/audio/transcriptions/realtime?model=voxtral-mini-transcribe-realtime-3"
    );
    // A query or fragment in the endpoint is dropped; the model is the only query.
    assert_eq!(
        realtime_url(
            "https://api.mistral.ai/v1/audio/transcriptions?x=1#y",
            "a b"
        )
        .unwrap(),
        "wss://api.mistral.ai/v1/audio/transcriptions/realtime?model=a%20b"
    );
    // Loopback hosts may use a plain socket, for tests and local servers.
    assert_eq!(
        realtime_url("http://127.0.0.1:9000/v1/audio/transcriptions", MODEL).unwrap(),
        "ws://127.0.0.1:9000/v1/audio/transcriptions/realtime?model=voxtral-mini-transcribe-realtime-3"
    );
    assert_eq!(
        realtime_url("ws://localhost:9000/v1/audio/transcriptions", MODEL).unwrap(),
        "ws://localhost:9000/v1/audio/transcriptions/realtime?model=voxtral-mini-transcribe-realtime-3"
    );
    assert_eq!(
        realtime_url("http://[::1]:9000/v1", MODEL).unwrap(),
        "ws://[::1]:9000/v1/realtime?model=voxtral-mini-transcribe-realtime-3"
    );
}

#[test]
fn realtime_url_refuses_plain_sockets_to_other_hosts() {
    for endpoint in [
        "http://api.mistral.ai/v1/audio/transcriptions",
        "ws://api.mistral.ai/v1/audio/transcriptions",
        "http://192.168.1.10:8000/v1/audio/transcriptions",
        "http://0.0.0.0:8000/v1/audio/transcriptions",
    ] {
        let error = realtime_url(endpoint, MODEL).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Config, "{endpoint}");
        assert!(error.message.contains("secure connection"), "{error:?}");
    }
    for endpoint in [
        "",
        "   ",
        "ftp://api.mistral.ai/v1",
        "api.mistral.ai/v1",
        "https://",
        "https://api.mistral.ai:99999/v1",
    ] {
        let error = realtime_url(endpoint, MODEL).unwrap_err();
        assert_eq!(error.kind, ErrorKind::Config, "{endpoint:?}");
    }
    let error = realtime_url("https://api.mistral.ai/v1/audio/transcriptions", " ").unwrap_err();
    assert_eq!(error.kind, ErrorKind::Config);
}

#[test]
fn new_needs_a_key_and_never_shows_it() {
    let endpoint = "https://api.mistral.ai/v1/audio/transcriptions";
    let error = MistralRealtime::new(&cfg(endpoint, MODEL, "  ")).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Config);
    assert_eq!(
        error.message,
        "No API key set for meeting transcription. Open Settings > Meetings > Transcription and add a key."
    );
    let error = MistralRealtime::new(&cfg("", MODEL, KEY)).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Config);

    let client = MistralRealtime::new(&cfg(endpoint, MODEL, KEY)).unwrap();
    let debug = format!("{client:?}");
    assert!(!debug.contains(KEY), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}

#[test]
fn only_mistrals_realtime_model_streams() {
    let endpoint = "https://api.mistral.ai/v1/audio/transcriptions";
    let realtime = cfg(endpoint, MODEL, KEY);
    assert!(streams(&realtime));
    assert!(streaming_speech(&realtime).is_ok());

    let label = ProviderConfig {
        provider: "Mistral".into(),
        ..realtime.clone()
    };
    assert!(streaming_speech(&label).is_ok());

    for refused in [
        cfg(endpoint, "voxtral-mini-latest", KEY),
        ProviderConfig {
            provider: "openai".into(),
            ..realtime.clone()
        },
        ProviderConfig {
            provider: "orukeet".into(),
            ..realtime.clone()
        },
        ProviderConfig {
            provider: "".into(),
            ..realtime.clone()
        },
    ] {
        let error = match streaming_speech(&refused) {
            Ok(_) => panic!("{refused:?} streams"),
            Err(error) => error,
        };
        assert_eq!(error.kind, ErrorKind::Config, "{refused:?}");
    }
    // The realtime model without a key is a config error, not a stream that fails later.
    let error = match streaming_speech(&cfg(endpoint, MODEL, "")) {
        Ok(_) => panic!("streams without a key"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Config);
}

#[test]
fn delays_snap_to_values_mistral_accepts() {
    assert_eq!(streaming_delay_ms(0), 480);
    assert_eq!(streaming_delay_ms(480), 480);
    assert_eq!(streaming_delay_ms(500), 480);
    assert_eq!(streaming_delay_ms(520), 560);
    assert_eq!(streaming_delay_ms(1), 80);
    assert_eq!(streaming_delay_ms(1200), 1200);
    assert_eq!(streaming_delay_ms(1799), 1200);
    assert_eq!(streaming_delay_ms(1800), 2400);
    assert_eq!(streaming_delay_ms(u32::MAX), 2400);
    for ms in 0..5000 {
        let delay = streaming_delay_ms(ms);
        assert!(
            delay == 2400 || ((80..=1200).contains(&delay) && delay % 80 == 0),
            "{ms} -> {delay}"
        );
    }
}

#[test]
fn a_session_streams_audio_and_reads_segments() {
    let first = pcm(1600, 3);
    let second = pcm(800, -7);
    let fake = Fake::start(|server| {
        server.accept_session();
        let one = server.recv_type("input_audio.append");
        let two = server.recv_type("input_audio.append");
        server.recv_type("input_audio.end");
        assert_eq!(decode_audio(&one).len(), 1600);
        assert_eq!(decode_audio(&two).len(), 800);
        server.send(json!({"type": "transcription.segment", "text": " Hi,", "start": null, "end": null, "speaker_id": "speaker_2"}));
        server.send(json!({"type": "turn.start", "time_s": 0.5}));
        server.send(json!({"type": "transcription.segment", "text": " there.", "start": 1.25, "end": 2.5, "speaker_id": "call-1"}));
        server.send(json!({"type": "transcription.text.delta", "text": " And more"}));
        server.send(json!({"type": "transcription.text.delta", "text": ""}));
        server.send(json!({"type": "transcription.language", "language": "en"}));
        server.send(json!({"type": "transcription.done", "text": "Hi, there. And more", "segments": [], "usage": {}}));
        server.ws.close(None).unwrap();
        server.wait_for_close();
    });
    let reference = wav_bytes(&pcm(16_000 * 3, 5), 16_000);
    let session = StreamSession {
        diarize: true,
        language: Some("nl".into()),
        context_bias: vec!["Rukoo".into()],
        delay_ms: 480,
        references: vec![
            ("call-1".into(), reference.clone()),
            ("call-2".into(), reference.clone()),
        ],
    };
    let client = fake.client();
    let mut stream = client.connect(&session).unwrap();
    stream.push(&first).unwrap();
    stream.push(&[]).unwrap();
    stream.push(&second).unwrap();
    stream.end().unwrap();
    stream.end().unwrap();
    let (events, error) = poll_all(stream.as_mut());
    assert_eq!(error, None);
    assert_eq!(
        events,
        vec![
            StreamEvent::Segment {
                text: " Hi,".into(),
                speaker: Some("speaker_2".into()),
                start: None,
                end: None,
            },
            StreamEvent::Segment {
                text: " there.".into(),
                speaker: Some("call-1".into()),
                start: Some(1.25),
                end: Some(2.5),
            },
            StreamEvent::Segment {
                text: " And more".into(),
                speaker: None,
                start: None,
                end: None,
            },
            StreamEvent::Done,
        ]
    );
    // After done and the server's close, polling is quiet rather than an error.
    assert_eq!(stream.poll(Duration::from_millis(50)), Ok(Vec::new()));
    let error = stream.push(&first).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Provider);
    drop(stream);

    let seen = fake.finish();
    assert_eq!(
        seen.uri,
        "/v1/audio/transcriptions/realtime?model=voxtral-mini-transcribe-realtime-3"
    );
    assert_eq!(
        seen.authorization.as_deref(),
        Some("Bearer test-key-1234567890")
    );
    let update = &seen.messages[0];
    assert_eq!(update["type"], "session.update");
    let fields = &update["session"];
    assert_eq!(
        fields["audio_format"],
        json!({"encoding": "pcm_s16le", "sample_rate": 16000})
    );
    assert_eq!(fields["target_streaming_delay_ms"], 480);
    assert_eq!(fields["diarize"], true);
    assert_eq!(fields["language"], "nl");
    assert_eq!(fields["context_bias"], json!(["Rukoo"]));
    let references = fields["speaker_references"].as_object().unwrap();
    assert_eq!(references.len(), 2);
    for name in ["call-1", "call-2"] {
        let wav = base64::engine::general_purpose::STANDARD
            .decode(references[name].as_str().unwrap())
            .unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 16_000 * 3 * 2);
        assert_eq!(wav, reference);
    }
    let appended: Vec<Vec<i16>> = seen.messages[1..3].iter().map(decode_audio).collect();
    assert_eq!(appended, vec![first, second]);
    assert_eq!(seen.messages[3], json!({"type": "input_audio.end"}));
    assert_eq!(seen.messages.len(), 4);
}

#[test]
fn a_session_without_diarization_sends_no_references() {
    let fake = Fake::start(|server| {
        server.accept_session();
        server.wait_for_close();
    });
    let session = StreamSession {
        diarize: false,
        language: Some("auto".into()),
        references: vec![("call-1".into(), wav_bytes(&pcm(48_000, 1), 16_000))],
        ..StreamSession::default()
    };
    let stream = fake.client().connect(&session).unwrap();
    drop(stream);
    let seen = fake.finish();
    let fields = &seen.messages[0]["session"];
    assert_eq!(fields["diarize"], false);
    assert_eq!(fields["target_streaming_delay_ms"], 480);
    assert!(fields.get("speaker_references").is_none(), "{fields}");
    assert!(fields.get("language").is_none(), "{fields}");
    assert!(fields.get("context_bias").is_none(), "{fields}");
}

#[test]
fn an_error_event_becomes_a_provider_error_after_the_segments_before_it() {
    let fake = Fake::start(|server| {
        server.accept_session();
        server.recv_type("input_audio.append");
        server.send(json!({"type": "transcription.segment", "text": " One", "start": null, "end": null, "speaker_id": "speaker_1"}));
        server.send(json!({"type": "error", "error": {"message": "Audio is too fast for this session.", "code": 400}}));
        server.ws.close(None).unwrap();
        server.wait_for_close();
    });
    let mut stream = fake.client().connect(&StreamSession::default()).unwrap();
    stream.push(&pcm(1600, 2)).unwrap();
    let (events, error) = poll_all(stream.as_mut());
    assert_eq!(
        events,
        vec![StreamEvent::Segment {
            text: " One".into(),
            speaker: Some("speaker_1".into()),
            start: None,
            end: None,
        }]
    );
    let error = error.expect("an error");
    assert_eq!(error.kind, ErrorKind::Provider);
    assert_eq!(
        error.message,
        "API error 400: Audio is too fast for this session."
    );
    // The failure sticks: every later call says the same.
    assert_eq!(stream.push(&pcm(10, 1)).unwrap_err(), error);
    assert_eq!(stream.poll(Duration::from_millis(10)).unwrap_err(), error);
    assert_eq!(stream.end().unwrap_err(), error);
    drop(stream);
    fake.finish();
}

#[test]
fn a_refused_session_update_fails_the_connect() {
    let fake = Fake::start(|server| {
        server.recv_type("session.update");
        server.send(json!({"type": "session.created", "session": {}}));
        server.send(json!({"type": "error", "error": {"message": "Speaker references require diarization, set diarize=true.", "code": 400}}));
        server.wait_for_close();
    });
    let error = match fake.client().connect(&StreamSession::default()) {
        Ok(_) => panic!("connected"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Provider);
    assert_eq!(
        error.message,
        "API error 400: Speaker references require diarization, set diarize=true."
    );
    fake.finish();
}

#[test]
fn a_refused_handshake_names_the_status() {
    let fake = Fake::refusing(401, r#"{"message":"Unauthorized","request_id":"abc"}"#);
    let error = match fake.client().connect(&StreamSession::default()) {
        Ok(_) => panic!("connected"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Auth);
    assert_eq!(error.message, "API error 401: Unauthorized");
    assert_no_key(&error);
    let seen = fake.finish();
    assert_eq!(
        seen.authorization.as_deref(),
        Some("Bearer test-key-1234567890")
    );

    let fake = Fake::refusing(400, r#"{"error":{"message":"Invalid model: voxtral-x"}}"#);
    let error = match fake.client().connect(&StreamSession::default()) {
        Ok(_) => panic!("connected"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Provider);
    assert_eq!(error.message, "API error 400: Invalid model: voxtral-x");
    assert_no_key(&error);
    fake.finish();
}

#[test]
fn a_key_the_server_echoes_never_reaches_an_error() {
    // In the handshake refusal.
    let fake = Fake::refusing(401, format!(r#"{{"message":"Invalid API key: {KEY}"}}"#));
    let error = match fake.client().connect(&StreamSession::default()) {
        Ok(_) => panic!("connected"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Auth);
    assert_eq!(error.message, "API error 401: Invalid API key: <redacted>");
    assert_no_key(&error);
    fake.finish();

    // In an error event during the session: every later call returns the scrubbed error too.
    let fake = Fake::start(|server| {
        server.accept_session();
        server.recv_type("input_audio.append");
        server.send(json!({"type": "error", "error": {"message": format!("Key {KEY} was revoked."), "code": 401}}));
        server.wait_for_close();
    });
    let mut stream = fake.client().connect(&StreamSession::default()).unwrap();
    stream.push(&pcm(1600, 2)).unwrap();
    let (events, error) = poll_all(stream.as_mut());
    assert!(events.is_empty(), "{events:?}");
    let error = error.expect("an error");
    assert_eq!(error.kind, ErrorKind::Auth);
    assert_eq!(error.message, "API error 401: Key <redacted> was revoked.");
    assert_no_key(&error);
    assert_no_key(&stream.push(&pcm(10, 1)).unwrap_err());
    assert_no_key(&stream.end().unwrap_err());
    drop(stream);
    fake.finish();

    // In the reason of a close frame.
    let fake = Fake::start(|server| {
        server.accept_session();
        server.recv_type("input_audio.append");
        server
            .ws
            .close(Some(CloseFrame {
                code: CloseCode::Policy,
                reason: format!("bad key {KEY}").into(),
            }))
            .unwrap();
        server.wait_for_close();
    });
    let mut stream = fake.client().connect(&StreamSession::default()).unwrap();
    stream.push(&pcm(1600, 2)).unwrap();
    let (_, error) = poll_all(stream.as_mut());
    let error = error.expect("an error");
    assert_eq!(
        error.message,
        "Mistral closed the stream: bad key <redacted>"
    );
    assert_no_key(&error);
    drop(stream);
    fake.finish();
}

#[test]
fn a_huge_wait_is_a_long_wait_not_a_panic() {
    let fake = Fake::start(|server| {
        server.accept_session();
        server.recv_type("input_audio.append");
        server.send(json!({"type": "transcription.done", "text": "", "segments": []}));
        server.wait_for_close();
    });
    let client = fake.client().with_timeout(Duration::MAX);
    let mut stream = client.open(&StreamSession::default()).unwrap();
    let kinds: Vec<String> = stream
        .poll_raw(Duration::ZERO)
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(kinds, ["session.created", "session.updated"]);
    stream.push(&pcm(160, 1)).unwrap();
    // Returns as soon as the first event arrives.
    assert_eq!(stream.poll(Duration::MAX), Ok(vec![StreamEvent::Done]));
    assert!(stream.is_done());
    drop(stream);
    fake.finish();
}

/// A plain TCP server that answers the upgrade request with this raw HTTP response, as a real
/// server's refusal arrives: Mistral sends its 401 with a chunked body.
fn raw_refusal(response: Vec<u8>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = thread::spawn(move || {
        let (mut tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(WAIT)).unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            match tcp.read(&mut byte) {
                Ok(1) => request.push(byte[0]),
                _ => return,
            }
        }
        tcp.write_all(&response).unwrap();
        tcp.flush().unwrap();
        // Until the client hangs up.
        let _ = tcp.read(&mut byte);
    });
    (
        format!("http://127.0.0.1:{port}/v1/audio/transcriptions"),
        thread,
    )
}

#[test]
fn a_chunked_refusal_reads_as_its_body() {
    let body = br#"{"detail":"Invalid API Key"}"#;
    let mut response = format!(
        "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response.extend_from_slice(b"\r\n0\r\n\r\n");
    let (endpoint, server) = raw_refusal(response);
    let client = MistralRealtime::new(&cfg(&endpoint, MODEL, KEY)).unwrap();
    let error = match client.connect(&StreamSession::default()) {
        Ok(_) => panic!("connected"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Auth);
    assert_eq!(error.message, "API error 401: Invalid API Key");
    server.join().unwrap();
}

#[test]
fn nothing_listening_is_a_network_error() {
    let port = closed_port();
    let endpoint = format!("http://127.0.0.1:{port}/v1/audio/transcriptions");
    let client = MistralRealtime::new(&cfg(&endpoint, MODEL, KEY)).unwrap();
    let error = match client.connect(&StreamSession::default()) {
        Ok(_) => panic!("connected"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Network);
    assert_eq!(
        error.message,
        format!("Couldn't connect to 127.0.0.1:{port}. Check that the server is running.")
    );
    assert_no_key(&error);
}

#[test]
fn no_session_updated_is_a_timeout() {
    let fake = Fake::start(|server| {
        server.recv_type("session.update");
        server.wait_for_close();
    });
    let client = fake.client().with_timeout(Duration::from_millis(400));
    let started = Instant::now();
    let error = match client.connect(&StreamSession::default()) {
        Ok(_) => panic!("connected"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::Timeout);
    assert_eq!(
        error.message,
        "Mistral didn't answer within 1 second. Try again."
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    fake.finish();
}

#[test]
fn a_server_that_closes_before_done_is_an_error() {
    let fake = Fake::start(|server| {
        server.accept_session();
        server.recv_type("input_audio.append");
        server.ws.close(None).unwrap();
        server.wait_for_close();
    });
    let mut stream = fake.client().connect(&StreamSession::default()).unwrap();
    stream.push(&pcm(1600, 4)).unwrap();
    let (events, error) = poll_all(stream.as_mut());
    assert!(events.is_empty(), "{events:?}");
    let error = error.expect("an error");
    assert_eq!(error.kind, ErrorKind::Network);
    assert_eq!(
        error.message,
        "Mistral closed the stream before it finished."
    );
    drop(stream);
    fake.finish();
}

#[test]
fn poll_waits_at_most_the_given_time() {
    let fake = Fake::start(|server| {
        server.accept_session();
        server.wait_for_close();
    });
    let mut stream = fake.client().open(&StreamSession::default()).unwrap();
    // The session messages read during open come first, without waiting.
    let raw = stream.poll_raw(Duration::from_secs(5)).unwrap();
    let kinds: Vec<&str> = raw.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["session.created", "session.updated"]);
    let started = Instant::now();
    assert_eq!(stream.poll(Duration::from_millis(150)), Ok(Vec::new()));
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(100), "{waited:?}");
    assert!(waited < Duration::from_secs(2), "{waited:?}");
    assert!(!stream.is_done());
    drop(stream);
    fake.finish();
}

#[test]
fn parse_realtime_event_ignores_what_voice_doesnt_use() {
    for ignored in [
        json!({"type": "session.created", "session": {}}),
        json!({"type": "session.updated", "session": {}}),
        json!({"type": "turn.end", "time_s": 1.0, "end_reason": "silence"}),
        json!({"type": "transcription.segment", "text": "", "speaker_id": "speaker_1"}),
        json!({"type": "something.new"}),
        json!({"no": "type"}),
    ] {
        assert_eq!(parse_realtime_event(&ignored), Ok(None), "{ignored}");
    }
    assert_eq!(
        parse_realtime_event(
            &json!({"type": "transcription.segment", "text": "Ja", "speaker_id": 2, "start": 0, "end": 1})
        ),
        Ok(Some(StreamEvent::Segment {
            text: "Ja".into(),
            speaker: Some("2".into()),
            start: Some(0.0),
            end: Some(1.0),
        }))
    );
    assert_eq!(
        parse_realtime_event(
            &json!({"type": "transcription.segment", "text": "Ja", "speaker_id": " "})
        ),
        Ok(Some(StreamEvent::Segment {
            text: "Ja".into(),
            speaker: None,
            start: None,
            end: None,
        }))
    );
    let error = parse_realtime_event(
        &json!({"type": "error", "error": {"message": "Bad key", "code": 401}}),
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Auth);
}
