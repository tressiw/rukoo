//! Test helpers: temporary folders and a local fake HTTP server that records every request.
//! No test talks to a real provider or uses a real key.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::Value;

/// A folder under the system temp folder, removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join("voice-core-tests")
            .join(format!("{label}-{}-{count}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, part: &str) -> PathBuf {
        self.0.join(part)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    /// Path and query, as sent.
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }

    /// The parts of a multipart/form-data body, in order.
    pub fn parts(&self) -> Vec<Part> {
        let content_type = self.header("content-type").expect("content type");
        let boundary = content_type
            .split("boundary=")
            .nth(1)
            .expect("boundary")
            .trim_matches('"');
        let delimiter = format!("--{boundary}").into_bytes();
        let mut parts = Vec::new();
        let sections = split_bytes(&self.body, &delimiter);
        for section in sections.iter().skip(1) {
            if section.starts_with(b"--") {
                break;
            }
            let section = section.strip_prefix(b"\r\n").expect("CRLF after delimiter");
            let section = section
                .strip_suffix(b"\r\n")
                .expect("CRLF before delimiter");
            let split = find(section, b"\r\n\r\n").expect("part header end");
            let head = String::from_utf8(section[..split].to_vec()).unwrap();
            let data = section[split + 4..].to_vec();
            let mut part = Part {
                name: String::new(),
                filename: None,
                content_type: None,
                data,
            };
            for line in head.split("\r\n") {
                let lower = line.to_ascii_lowercase();
                if lower.starts_with("content-disposition:") {
                    part.name = attribute(line, "name").unwrap_or_default();
                    part.filename = attribute(line, "filename");
                } else if lower.starts_with("content-type:") {
                    part.content_type = Some(line["content-type:".len()..].trim().to_string());
                }
            }
            parts.push(part);
        }
        parts
    }

    /// The text parts of a multipart body as (name, value) pairs, in order, without files.
    pub fn form_fields(&self) -> Vec<(String, String)> {
        self.parts()
            .into_iter()
            .filter(|part| part.filename.is_none())
            .map(|part| (part.name, String::from_utf8(part.data).unwrap()))
            .collect()
    }
}

fn attribute(line: &str, name: &str) -> Option<String> {
    let needle = format!("; {name}=\"");
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn split_bytes(haystack: &[u8], delimiter: &[u8]) -> Vec<Vec<u8>> {
    let mut sections = Vec::new();
    let mut rest = haystack;
    while let Some(at) = find(rest, delimiter) {
        sections.push(rest[..at].to_vec());
        rest = &rest[at + delimiter.len()..];
    }
    sections.push(rest.to_vec());
    sections
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: &'static str,
    /// Accept the request but never answer, to test timeouts.
    pub hang: bool,
}

impl Reply {
    pub fn json(status: u16, body: Value) -> Reply {
        Reply {
            status,
            body: body.to_string().into_bytes(),
            content_type: "application/json",
            hang: false,
        }
    }

    pub fn text(status: u16, body: &str) -> Reply {
        Reply {
            status,
            body: body.as_bytes().to_vec(),
            content_type: "text/plain",
            hang: false,
        }
    }

    pub fn bytes(status: u16, body: Vec<u8>) -> Reply {
        Reply {
            status,
            body,
            content_type: "application/octet-stream",
            hang: false,
        }
    }

    pub fn hang() -> Reply {
        Reply {
            status: 200,
            body: Vec::new(),
            content_type: "text/plain",
            hang: true,
        }
    }
}

type Handler = dyn Fn(&Recorded) -> Reply + Send + Sync;

pub struct FakeServer {
    port: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
    stopped: Arc<AtomicBool>,
}

impl FakeServer {
    pub fn start(handler: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let handler: Arc<Handler> = Arc::new(handler);
        let (recorded, stop) = (requests.clone(), stopped.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (recorded, handler) = (recorded.clone(), handler.clone());
                thread::spawn(move || serve(stream, &recorded, &*handler));
            }
        });
        FakeServer {
            port,
            requests,
            stopped,
        }
    }

    /// Answers every request with the same reply.
    pub fn always(reply: Reply) -> FakeServer {
        FakeServer::start(move |_| reply.clone())
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub fn only_request(&self) -> Recorded {
        let requests = self.requests();
        assert_eq!(requests.len(), 1, "expected exactly one request");
        requests.into_iter().next().unwrap()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve(stream: TcpStream, recorded: &Mutex<Vec<Recorded>>, handler: &Handler) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let mut words = request_line.split_whitespace();
    let method = words.next().unwrap_or_default().to_string();
    let target = words.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    let mut body = Vec::new();
    if let Some(length) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(length, 0);
        if reader.read_exact(&mut body).is_err() {
            return;
        }
    } else if header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size_line = String::new();
            if reader.read_line(&mut size_line).unwrap_or(0) == 0 {
                return;
            }
            let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or("0"), 16)
                .unwrap_or(0);
            let mut chunk = vec![0u8; size + 2];
            if reader.read_exact(&mut chunk).is_err() {
                return;
            }
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
    }
    let request = Recorded {
        method,
        target,
        headers,
        body,
    };
    recorded.lock().unwrap().push(request.clone());
    let reply = handler(&request);
    if reply.hang {
        thread::sleep(Duration::from_secs(60));
        return;
    }
    let mut stream = stream;
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reason(reply.status),
        reply.content_type,
        reply.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "Status",
    }
}

/// A port nothing listens on: 1, tcpmux, which no system the tests run on serves. A port freed
/// after binding port 0 could go to a `FakeServer` that another test starts at the same moment,
/// and the "unreachable" request would reach it (#313). Binding port 0 only hands out ports from
/// the ephemeral range, never 1.
pub fn closed_port() -> u16 {
    1
}
