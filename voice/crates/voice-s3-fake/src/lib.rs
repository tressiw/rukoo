//! A small S3-compatible server with fault injection, for Rukoo Voice's sync tests and the live
//! paired run. std only; it never ships.
//!
//! It answers path-style and virtual-host requests for one bucket, checks the access key in
//! `X-Amz-Credential` and the expiry from `X-Amz-Date` plus `X-Amz-Expires` (403 `AccessDenied`
//! "Request has expired" afterwards), and doesn't verify signatures. XML bodies follow the S3 API
//! for the calls in [`Op`]. ETags are `"<length hex>-<FNV-1a 64 hex>"`.
//!
//! Each connection gets a thread and may carry many requests (HTTP/1.1 keep-alive). Requests
//! with a `Content-Length` or a chunked body are read in full before anything is stored, so a
//! client killed mid-upload leaves no object and no part behind.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long stopping waits for answers already on their way and for their log lines.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// A TcpListener on 127.0.0.1, a thread per connection, the store, the log and the faults.
pub struct FakeS3 {
    addr: SocketAddr,
    faults: Arc<Mutex<Faults>>,
    log: Arc<Mutex<Vec<Logged>>>,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

/// Faults to inject. Each applies to the requests that arrive after it was set.
#[derive(Debug, Default)]
pub struct Faults {
    /// The next matching requests answer this status and S3 code. Each entry answers one
    /// request and is then removed.
    pub fail: Vec<(Op, u16, &'static str)>,
    /// Hold every answer this long.
    pub stall: Option<Duration>,
    /// Close the socket after this many body bytes of the next upload. Clears itself once used.
    pub drop_after: Option<u64>,
    /// Close every connection at once.
    pub refuse: bool,
    /// Refuse unequal non-last parts, as R2 does (InvalidPart).
    pub r2_parts: bool,
    /// NoSuchUpload for every open upload id. The next request forgets every upload open at that
    /// moment and clears the flag, so uploads created afterwards work.
    pub forget_uploads: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Put,
    Get,
    Head,
    Delete,
    DeleteMany,
    List,
    Create,
    UploadPart,
    ListParts,
    Complete,
    Abort,
}

impl Op {
    /// The name in `log.jsonl`: lowercase snake case.
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Put => "put",
            Op::Get => "get",
            Op::Head => "head",
            Op::Delete => "delete",
            Op::DeleteMany => "delete_many",
            Op::List => "list",
            Op::Create => "create",
            Op::UploadPart => "upload_part",
            Op::ListParts => "list_parts",
            Op::Complete => "complete",
            Op::Abort => "abort",
        }
    }
}

/// One answered request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logged {
    pub op: Op,
    /// The object key, or for a listing its prefix; empty for `DeleteMany`.
    pub key: String,
    pub part: Option<u16>,
    /// Body bytes received for uploads (`Put`, `UploadPart`), sent for successful `Get`s; 0 for
    /// everything else.
    pub bytes: u64,
    pub status: u16,
    /// Wall ms since the epoch when the answer went out.
    pub at: u64,
}

impl FakeS3 {
    /// On a free port. `dir`: keep objects on disk, so another process sees them after a restart,
    /// in the layout `src/main.rs` describes.
    pub fn start(bucket: &str, access_key: &str, dir: Option<PathBuf>) -> io::Result<FakeS3> {
        FakeS3::start_on(0, bucket, access_key, dir)
    }

    /// On `port` of 127.0.0.1 (0 for a free one), so a restarted fake keeps its endpoint.
    pub fn start_on(
        port: u16,
        bucket: &str,
        access_key: &str,
        dir: Option<PathBuf>,
    ) -> io::Result<FakeS3> {
        let store = match &dir {
            Some(dir) => Store::load(dir)?,
            None => Store::default(),
        };
        let log_file = match &dir {
            Some(dir) => Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join("log.jsonl"))?,
            ),
            None => None,
        };
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let faults = Arc::new(Mutex::new(Faults::default()));
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::new(Shared {
            bucket: bucket.to_string(),
            access_key: access_key.to_string(),
            dir,
            store: Mutex::new(store),
            faults: Arc::clone(&faults),
            log: Arc::clone(&log),
            log_file: Mutex::new(log_file),
            stopping: AtomicBool::new(false),
            connections: Mutex::new(BTreeMap::new()),
            counter: AtomicU64::new(0),
            answering: Mutex::new(0),
            answered: Condvar::new(),
        });
        let accept = {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("fake-s3-accept".into())
                .spawn(move || accept_loop(&shared, &listener))?
        };
        Ok(FakeS3 {
            addr,
            faults,
            log,
            shared,
            accept: Some(accept),
        })
    }

    /// `http://127.0.0.1:<port>`.
    pub fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn faults(&self) -> MutexGuard<'_, Faults> {
        self.faults.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the fake answered, oldest first. A client has its answer a moment before the fake
    /// notes the request, so this waits up to 2 s for answers being written: a test that reads
    /// the log right after a call sees that call.
    pub fn log(&self) -> Vec<Logged> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut answering = lock(&self.shared.answering);
        while *answering > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            answering = self
                .shared
                .answered
                .wait_timeout(answering, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        drop(answering);
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keys and raw stored bytes, sorted by key.
    pub fn objects(&self) -> Vec<(String, Vec<u8>)> {
        lock(&self.shared.store)
            .objects
            .iter()
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    }

    /// Stops listening and closes every open connection.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for FakeS3 {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        // Wake the accept loop.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        // A connection between requests sees EOF and leaves, while one that is writing its answer
        // still finishes and logs it. A request is logged after its answer, so a client that got
        // one finds its line once `stop` returns.
        for stream in lock(&self.shared.connections).values() {
            let _ = stream.shutdown(Shutdown::Read);
        }
        // A thread drops its entry once it has logged its last answer.
        let deadline = Instant::now() + STOP_WAIT;
        loop {
            if lock(&self.shared.connections).is_empty() {
                return;
            }
            if Instant::now() >= deadline {
                // A client that doesn't read its answer keeps the thread in a write: let go.
                for stream in lock(&self.shared.connections).values() {
                    let _ = stream.shutdown(Shutdown::Both);
                }
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

struct Shared {
    bucket: String,
    access_key: String,
    dir: Option<PathBuf>,
    store: Mutex<Store>,
    faults: Arc<Mutex<Faults>>,
    log: Arc<Mutex<Vec<Logged>>>,
    log_file: Mutex<Option<File>>,
    stopping: AtomicBool,
    /// A clone of each open connection, so stopping can close it.
    connections: Mutex<BTreeMap<u64, TcpStream>>,
    /// Connection ids, upload ids and temporary file names.
    counter: AtomicU64,
    /// Answers being written whose request isn't in the log yet, and the signal when one is.
    answering: Mutex<usize>,
    answered: Condvar,
}

impl Shared {
    fn faults(&self) -> MutexGuard<'_, Faults> {
        lock(&self.faults)
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    fn record(&self, entry: Logged) {
        if let Some(file) = lock(&self.log_file).as_mut() {
            let part = entry
                .part
                .map_or_else(|| "null".to_string(), |p| p.to_string());
            // One write per line: the live run stops the fake by its process id, and a line
            // written in pieces could be cut in half.
            let line = format!(
                "{{\"op\":\"{}\",\"key\":{},\"part\":{},\"bytes\":{},\"status\":{},\"at\":{}}}\n",
                entry.op.as_str(),
                json_string(&entry.key),
                part,
                entry.bytes,
                entry.status,
                entry.at
            );
            let _ = file.write_all(line.as_bytes());
        }
        lock(&self.log).push(entry);
    }
}

/// One answer being written, from just before its first byte until its request is in the log
/// or the answer failed to go out.
struct Answering<'a>(&'a Shared);

impl<'a> Answering<'a> {
    fn start(shared: &'a Shared) -> Answering<'a> {
        *lock(&shared.answering) += 1;
        Answering(shared)
    }
}

impl Drop for Answering<'_> {
    fn drop(&mut self) {
        let mut answering = lock(&self.0.answering);
        *answering = answering.saturating_sub(1);
        self.0.answered.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn accept_loop(shared: &Arc<Shared>, listener: &TcpListener) {
    for stream in listener.incoming() {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        if shared.faults().refuse {
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }
        let id = shared.next();
        if let Ok(clone) = stream.try_clone() {
            lock(&shared.connections).insert(id, clone);
        }
        let shared = Arc::clone(shared);
        let spawned = thread::Builder::new()
            .name("fake-s3-conn".into())
            .spawn(move || {
                serve(&shared, stream);
                lock(&shared.connections).remove(&id);
            });
        if spawned.is_err() {
            break;
        }
    }
}

// ---- HTTP ------------------------------------------------------------------------------------

struct Head {
    method: String,
    /// The path, still percent-encoded.
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn has(&self, name: &str) -> bool {
        self.query.iter().any(|(k, _)| k == name)
    }
}

struct Answer {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    /// HEAD: send the headers without a body.
    head_only: bool,
    /// Content-Length when it isn't the body's: an object's length for HEAD.
    length: Option<u64>,
}

impl Answer {
    fn new(status: u16) -> Answer {
        Answer {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            head_only: false,
            length: None,
        }
    }

    fn xml(status: u16, body: String) -> Answer {
        let mut answer = Answer::new(status);
        answer
            .headers
            .push(("Content-Type", "application/xml".to_string()));
        answer.body = body.into_bytes();
        answer
    }

    fn error(status: u16, code: &str, resource: &str) -> Answer {
        Answer::xml(
            status,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>{}</Code><Message>{}</Message><Resource>{}</Resource><RequestId>fake</RequestId></Error>",
                xml_escape(code),
                xml_escape(message_for(code)),
                xml_escape(resource)
            ),
        )
    }
}

fn message_for(code: &str) -> &'static str {
    match code {
        "AccessDenied" => "Access Denied",
        "InvalidAccessKeyId" => "The AWS Access Key Id you provided does not exist in our records.",
        "SignatureDoesNotMatch" => {
            "The request signature we calculated does not match the signature you provided."
        }
        "NoSuchBucket" => "The specified bucket does not exist",
        "NoSuchKey" => "The specified key does not exist.",
        "NoSuchUpload" => "The specified multipart upload does not exist.",
        "InvalidPart" => "One or more of the specified parts could not be found.",
        "InvalidPartOrder" => "The list of parts was not in ascending order.",
        "InvalidRange" => "The requested range is not satisfiable",
        "MalformedXML" => "The XML you provided was not well-formed.",
        "RequestTimeTooSkewed" => {
            "The difference between the request time and the current time is too large."
        }
        "SlowDown" => "Please reduce your request rate.",
        "InternalError" => "We encountered an internal error. Please try again.",
        "ServiceUnavailable" => "Please reduce your request rate.",
        _ => "The fake S3 refused the request.",
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// One connection: requests until the client closes, asks to close, or a fault closes it.
fn serve(shared: &Shared, stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let head = match read_head(&mut reader) {
            Ok(Some(head)) => head,
            Ok(None) | Err(_) => break,
        };
        if shared.faults().refuse {
            break;
        }
        let close = head
            .header("connection")
            .is_some_and(|v| v.eq_ignore_ascii_case("close"));
        if head
            .header("expect")
            .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"))
            && writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").is_err()
        {
            break;
        }
        let op = op_of(&head);
        let upload = matches!(op, Some(Op::Put | Op::UploadPart));
        let drop_after = if upload {
            shared.faults().drop_after.take()
        } else {
            None
        };
        if let Some(limit) = drop_after {
            // Take what the fault allows of the body, then hang up without an answer.
            let _ = read_body(&mut reader, &head, Some(limit));
            break;
        }
        let Ok(body) = read_body(&mut reader, &head, None) else {
            break;
        };
        let (answer, logged) = match op {
            Some(op) => {
                let (answer, seen) = handle(shared, op, &head, &body);
                let bytes = match seen.op {
                    Op::Put | Op::UploadPart => body.len() as u64,
                    Op::Get if answer.status < 300 => answer.body.len() as u64,
                    _ => 0,
                };
                let logged = Logged {
                    op: seen.op,
                    key: seen.key,
                    part: seen.part,
                    bytes,
                    status: answer.status,
                    at: 0,
                };
                (answer, Some(logged))
            }
            None => (Answer::error(405, "MethodNotAllowed", &head.path), None),
        };
        let stall = shared.faults().stall;
        if let Some(stall) = stall {
            thread::sleep(stall);
        }
        let answering = Answering::start(shared);
        if write_answer(&mut writer, &head, &answer, close).is_err() {
            break;
        }
        if let Some(mut logged) = logged {
            logged.at = now_ms();
            shared.record(logged);
        }
        drop(answering);
        if close {
            break;
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}

fn read_head(reader: &mut impl BufRead) -> io::Result<Option<Head>> {
    let mut line = String::new();
    // Skip blank lines between requests.
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.trim_end().splitn(3, ' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), parse_query(query)),
        None => (target.clone(), Vec::new()),
    };
    Ok(Some(Head {
        method,
        path,
        query,
        headers,
    }))
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// The request body; with `limit`, stops after that many bytes.
fn read_body(reader: &mut impl BufRead, head: &Head, limit: Option<u64>) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    let chunked = head
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size)?;
            let size = size.trim().split(';').next().unwrap_or("");
            let size = u64::from_str_radix(size, 16)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
            if size == 0 {
                // Trailers, then the blank line.
                loop {
                    let mut trailer = String::new();
                    if reader.read_line(&mut trailer)? == 0 || trailer.trim().is_empty() {
                        break;
                    }
                }
                return Ok(body);
            }
            let take = match limit {
                Some(limit) => size.min(limit.saturating_sub(body.len() as u64)),
                None => size,
            };
            let start = body.len();
            body.resize(start + take as usize, 0);
            reader.read_exact(&mut body[start..])?;
            if take < size {
                return Ok(body);
            }
            let mut crlf = [0u8; 2];
            reader.read_exact(&mut crlf)?;
        }
    }
    let length: u64 = head
        .header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let take = limit.map_or(length, |limit| length.min(limit));
    body.resize(take as usize, 0);
    reader.read_exact(&mut body)?;
    Ok(body)
}

fn write_answer(
    writer: &mut impl Write,
    head: &Head,
    answer: &Answer,
    close: bool,
) -> io::Result<()> {
    let mut out = format!("HTTP/1.1 {} {}\r\n", answer.status, reason(answer.status));
    if answer.status != 204 {
        let length = answer.length.unwrap_or(answer.body.len() as u64);
        out.push_str(&format!("Content-Length: {length}\r\n"));
    }
    out.push_str("x-amz-request-id: fake\r\nServer: rukoo-voice-fake-s3\r\n");
    for (name, value) in &answer.headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    if close {
        out.push_str("Connection: close\r\n");
    }
    out.push_str("\r\n");
    writer.write_all(out.as_bytes())?;
    if !answer.head_only && head.method != "HEAD" {
        writer.write_all(&answer.body)?;
    }
    writer.flush()
}

// ---- Routing and the S3 calls ------------------------------------------------------------------

fn op_of(head: &Head) -> Option<Op> {
    Some(match head.method.as_str() {
        "PUT" if head.has("partNumber") && head.has("uploadId") => Op::UploadPart,
        "PUT" => Op::Put,
        "GET" if head.has("uploadId") => Op::ListParts,
        "GET" if head.has("list-type") || head.has("prefix") || head.has("delimiter") => Op::List,
        "GET" => Op::Get,
        "HEAD" => Op::Head,
        "DELETE" if head.has("uploadId") => Op::Abort,
        "DELETE" => Op::Delete,
        "POST" if head.has("uploads") => Op::Create,
        "POST" if head.has("uploadId") => Op::Complete,
        "POST" if head.has("delete") => Op::DeleteMany,
        _ => return None,
    })
}

/// What the log keeps of a request besides its answer.
struct Seen {
    op: Op,
    key: String,
    part: Option<u16>,
}

/// Answers one request.
fn handle(shared: &Shared, op: Op, head: &Head, body: &[u8]) -> (Answer, Seen) {
    let part = head.query("partNumber").and_then(|n| n.parse::<u16>().ok());
    let Some(key) = route(shared, head) else {
        let seen = Seen {
            op,
            key: String::new(),
            part,
        };
        return (Answer::error(404, "NoSuchBucket", &head.path), seen);
    };
    // An empty key at the bucket's root is a listing, whatever else the query says.
    let op = if op == Op::Get && key.is_empty() {
        Op::List
    } else {
        op
    };
    let seen = Seen {
        op,
        key: match op {
            Op::List => head.query("prefix").unwrap_or("").to_string(),
            Op::DeleteMany => String::new(),
            _ => key.clone(),
        },
        part,
    };
    let resource = format!("/{}/{}", shared.bucket, key);
    if let Some(answer) = check_auth(shared, head, &resource) {
        return (answer, seen);
    }
    {
        let mut faults = shared.faults();
        if faults.forget_uploads {
            faults.forget_uploads = false;
            drop(faults);
            lock(&shared.store).forget_uploads(shared.dir.as_deref());
            faults = shared.faults();
        }
        if let Some(i) = faults.fail.iter().position(|(o, _, _)| *o == op) {
            let (_, status, code) = faults.fail.remove(i);
            return (Answer::error(status, code, &resource), seen);
        }
    }
    let answer = match op {
        Op::Put => put(shared, &key, body),
        Op::Get => get(shared, head, &key, &resource),
        Op::Head => head_object(shared, &key),
        Op::Delete => {
            lock(&shared.store).remove(shared, &key);
            Answer::new(204)
        }
        Op::DeleteMany => delete_many(shared, body),
        Op::List => list(shared, head),
        Op::Create => create(shared, &key),
        Op::UploadPart => upload_part(shared, head, &key, body, &resource),
        Op::ListParts => list_parts(shared, head, &key, &resource),
        Op::Complete => complete(shared, head, &key, body, &resource),
        Op::Abort => abort(shared, head, &key, &resource),
    };
    (answer, seen)
}

/// The object key of a request for our bucket, path-style or virtual-host; None for another
/// bucket.
fn route(shared: &Shared, head: &Head) -> Option<String> {
    let host = head.header("host").unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(name, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            name
        } else {
            host
        }
    });
    let path = percent_decode(&head.path);
    let virtual_host = format!("{}.", shared.bucket);
    if host.len() > virtual_host.len() && host.starts_with(&virtual_host) {
        return Some(path.strip_prefix('/').unwrap_or(&path).to_string());
    }
    let rest = path
        .strip_prefix('/')?
        .strip_prefix(shared.bucket.as_str())?;
    match rest {
        "" => Some(String::new()),
        _ => rest.strip_prefix('/').map(str::to_string),
    }
}

/// The access key and the presigned URL's expiry; None when the request may go on.
fn check_auth(shared: &Shared, head: &Head, resource: &str) -> Option<Answer> {
    let credential = head
        .query("X-Amz-Credential")
        .map(str::to_string)
        .or_else(|| {
            let auth = head.header("authorization")?;
            let start = auth.find("Credential=")? + "Credential=".len();
            let rest = &auth[start..];
            Some(rest.split([',', ' ']).next().unwrap_or("").to_string())
        });
    let Some(credential) = credential else {
        return Some(Answer::error(403, "AccessDenied", resource));
    };
    let access_key = credential.split('/').next().unwrap_or("");
    if access_key != shared.access_key {
        return Some(Answer::error(403, "InvalidAccessKeyId", resource));
    }
    if let (Some(date), Some(expires)) = (head.query("X-Amz-Date"), head.query("X-Amz-Expires")) {
        let (Some(signed), Ok(expires)) = (parse_amz_date(date), expires.parse::<u64>()) else {
            return Some(Answer::error(
                400,
                "AuthorizationQueryParametersError",
                resource,
            ));
        };
        if now_ms() / 1000 > signed + expires {
            let mut answer = Answer::xml(
                403,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>AccessDenied</Code><Message>Request has expired</Message><Resource>{}</Resource><RequestId>fake</RequestId></Error>",
                    xml_escape(resource)
                ),
            );
            answer.head_only = head.method == "HEAD";
            return Some(answer);
        }
    }
    None
}

fn put(shared: &Shared, key: &str, body: &[u8]) -> Answer {
    if key.is_empty() {
        return Answer::error(400, "InvalidArgument", "/");
    }
    let mut store = lock(&shared.store);
    if let Err(answer) = store.insert(shared, key, body.to_vec()) {
        return answer;
    }
    let mut answer = Answer::new(200);
    answer.headers.push(("ETag", etag(body)));
    answer
}

fn get(shared: &Shared, head: &Head, key: &str, resource: &str) -> Answer {
    let store = lock(&shared.store);
    let Some(object) = store.objects.get(key) else {
        return Answer::error(404, "NoSuchKey", resource);
    };
    let len = object.len() as u64;
    let range = head.header("range").map(|r| parse_range(r, len));
    let mut answer;
    match range {
        None => {
            answer = Answer::new(200);
            answer.body = object.clone();
        }
        Some(Some((from, to))) => {
            answer = Answer::new(206);
            answer.body = object[from as usize..=to as usize].to_vec();
            answer
                .headers
                .push(("Content-Range", format!("bytes {from}-{to}/{len}")));
        }
        Some(None) => {
            let mut refused = Answer::error(416, "InvalidRange", resource);
            refused
                .headers
                .push(("Content-Range", format!("bytes */{len}")));
            return refused;
        }
    }
    answer.headers.push(("ETag", etag(object)));
    answer.headers.push(("Accept-Ranges", "bytes".into()));
    answer
        .headers
        .push(("Content-Type", "application/octet-stream".into()));
    answer
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` against `len`: the inclusive range, or None when it
/// can't be satisfied.
fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (from, to) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            if n == 0 || len == 0 {
                return None;
            }
            (len.saturating_sub(n), len - 1)
        }
        (a, "") => (a.parse().ok()?, len.checked_sub(1)?),
        (a, b) => (
            a.parse().ok()?,
            b.parse::<u64>().ok()?.min(len.checked_sub(1)?),
        ),
    };
    (from < len && from <= to).then_some((from, to))
}

fn head_object(shared: &Shared, key: &str) -> Answer {
    let store = lock(&shared.store);
    let Some(object) = store.objects.get(key) else {
        let mut answer = Answer::new(404);
        answer.head_only = true;
        return answer;
    };
    let mut answer = Answer::new(200);
    answer.length = Some(object.len() as u64);
    answer.head_only = true;
    answer.headers.push(("ETag", etag(object)));
    answer
        .headers
        .push(("Content-Type", "application/octet-stream".into()));
    answer
}

fn delete_many(shared: &Shared, body: &[u8]) -> Answer {
    let text = String::from_utf8_lossy(body);
    let quiet = elements(&text, "Quiet")
        .first()
        .is_some_and(|v| v.trim() == "true");
    let keys: Vec<String> = elements(&text, "Object")
        .iter()
        .filter_map(|object| elements(object, "Key").first().map(|k| xml_unescape(k)))
        .collect();
    if keys.is_empty() || keys.len() > 1000 {
        return Answer::error(400, "MalformedXML", "/");
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
    );
    let mut store = lock(&shared.store);
    for key in keys {
        store.remove(shared, &key);
        if !quiet {
            out.push_str(&format!(
                "<Deleted><Key>{}</Key></Deleted>",
                xml_escape(&key)
            ));
        }
    }
    out.push_str("</DeleteResult>");
    Answer::xml(200, out)
}

fn list(shared: &Shared, head: &Head) -> Answer {
    let prefix = head.query("prefix").unwrap_or("");
    let delimiter = head.query("delimiter").filter(|d| !d.is_empty());
    let max_keys = head
        .query("max-keys")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(1000)
        .min(1000);
    let after = match head.query("continuation-token") {
        Some(token) => match unhex(token) {
            Some(after) => Some(after),
            None => return Answer::error(400, "InvalidArgument", "/"),
        },
        None => head.query("start-after").map(str::to_string),
    };
    let url = head.query("encoding-type") == Some("url");
    let encode = |s: &str| {
        if url {
            percent_encode(s)
        } else {
            xml_escape(s)
        }
    };

    enum Entry {
        Key(String, u64, String),
        Prefix(String),
    }
    let store = lock(&shared.store);
    let mut entries: Vec<Entry> = Vec::new();
    // Keys that start with the prefix sit together right from the prefix on.
    let start = match &after {
        Some(after) if after.as_str() > prefix => after.clone(),
        _ => prefix.to_string(),
    };
    for (key, bytes) in store.objects.range(start..) {
        if after.as_deref().is_some_and(|after| key.as_str() <= after) {
            continue;
        }
        if !key.starts_with(prefix) {
            break;
        }
        if let Some(delimiter) = delimiter {
            if let Some(pos) = key[prefix.len()..].find(delimiter) {
                let common = &key[..prefix.len() + pos + delimiter.len()];
                if after.as_deref().is_some_and(|after| common <= after) {
                    continue;
                }
                if matches!(entries.last(), Some(Entry::Prefix(p)) if p == common) {
                    continue;
                }
                entries.push(Entry::Prefix(common.to_string()));
                if entries.len() > max_keys {
                    break;
                }
                continue;
            }
        }
        entries.push(Entry::Key(key.clone(), bytes.len() as u64, etag(bytes)));
        if entries.len() > max_keys {
            break;
        }
    }
    drop(store);
    let truncated = entries.len() > max_keys;
    entries.truncate(max_keys);

    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
    );
    out.push_str(&format!("<Name>{}</Name>", xml_escape(&shared.bucket)));
    out.push_str(&format!("<Prefix>{}</Prefix>", encode(prefix)));
    if let Some(delimiter) = delimiter {
        out.push_str(&format!("<Delimiter>{}</Delimiter>", encode(delimiter)));
    }
    out.push_str(&format!(
        "<KeyCount>{}</KeyCount><MaxKeys>{max_keys}</MaxKeys><IsTruncated>{truncated}</IsTruncated>",
        entries.len()
    ));
    if url {
        out.push_str("<EncodingType>url</EncodingType>");
    }
    if let Some(token) = head.query("continuation-token") {
        out.push_str(&format!(
            "<ContinuationToken>{}</ContinuationToken>",
            xml_escape(token)
        ));
    }
    if truncated {
        let last = match entries.last() {
            Some(Entry::Key(key, _, _)) => key.as_str(),
            Some(Entry::Prefix(prefix)) => prefix.as_str(),
            None => "",
        };
        out.push_str(&format!(
            "<NextContinuationToken>{}</NextContinuationToken>",
            hex(last.as_bytes())
        ));
    }
    let modified = iso8601(now_ms() / 1000);
    for entry in &entries {
        if let Entry::Key(key, size, etag) = entry {
            out.push_str(&format!(
                "<Contents><Key>{}</Key><LastModified>{modified}</LastModified><ETag>{}</ETag><Size>{size}</Size><StorageClass>STANDARD</StorageClass></Contents>",
                encode(key),
                xml_escape(etag)
            ));
        }
    }
    for entry in &entries {
        if let Entry::Prefix(prefix) = entry {
            out.push_str(&format!(
                "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
                encode(prefix)
            ));
        }
    }
    out.push_str("</ListBucketResult>");
    Answer::xml(200, out)
}

fn create(shared: &Shared, key: &str) -> Answer {
    if key.is_empty() {
        return Answer::error(400, "InvalidArgument", "/");
    }
    let id = format!("{:016x}{:08x}", now_nanos(), shared.next());
    if let Err(answer) = lock(&shared.store).create(shared, &id, key) {
        return answer;
    }
    Answer::xml(
        200,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>{}</Bucket><Key>{}</Key><UploadId>{id}</UploadId></InitiateMultipartUploadResult>",
            xml_escape(&shared.bucket),
            xml_escape(key)
        ),
    )
}

fn upload_part(shared: &Shared, head: &Head, key: &str, body: &[u8], resource: &str) -> Answer {
    let Some(number) = head
        .query("partNumber")
        .and_then(|n| n.parse::<u16>().ok())
        .filter(|n| (1..=10_000).contains(n))
    else {
        return Answer::error(400, "InvalidArgument", resource);
    };
    let id = head.query("uploadId").unwrap_or("");
    let mut store = lock(&shared.store);
    if store.uploads.get(id).is_none_or(|u| u.key != key) {
        return Answer::error(404, "NoSuchUpload", resource);
    }
    if let Err(answer) = store.put_part(shared, id, number, body.to_vec()) {
        return answer;
    }
    let mut answer = Answer::new(200);
    answer.headers.push(("ETag", etag(body)));
    answer
}

fn list_parts(shared: &Shared, head: &Head, key: &str, resource: &str) -> Answer {
    let id = head.query("uploadId").unwrap_or("");
    let max_parts = head
        .query("max-parts")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(1000)
        .clamp(1, 1000);
    let marker = head
        .query("part-number-marker")
        .and_then(|n| n.parse::<u16>().ok())
        .unwrap_or(0);
    let store = lock(&shared.store);
    let Some(upload) = store.uploads.get(id).filter(|u| u.key == key) else {
        return Answer::error(404, "NoSuchUpload", resource);
    };
    let parts: Vec<(u16, u64, String)> = upload
        .parts
        .range(marker.saturating_add(1)..)
        .take(max_parts + 1)
        .map(|(n, bytes)| (*n, bytes.len() as u64, etag(bytes)))
        .collect();
    drop(store);
    let truncated = parts.len() > max_parts;
    let parts = &parts[..parts.len().min(max_parts)];
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListPartsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Bucket>{}</Bucket><Key>{}</Key><UploadId>{}</UploadId><PartNumberMarker>{marker}</PartNumberMarker>",
        xml_escape(&shared.bucket),
        xml_escape(key),
        xml_escape(id)
    );
    if let Some((last, _, _)) = parts.last() {
        out.push_str(&format!(
            "<NextPartNumberMarker>{last}</NextPartNumberMarker>"
        ));
    }
    out.push_str(&format!(
        "<MaxParts>{max_parts}</MaxParts><IsTruncated>{truncated}</IsTruncated>"
    ));
    let modified = iso8601(now_ms() / 1000);
    for (number, size, etag) in parts {
        out.push_str(&format!(
            "<Part><PartNumber>{number}</PartNumber><LastModified>{modified}</LastModified><ETag>{}</ETag><Size>{size}</Size></Part>",
            xml_escape(etag)
        ));
    }
    out.push_str("</ListPartsResult>");
    Answer::xml(200, out)
}

fn complete(shared: &Shared, head: &Head, key: &str, body: &[u8], resource: &str) -> Answer {
    let id = head.query("uploadId").unwrap_or("").to_string();
    let text = String::from_utf8_lossy(body);
    let mut listed = Vec::new();
    for part in elements(&text, "Part") {
        let number = elements(part, "PartNumber")
            .first()
            .and_then(|n| n.trim().parse::<u16>().ok());
        let etag = elements(part, "ETag").first().map(|e| xml_unescape(e));
        match (number, etag) {
            (Some(number), Some(etag)) => listed.push((number, etag)),
            _ => return Answer::error(400, "MalformedXML", resource),
        }
    }
    let r2_parts = shared.faults().r2_parts;
    let mut store = lock(&shared.store);
    let Some(upload) = store.uploads.get(&id).filter(|u| u.key == key) else {
        return Answer::error(404, "NoSuchUpload", resource);
    };
    if listed.is_empty() {
        return Answer::error(400, "MalformedXML", resource);
    }
    if listed.windows(2).any(|w| w[0].0 >= w[1].0) {
        return Answer::error(400, "InvalidPartOrder", resource);
    }
    let mut object = Vec::new();
    let mut sizes = Vec::new();
    for (number, tag) in &listed {
        let Some(bytes) = upload.parts.get(number) else {
            return Answer::error(400, "InvalidPart", resource);
        };
        if tag.trim_matches('"') != etag(bytes).trim_matches('"') {
            return Answer::error(400, "InvalidPart", resource);
        }
        sizes.push(bytes.len());
        object.extend_from_slice(bytes);
    }
    if r2_parts {
        let first = sizes[0];
        let (last, rest) = sizes.split_last().expect("at least one part");
        if rest.iter().any(|&size| size != first) || *last > first {
            return Answer::error(400, "InvalidPart", resource);
        }
    }
    let tag = etag(&object);
    if let Err(answer) = store.insert(shared, key, object) {
        return answer;
    }
    store.drop_upload(shared.dir.as_deref(), &id);
    let mut answer = Answer::xml(
        200,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Location>{}</Location><Bucket>{}</Bucket><Key>{}</Key><ETag>{}</ETag></CompleteMultipartUploadResult>",
            xml_escape(resource),
            xml_escape(&shared.bucket),
            xml_escape(key),
            xml_escape(&tag)
        ),
    );
    answer.headers.push(("ETag", tag));
    answer
}

fn abort(shared: &Shared, head: &Head, key: &str, resource: &str) -> Answer {
    let id = head.query("uploadId").unwrap_or("");
    let mut store = lock(&shared.store);
    if store.uploads.get(id).is_none_or(|u| u.key != key) {
        return Answer::error(404, "NoSuchUpload", resource);
    }
    store.drop_upload(shared.dir.as_deref(), id);
    Answer::new(204)
}

// ---- The store -------------------------------------------------------------------------------

#[derive(Default)]
struct Store {
    objects: BTreeMap<String, Vec<u8>>,
    uploads: BTreeMap<String, Upload>,
}

struct Upload {
    key: String,
    parts: BTreeMap<u16, Vec<u8>>,
}

impl Store {
    /// What an earlier fake left in `dir`; removes its temporary files.
    fn load(dir: &Path) -> io::Result<Store> {
        let mut store = Store::default();
        let objects = dir.join("objects");
        let uploads = dir.join("uploads");
        fs::create_dir_all(&objects)?;
        fs::create_dir_all(&uploads)?;
        let mut files = Vec::new();
        walk(&objects, &mut files)?;
        for file in files {
            let Ok(relative) = file.strip_prefix(&objects) else {
                continue;
            };
            let key = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            store.objects.insert(key, fs::read(&file)?);
        }
        for entry in fs::read_dir(&uploads)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".tmp-") {
                let _ = fs::remove_file(entry.path());
                continue;
            }
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Ok(key) = fs::read_to_string(entry.path().join("key")) else {
                continue;
            };
            let mut parts = BTreeMap::new();
            for part in fs::read_dir(entry.path())? {
                let part = part?;
                if let Ok(number) = part.file_name().to_string_lossy().parse::<u16>() {
                    parts.insert(number, fs::read(part.path())?);
                }
            }
            store.uploads.insert(name, Upload { key, parts });
        }
        Ok(store)
    }

    fn insert(&mut self, shared: &Shared, key: &str, bytes: Vec<u8>) -> Result<(), Answer> {
        if let Some(dir) = &shared.dir {
            let Some(path) = object_path(dir, key) else {
                return Err(Answer::error(400, "InvalidArgument", key));
            };
            write_file(shared, dir, &path, &bytes)
                .map_err(|_| Answer::error(500, "InternalError", key))?;
        }
        self.objects.insert(key.to_string(), bytes);
        Ok(())
    }

    fn remove(&mut self, shared: &Shared, key: &str) {
        if self.objects.remove(key).is_none() {
            return;
        }
        if let Some(dir) = &shared.dir {
            if let Some(path) = object_path(dir, key) {
                let _ = fs::remove_file(&path);
                // Leave no empty folders behind in objects/.
                let root = dir.join("objects");
                let mut parent = path.parent();
                while let Some(folder) = parent {
                    if folder == root || fs::remove_dir(folder).is_err() {
                        break;
                    }
                    parent = folder.parent();
                }
            }
        }
    }

    fn create(&mut self, shared: &Shared, id: &str, key: &str) -> Result<(), Answer> {
        if let Some(dir) = &shared.dir {
            let folder = dir.join("uploads").join(id);
            fs::create_dir_all(&folder)
                .and_then(|()| fs::write(folder.join("key"), key))
                .map_err(|_| Answer::error(500, "InternalError", key))?;
        }
        self.uploads.insert(
            id.to_string(),
            Upload {
                key: key.to_string(),
                parts: BTreeMap::new(),
            },
        );
        Ok(())
    }

    fn put_part(
        &mut self,
        shared: &Shared,
        id: &str,
        number: u16,
        bytes: Vec<u8>,
    ) -> Result<(), Answer> {
        if let Some(dir) = &shared.dir {
            let path = dir.join("uploads").join(id).join(number.to_string());
            write_file(shared, dir, &path, &bytes)
                .map_err(|_| Answer::error(500, "InternalError", id))?;
        }
        if let Some(upload) = self.uploads.get_mut(id) {
            upload.parts.insert(number, bytes);
        }
        Ok(())
    }

    fn drop_upload(&mut self, dir: Option<&Path>, id: &str) {
        self.uploads.remove(id);
        if let Some(dir) = dir {
            let _ = fs::remove_dir_all(dir.join("uploads").join(id));
        }
    }

    fn forget_uploads(&mut self, dir: Option<&Path>) {
        let ids: Vec<String> = self.uploads.keys().cloned().collect();
        for id in ids {
            self.drop_upload(dir, &id);
        }
    }
}

/// `<dir>/objects/<key>`, the key's `/` as folders; None for a key no file system path can hold.
fn object_path(dir: &Path, key: &str) -> Option<PathBuf> {
    let mut path = dir.join("objects");
    for segment in key.split('/') {
        let bad = segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment
                .chars()
                .any(|c| c.is_control() || "<>:\"\\|?*".contains(c));
        if bad {
            return None;
        }
        path.push(segment);
    }
    Some(path)
}

/// Writes through a temporary file in `uploads/`, so `objects/` only ever holds whole objects.
fn write_file(shared: &Shared, dir: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = dir
        .join("uploads")
        .join(format!(".tmp-{}-{}", std::process::id(), shared.next()));
    let result = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            walk(&entry.path(), files)?;
        } else {
            files.push(entry.path());
        }
    }
    Ok(())
}

// ---- Small helpers -----------------------------------------------------------------------------

/// `"<length hex>-<FNV-1a 64 hex>"`, quotes included.
fn etag(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("\"{:x}-{hash:016x}\"", bytes.len())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// `20261102T120000Z` as seconds since the epoch.
fn parse_amz_date(value: &str) -> Option<u64> {
    let b = value.as_bytes();
    if b.len() != 16 || b[8] != b'T' || b[15] != b'Z' {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<i64> { value.get(range)?.parse().ok() };
    let (year, month, day) = (num(0..4)?, num(4..6)?, num(6..8)?);
    let (hour, minute, second) = (num(9..11)?, num(11..13)?, num(13..15)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    u64::try_from(days * 86_400 + hour * 3600 + minute * 60 + second).ok()
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2026-11-02T12:00:00.000Z`.
fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let out: Option<Vec<u8>> = bytes
        .chunks(2)
        .map(|pair| Some(hex_value(pair[0])? << 4 | hex_value(pair[1])?))
        .collect();
    String::from_utf8(out?).ok()
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn xml_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The inner text of every `<tag>…</tag>` (or `<tag attr…>…</tag>`) in order, not nested.
fn elements<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let after = &rest[at + open.len()..];
        // `<Part>` but not `<PartNumber>`.
        if !after.starts_with('>') && !after.starts_with(' ') {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        found.push(&body[..end]);
        rest = &body[end + close.len()..];
    }
    found
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_convert_both_ways() {
        assert_eq!(parse_amz_date("19700101T000000Z"), Some(0));
        assert_eq!(parse_amz_date("20261102T120000Z"), Some(1_793_620_800));
        assert_eq!(iso8601(1_793_620_800), "2026-11-02T12:00:00.000Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00.000Z");
        assert_eq!(parse_amz_date("2026-11-02"), None);
    }

    #[test]
    fn ranges_follow_http() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-200", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=95-", 100), Some((95, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-200", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-4", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("bytes=0-0", 0), None);
    }

    #[test]
    fn xml_helpers_round_trip() {
        assert_eq!(xml_unescape(&xml_escape("a&b<\"c\">'")), "a&b<\"c\">'");
        assert_eq!(xml_unescape("&#34;x&#x22;"), "\"x\"");
        let body = "<C><Part><ETag>&quot;1&quot;</ETag><PartNumber>1</PartNumber></Part><Part><PartNumber>2</PartNumber><ETag>b</ETag></Part></C>";
        let parts = elements(body, "Part");
        assert_eq!(parts.len(), 2);
        assert_eq!(elements(parts[0], "PartNumber"), vec!["1"]);
        assert_eq!(xml_unescape(elements(parts[0], "ETag")[0]), "\"1\"");
    }

    #[test]
    fn percent_coding_round_trips() {
        assert_eq!(percent_decode("a%20b%2Fc%zz%"), "a b/c%zz%");
        assert_eq!(percent_encode("a b/ü"), "a%20b/%C3%BC");
        assert_eq!(
            unhex(&hex("rkv1/x/".as_bytes())).as_deref(),
            Some("rkv1/x/")
        );
    }

    #[test]
    fn etags_have_the_documented_shape() {
        assert_eq!(etag(b""), "\"0-cbf29ce484222325\"");
        assert_eq!(etag(b"a"), "\"1-af63dc4c8601ec8c\"");
    }
}
