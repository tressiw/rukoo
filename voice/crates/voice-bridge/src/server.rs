//! The WebSocket server of PROTOCOL.md on `127.0.0.1`.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{interval_at, sleep_until, timeout, timeout_at, Instant, MissedTickBehavior};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error as WsError, Message as Frame};
use tokio_tungstenite::WebSocketStream;
use voice_protocol::{
    close, new_token, replay_plan, validate_incoming, Command, CommandBody, ErrorCode, Hello,
    MeetingId, Message, Object, Refusal, Reply, ReplyError, Side, Welcome, BRIDGE_PATH, CLIP_WAIT,
    HELLO_TIMEOUT, MAX_CLIENTS, MAX_ID_CHARS, MAX_RUKOO_MESSAGE, MAX_VOICE_MESSAGE, MIN_PROTOCOL,
    PING_INTERVAL, PONG_TIMEOUT, PROTOCOL,
};

use crate::{ControlError, Discovery, EventLog, LogError, MeetingControl, Stored};

type Socket = WebSocketStream<TcpStream>;
type Sink = SplitSink<Socket, Frame>;
type Source = SplitStream<Socket>;

/// Events per read while replaying or catching up.
const PAGE: usize = 500;
/// Connections that may wait in their upgrade or `hello` at once. A new one closes the oldest,
/// so sockets that never say hello can neither pile up tasks nor keep Rukoo out: Rukoo says
/// hello as soon as it is upgraded.
const MAX_PENDING: usize = 16;
/// Commands a connection may queue while an earlier one runs.
const COMMAND_QUEUE: usize = 32;
/// How long a close waits for the client's close in return.
const CLOSE_GRACE: Duration = Duration::from_secs(1);
/// How long shutdown waits for connections to close before it drops them.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// WebSocket close code for a message over the size limit.
const CLOSE_TOO_BIG: u16 = 1009;
/// WebSocket close code when the event log fails under a connection.
const CLOSE_INTERNAL: u16 = 1011;

/// The server's timing. The defaults are PROTOCOL.md's; tests shorten them.
#[derive(Clone, Debug)]
pub struct Options {
    /// Time from connecting to a `hello`.
    pub hello_timeout: Duration,
    pub ping_interval: Duration,
    /// Time a ping waits for its pong before the connection closes.
    pub pong_timeout: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            hello_timeout: HELLO_TIMEOUT,
            ping_interval: PING_INTERVAL,
            pong_timeout: PONG_TIMEOUT,
        }
    }
}

/// A client that sent the token: what its `hello` said, and how far it has read since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    /// `client` from its hello, such as `rukoo`.
    pub client: String,
    pub client_version: Option<String>,
    /// Rukoo's process id from its hello, when it sent one: the process that owns its windows,
    /// which Voice leaves out of its screen recording.
    pub pid: Option<u64>,
    /// The protocol version the connection speaks.
    pub protocol: u32,
    /// `after` from its hello: the last event it had seen.
    pub after: u64,
    /// The highest sequence number the client has: sent on this connection, or `after` before
    /// the replay sent anything.
    pub sent: u64,
    /// Its `welcome` went out, so every event appended from now on reaches it after the replay,
    /// as a live event. An event appended between the `hello` and the `welcome` can fall inside
    /// the replay, which Rukoo reads as history.
    pub welcomed: bool,
    /// When it connected, in ms since the epoch.
    pub connected_at: u64,
}

/// A `hello` the server turned away for its token or its protocol version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub refusal: Refusal,
    /// The version the client asked for, when it was a whole number.
    pub protocol: Option<u64>,
    pub client: Option<String>,
    pub client_version: Option<String>,
    /// When, in ms since the epoch.
    pub at: u64,
}

/// Who is connected now, and the newest refused `hello`, for Settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clients {
    /// Oldest connection first.
    pub connected: Vec<Client>,
    /// The newest hello this server refused for its token or its version.
    pub refused: Option<Refused>,
    /// When a client with the token last connected, in ms since the epoch: a refusal before it
    /// is history.
    pub last_admitted_at: Option<u64>,
}

/// The connected clients, kept for [`Handle::clients`].
#[derive(Default)]
struct Registry {
    next: u64,
    connected: BTreeMap<u64, Arc<Entry>>,
    refused: Option<Refused>,
    last_admitted_at: Option<u64>,
}

struct Entry {
    client: Client,
    sent: AtomicU64,
    welcomed: AtomicBool,
}

/// Removes a connection from the registry when its task ends, also when it is aborted.
struct Registered {
    registry: Arc<Mutex<Registry>>,
    id: u64,
}

impl Drop for Registered {
    fn drop(&mut self) {
        lock(&self.registry).connected.remove(&self.id);
    }
}

fn lock(registry: &Mutex<Registry>) -> std::sync::MutexGuard<'_, Registry> {
    registry.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a refused hello said about its client, as far as it could be read.
#[derive(Default)]
struct Said {
    protocol: Option<u64>,
    client: Option<String>,
    client_version: Option<String>,
}

impl Said {
    fn of(value: &Value) -> Said {
        let text = |key: &str| value[key].as_str().map(str::to_owned);
        Said {
            protocol: value["protocol"].as_u64(),
            client: text("client"),
            client_version: text("clientVersion"),
        }
    }
}

/// Starts the bridge server.
pub struct BridgeServer;

impl BridgeServer {
    /// Listens on `127.0.0.1:0`, then writes the discovery file to `discovery_path`.
    ///
    /// The server runs on its own threads, so this can be called from anywhere, also from an
    /// async task. `version` is Voice's version, for `welcome` and the discovery file.
    pub fn start(
        log: EventLog,
        control: Arc<dyn MeetingControl>,
        discovery_path: &Path,
        version: &str,
    ) -> io::Result<Handle> {
        BridgeServer::start_with(log, control, discovery_path, version, Options::default())
    }

    /// [`BridgeServer::start`] with other timing.
    pub fn start_with(
        log: EventLog,
        control: Arc<dyn MeetingControl>,
        discovery_path: &Path,
        version: &str,
        options: Options,
    ) -> io::Result<Handle> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("voice-bridge")
            .enable_all()
            .build()?;
        match serve(&runtime, log, control, discovery_path, version, options) {
            Ok((port, discovery, shutdown, server, registry)) => Ok(Handle {
                port,
                discovery_path: discovery_path.to_owned(),
                discovery: Some(discovery),
                shutdown,
                server: Some(server),
                runtime: Some(runtime),
                registry,
            }),
            Err(e) => {
                // Dropping a runtime blocks, which panics inside an async task.
                runtime.shutdown_background();
                Err(e)
            }
        }
    }
}

type Started = (
    u16,
    Discovery,
    watch::Sender<bool>,
    JoinHandle<()>,
    Arc<Mutex<Registry>>,
);

fn serve(
    runtime: &Runtime,
    log: EventLog,
    control: Arc<dyn MeetingControl>,
    discovery_path: &Path,
    version: &str,
    options: Options,
) -> io::Result<Started> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let listener = {
        let _context = runtime.enter();
        TcpListener::from_std(listener)?
    };
    let token = new_token()?;
    let file = voice_protocol::Discovery {
        protocol: PROTOCOL,
        min_protocol: MIN_PROTOCOL,
        port,
        token: token.clone(),
        pid: std::process::id(),
        instance: log.instance().to_owned(),
        voice_version: version.to_owned(),
        started_at: now_ms(),
    };
    let registry = Arc::new(Mutex::new(Registry::default()));
    let shared = Arc::new(Shared {
        log,
        control,
        token,
        version: version.to_owned(),
        options,
        clients: Arc::new(Semaphore::new(MAX_CLIENTS)),
        registry: registry.clone(),
    });
    let (shutdown, quit) = watch::channel(false);
    let server = runtime.spawn(accept(listener, shared, quit));
    // The file comes last: Rukoo may connect the moment it appears.
    match Discovery::write(discovery_path, &file) {
        Ok(discovery) => Ok((port, discovery, shutdown, server, registry)),
        Err(e) => {
            let _ = shutdown.send(true);
            Err(e)
        }
    }
}

/// A running bridge server. Dropping it shuts the server down, as [`Handle::shutdown`] does.
pub struct Handle {
    port: u16,
    discovery_path: PathBuf,
    discovery: Option<Discovery>,
    shutdown: watch::Sender<bool>,
    server: Option<JoinHandle<()>>,
    runtime: Option<Runtime>,
    registry: Arc<Mutex<Registry>>,
}

impl Handle {
    pub fn protection_clients(&self) -> Vec<u64> {
        lock(&self.registry)
            .connected
            .iter()
            .filter(|(_, entry)| {
                entry.client.client == "rukoo" && entry.welcomed.load(Ordering::SeqCst)
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// The process ids of the `rukoo` clients that completed their `hello` and sent a pid,
    /// oldest connection first, without repeats: the processes whose windows a screen recording
    /// leaves out.
    pub fn rukoo_pids(&self) -> Vec<u32> {
        let registry = lock(&self.registry);
        let mut pids: Vec<u32> = Vec::new();
        for entry in registry.connected.values() {
            if entry.client.client != "rukoo" || !entry.welcomed.load(Ordering::SeqCst) {
                continue;
            }
            let pid = entry.client.pid.and_then(|pid| u32::try_from(pid).ok());
            if let Some(pid) = pid.filter(|&pid| pid != 0) {
                if !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
        pids
    }

    /// The clients connected now, and the newest hello refused for its token or version.
    pub fn clients(&self) -> Clients {
        let registry = lock(&self.registry);
        Clients {
            connected: registry
                .connected
                .values()
                .map(|entry| Client {
                    sent: entry.sent.load(Ordering::SeqCst),
                    welcomed: entry.welcomed.load(Ordering::SeqCst),
                    ..entry.client.clone()
                })
                .collect(),
            refused: registry.refused.clone(),
            last_admitted_at: registry.last_admitted_at,
        }
    }

    /// The port on `127.0.0.1`.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn discovery_path(&self) -> &Path {
        &self.discovery_path
    }

    /// Removes the discovery file, closes every connection with 1001 and stops the server.
    /// Waits about two seconds at most.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        // The file goes first, so Rukoo doesn't reconnect to a server that is going away.
        drop(self.discovery.take());
        let _ = self.shutdown.send(true);
        if let Some(server) = self.server.take() {
            let (done, finished) = std_mpsc::channel();
            runtime.spawn(async move {
                let _ = server.await;
                let _ = done.send(());
            });
            let _ = finished.recv_timeout(SHUTDOWN_GRACE + CLOSE_GRACE);
        }
        runtime.shutdown_background();
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Shared {
    log: EventLog,
    control: Arc<dyn MeetingControl>,
    token: String,
    version: String,
    options: Options,
    clients: Arc<Semaphore>,
    registry: Arc<Mutex<Registry>>,
}

impl Shared {
    /// Keeps a client that was admitted, until its connection ends.
    fn register(&self, hello: &Hello) -> (Arc<Entry>, Registered) {
        let entry = Arc::new(Entry {
            client: Client {
                client: hello.client.clone(),
                client_version: hello.client_version.clone(),
                pid: hello.pid,
                protocol: hello.protocol,
                after: hello.after,
                sent: hello.after,
                welcomed: false,
                connected_at: now_ms(),
            },
            sent: AtomicU64::new(hello.after),
            welcomed: AtomicBool::new(false),
        });
        let mut registry = lock(&self.registry);
        registry.last_admitted_at = Some(entry.client.connected_at);
        registry.next += 1;
        let id = registry.next;
        registry.connected.insert(id, entry.clone());
        let registered = Registered {
            registry: self.registry.clone(),
            id,
        };
        (entry, registered)
    }

    /// Remembers a hello refused for its token or version, so Settings can say why Rukoo isn't
    /// connected. Other refusals don't come from a client that said hello.
    fn note_refusal(&self, refusal: Refusal, said: Option<Said>) {
        let Some(said) = said else { return };
        if !matches!(refusal, Refusal::BadToken | Refusal::UnsupportedProtocol) {
            return;
        }
        lock(&self.registry).refused = Some(Refused {
            refusal,
            protocol: said.protocol,
            client: said.client,
            client_version: said.client_version,
            at: now_ms(),
        });
    }
}

async fn accept(listener: TcpListener, shared: Arc<Shared>, mut quit: watch::Receiver<bool>) {
    let mut connections = JoinSet::new();
    // One per connection still in its upgrade or `hello`, oldest first. A connection drops its
    // end when it gets past `hello` or goes away.
    let mut pending: VecDeque<oneshot::Sender<()>> = VecDeque::new();
    loop {
        tokio::select! {
            _ = quit.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((tcp, _)) => {
                    pending.retain(|waiting| !waiting.is_closed());
                    if pending.len() >= MAX_PENDING {
                        if let Some(oldest) = pending.pop_front() {
                            let _ = oldest.send(());
                        }
                    }
                    let (evict, evicted) = oneshot::channel();
                    pending.push_back(evict);
                    connections.spawn(connection(tcp, shared.clone(), evicted, quit.clone()));
                }
                // Out of sockets or handles; give the system a moment instead of spinning.
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    drop(listener);
    let closing = async { while connections.join_next().await.is_some() {} };
    let _ = timeout(SHUTDOWN_GRACE, closing).await;
    connections.abort_all();
}

async fn connection(
    tcp: TcpStream,
    shared: Arc<Shared>,
    mut evicted: oneshot::Receiver<()>,
    quit: watch::Receiver<bool>,
) {
    let _ = tcp.set_nodelay(true);
    // The time for `hello` runs from the connection, so a slow upgrade counts too.
    let deadline = Instant::now() + shared.options.hello_timeout;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_RUKOO_MESSAGE))
        .max_frame_size(Some(MAX_RUKOO_MESSAGE));
    let upgrade = tokio_tungstenite::accept_hdr_async_with_config(tcp, check_upgrade, Some(config));
    // Until `hello`, a newer connection may need the place: then this one just ends.
    let socket = tokio::select! {
        _ = &mut evicted => return,
        upgraded = timeout_at(deadline, upgrade) => match upgraded {
            Ok(Ok(socket)) => socket,
            _ => return,
        },
    };
    let (sink, mut source) = socket.split();
    let first = tokio::select! {
        _ = &mut evicted => return,
        first = timeout_at(deadline, first_message(&mut source)) => first,
    };
    let admitted = match first {
        Err(_) => Err((Refusal::HelloTimeout, None)),
        Ok(First::Closed) => return,
        Ok(First::Refused(refusal, said)) => Err((refusal, said)),
        Ok(First::Hello(hello)) => admit(&shared, hello),
    };
    drop(evicted);
    match admitted {
        Err((refusal, said)) => {
            shared.note_refusal(refusal, said);
            refuse(sink, source, refusal).await
        }
        Ok((hello, _slot)) => {
            let (entry, _registered) = shared.register(&hello);
            Session::new(shared, sink, quit, entry, _registered.id)
                .run(hello, source)
                .await
        }
    }
}

/// Refuses a browser's upgrade request and anything not on `/bridge`.
// tungstenite's `Callback` fixes this signature, large error and all.
#[allow(clippy::result_large_err)]
fn check_upgrade(request: &Request, response: Response) -> Result<Response, ErrorResponse> {
    if request.uri().path() != BRIDGE_PATH {
        return Err(refuse_upgrade(StatusCode::NOT_FOUND));
    }
    // Any web page can try to open a socket on 127.0.0.1, and a browser sends an Origin with
    // every WebSocket request: `null` from a sandboxed frame or a file, `file://` from Chrome.
    // Rukoo connects from its main process, which sends none, so any Origin is a page.
    if request.headers().contains_key("origin") {
        return Err(refuse_upgrade(StatusCode::FORBIDDEN));
    }
    Ok(response)
}

fn refuse_upgrade(status: StatusCode) -> ErrorResponse {
    let mut response = ErrorResponse::new(None);
    *response.status_mut() = status;
    response
}

enum First {
    Hello(Hello),
    /// With what the hello said, when the first message was one.
    Refused(Refusal, Option<Said>),
    Closed,
}

async fn first_message(source: &mut Source) -> First {
    while let Some(frame) = source.next().await {
        let text = match frame {
            Ok(Frame::Text(text)) => text,
            Ok(Frame::Ping(_) | Frame::Pong(_) | Frame::Frame(_)) => continue,
            Ok(Frame::Binary(_)) => return First::Refused(Refusal::HelloTimeout, None),
            Ok(Frame::Close(_)) | Err(_) => return First::Closed,
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return First::Refused(Refusal::HelloTimeout, None);
        };
        return match validate_incoming(Side::Rukoo, &value) {
            Ok(Message::Hello(hello)) => First::Hello(hello),
            // A hello with a malformed token or version gets the answer a wrong one gets.
            Err(invalid) if value["type"] == "hello" => First::Refused(
                match invalid.reason.as_str() {
                    "token" => Refusal::BadToken,
                    "protocol" => Refusal::UnsupportedProtocol,
                    _ => Refusal::HelloTimeout,
                },
                Some(Said::of(&value)),
            ),
            Ok(_) | Err(_) => First::Refused(Refusal::HelloTimeout, None),
        };
    }
    First::Closed
}

type Admitted = Result<(Hello, OwnedSemaphorePermit), (Refusal, Option<Said>)>;

fn admit(shared: &Shared, hello: Hello) -> Admitted {
    let said = || {
        Some(Said {
            protocol: Some(u64::from(hello.protocol)),
            client: Some(hello.client.clone()),
            client_version: hello.client_version.clone(),
        })
    };
    let token_matches: bool = hello.token.as_bytes().ct_eq(shared.token.as_bytes()).into();
    if !token_matches {
        return Err((Refusal::BadToken, said()));
    }
    if !(MIN_PROTOCOL..=PROTOCOL).contains(&hello.protocol) {
        return Err((Refusal::UnsupportedProtocol, said()));
    }
    // Only a client with the token takes a slot, so a program without it can't keep Rukoo out.
    let slot = shared
        .clients
        .clone()
        .try_acquire_owned()
        .map_err(|_| (Refusal::TooManyClients, said()))?;
    Ok((hello, slot))
}

async fn refuse(mut sink: Sink, source: Source, refusal: Refusal) {
    let error = Message::Error(refusal.message()).to_json();
    if sink.send(Frame::text(error)).await.is_ok() {
        close(sink, source, refusal.close_code(), "").await;
    }
}

/// Sends a close frame and waits briefly for the client's, so the client reads ours before the
/// TCP connection ends.
async fn close(mut sink: Sink, mut source: Source, code: u16, reason: &'static str) {
    let frame = CloseFrame {
        code: CloseCode::from(code),
        reason: reason.into(),
    };
    if sink.send(Frame::Close(Some(frame))).await.is_err() {
        return;
    }
    let drain = async { while let Some(Ok(_)) = source.next().await {} };
    let _ = timeout(CLOSE_GRACE, drain).await;
}

/// How a session ended.
enum End {
    Quit,
    TooBig,
    LogFailed,
    ClientClosed,
    Dropped,
}

/// A connection after a good `hello`.
struct Session {
    shared: Arc<Shared>,
    sink: Sink,
    quit: watch::Receiver<bool>,
    /// The highest sequence number sent on this connection.
    last: u64,
    client: Arc<Entry>,
    connection: u64,
}

impl Session {
    fn new(
        shared: Arc<Shared>,
        sink: Sink,
        quit: watch::Receiver<bool>,
        client: Arc<Entry>,
        connection: u64,
    ) -> Session {
        Session {
            shared,
            sink,
            quit,
            last: 0,
            client,
            connection,
        }
    }

    /// The client has every event up to `seq`.
    fn reached(&mut self, seq: u64) {
        self.last = seq;
        self.client.sent.store(seq, Ordering::SeqCst);
    }

    async fn run(mut self, hello: Hello, mut source: Source) {
        let end = match self.serve(hello, &mut source).await {
            Ok(end) => end,
            Err(end) => end,
        };
        let Session { sink, .. } = self;
        match end {
            End::Quit => close(sink, source, close::QUITTING, "Voice is quitting").await,
            End::TooBig => close(sink, source, CLOSE_TOO_BIG, "").await,
            End::LogFailed => close(sink, source, CLOSE_INTERNAL, "").await,
            End::ClientClosed => {
                // tungstenite answered the client's close already; send it on its way.
                let mut sink = sink;
                let _ = sink.close().await;
            }
            End::Dropped => {}
        }
    }

    async fn serve(&mut self, hello: Hello, source: &mut Source) -> Result<End, End> {
        // Subscribe before reading the log: an event appended meanwhile then arrives both ways,
        // and `last` drops the second copy, instead of falling between the two.
        let mut live = self.shared.log.subscribe();
        let log = self.shared.log.clone();
        let (head, oldest) = blocking(move || log.bounds()).await?;
        let instance = self.shared.log.instance();
        let (replay, from) = replay_plan(
            hello.instance.as_deref(),
            hello.after,
            instance,
            head,
            oldest,
        );
        let control = self.shared.control.clone();
        let status = tokio::task::spawn_blocking(move || control.status())
            .await
            .unwrap_or_default();
        let welcome = Welcome {
            protocol: hello.protocol,
            voice_version: self.shared.version.clone(),
            instance: instance.to_owned(),
            head,
            oldest,
            replay,
            status,
        };
        self.send(Message::Welcome(welcome).to_json()).await?;
        self.client.welcomed.store(true, Ordering::SeqCst);
        self.reached(from.saturating_sub(1));
        self.catch_up().await?;

        let (jobs, queue) = mpsc::channel(COMMAND_QUEUE);
        // Unbounded, so the worker never waits on this loop while this loop waits on the worker.
        let (replies_out, mut replies) = mpsc::unbounded_channel();
        let worker = tokio::spawn(run_commands(
            self.shared.control.clone(),
            queue,
            replies_out,
        ));
        let end = self.live(&mut live, source, &jobs, &mut replies).await;
        worker.abort();
        end
    }

    async fn live(
        &mut self,
        live: &mut tokio::sync::broadcast::Receiver<Stored>,
        source: &mut Source,
        jobs: &mpsc::Sender<Job>,
        replies: &mut mpsc::UnboundedReceiver<String>,
    ) -> Result<End, End> {
        let options = &self.shared.options;
        let mut ping = interval_at(
            Instant::now() + options.ping_interval,
            options.ping_interval,
        );
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let pong_timeout = options.pong_timeout;
        let mut pong_due: Option<Instant> = None;
        if *self.quit.borrow() {
            return Ok(End::Quit);
        }
        loop {
            tokio::select! {
                _ = self.quit.changed() => return Ok(End::Quit),
                frame = source.next() => match frame {
                    Some(Ok(Frame::Text(text))) => {
                        // Stamped before the job waits in the queue, so a command's deadline
                        // counts the time it spends behind earlier ones.
                        let received = std::time::Instant::now();
                        if let Some(job) = read_command(&text, received) {
                            // screen.set waits for all clients, including this one. An ack
                            // behind it in the ordinary queue would deadlock that protection.
                            match job {
                                Job::Run(Command { id, body: CommandBody::MeetingProtected { meeting, request } }, _) => {
                                    let answer = self.shared.control.protected_for(meeting.as_str(), request.as_deref(), self.connection);
                                    let reply = match answer {
                                        Ok(()) => Reply::ok(id, Object::new()),
                                        Err(error) => Reply::error(id, reply_error(error)),
                                    };
                                    self.send(within_limit(reply)).await?;
                                }
                                job => jobs.send(job).await.map_err(|_| End::Dropped)?,
                            }
                        }
                    }
                    Some(Ok(Frame::Pong(_))) => pong_due = None,
                    // tungstenite answers pings; binary frames mean nothing here.
                    Some(Ok(Frame::Ping(_) | Frame::Binary(_) | Frame::Frame(_))) => {}
                    Some(Ok(Frame::Close(_))) | None => return Ok(End::ClientClosed),
                    Some(Err(WsError::Capacity(_))) => return Ok(End::TooBig),
                    Some(Err(_)) => return Ok(End::Dropped),
                },
                received = live.recv() => match received {
                    Ok(stored) if stored.seq <= self.last => {}
                    Ok(stored) if stored.seq == self.last + 1 => {
                        self.last = stored.seq;
                        self.send(stored.to_json()).await?;
                        self.reached(stored.seq);
                    }
                    // A burst outran the channel, or an event went missing in it: the log has
                    // every event that still exists.
                    Ok(_) | Err(RecvError::Lagged(_)) => self.catch_up().await?,
                    Err(RecvError::Closed) => return Ok(End::Dropped),
                },
                Some(reply) = replies.recv() => self.send(reply).await?,
                _ = ping.tick() => {
                    if pong_due.is_none() {
                        self.sink
                            .send(Frame::Ping(Default::default()))
                            .await
                            .map_err(|_| End::Dropped)?;
                        pong_due = Some(Instant::now() + pong_timeout);
                    }
                }
                _ = sleep_until(pong_due.unwrap_or_else(Instant::now)), if pong_due.is_some() => {
                    // No answer: the peer is gone or stuck. A close frame would wait for it too.
                    return Ok(End::Dropped);
                }
            }
        }
    }

    /// Sends every stored event after `last`, page by page.
    async fn catch_up(&mut self) -> Result<(), End> {
        loop {
            let log = self.shared.log.clone();
            let after = self.last;
            let page = blocking(move || log.page(after, PAGE)).await?;
            let more = page.len() == PAGE;
            for stored in page {
                self.sink
                    .feed(Frame::text(stored.to_json()))
                    .await
                    .map_err(|_| End::Dropped)?;
                self.last = stored.seq;
            }
            self.sink.flush().await.map_err(|_| End::Dropped)?;
            let last = self.last;
            self.reached(last);
            if !more {
                return Ok(());
            }
        }
    }

    async fn send(&mut self, json: String) -> Result<(), End> {
        self.sink
            .send(Frame::text(json))
            .await
            .map_err(|_| End::Dropped)
    }
}

/// Runs an event log call on a blocking thread.
async fn blocking<T, F>(call: F) -> Result<T, End>
where
    F: FnOnce() -> Result<T, LogError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) | Err(_) => Err(End::LogFailed),
    }
}

/// One entry in a connection's command queue. Answers to bad commands queue too, so every
/// reply leaves in the order its command came in.
enum Job {
    /// A command and the moment the server read its frame.
    Run(Command, std::time::Instant),
    Answer(Reply),
}

/// The job for a text frame read at `received`, or `None` when there is nothing to answer.
fn read_command(text: &str, received: std::time::Instant) -> Option<Job> {
    let value: Value = serde_json::from_str(text).ok()?;
    match validate_incoming(Side::Rukoo, &value) {
        Ok(Message::Command(command)) => Some(Job::Run(command, received)),
        // A second hello.
        Ok(_) => None,
        Err(invalid) => {
            if value["type"] != "command" {
                return None;
            }
            // A bad command is answered when its id can be read.
            let id = value["id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.chars().count() <= MAX_ID_CHARS)?;
            let message = format!("The command's `{}` is missing or invalid.", invalid.reason);
            Some(Job::Answer(Reply::error(
                id,
                ReplyError::new(ErrorCode::BadRequest, message),
            )))
        }
    }
}

async fn run_commands(
    control: Arc<dyn MeetingControl>,
    mut queue: mpsc::Receiver<Job>,
    replies: mpsc::UnboundedSender<String>,
) {
    while let Some(job) = queue.recv().await {
        let reply = match job {
            Job::Answer(reply) => reply,
            Job::Run(Command { id, body }, received) => {
                let control = control.clone();
                let run = move || dispatch(control.as_ref(), body, received);
                match tokio::task::spawn_blocking(run).await {
                    Ok(Ok(result)) => Reply::ok(id, result),
                    Ok(Err(error)) => Reply::error(id, reply_error(error)),
                    Err(_) => Reply::error(
                        id,
                        ReplyError::new(
                            ErrorCode::Failed,
                            "Voice failed while running the command.",
                        ),
                    ),
                }
            }
        };
        if replies.send(within_limit(reply)).is_err() {
            return;
        }
    }
}

/// Runs one command. `received` is when the server read it: `clip.get`, `screen.get` and
/// `screen.select`, the commands that wait for a cut or the screen recorder, answer by
/// [`CLIP_WAIT`](voice_protocol::CLIP_WAIT) after that, however long they waited in the queue.
fn dispatch(
    control: &dyn MeetingControl,
    body: CommandBody,
    received: std::time::Instant,
) -> Result<Object, ControlError> {
    let deadline = received + CLIP_WAIT;
    let result = match body {
        CommandBody::MeetingStart(args) => json!({ "meeting": control.start(args)? }),
        CommandBody::MeetingStop { meeting } => {
            control.stop(meeting.as_str())?;
            json!({})
        }
        CommandBody::MeetingDelete { meeting } => {
            control.delete(meeting.as_str())?;
            json!({})
        }
        CommandBody::MeetingPause { meeting } => {
            control.pause(meeting.as_str())?;
            json!({})
        }
        CommandBody::MeetingResume { meeting } => {
            control.resume(meeting.as_str())?;
            json!({})
        }
        CommandBody::MeetingMark { meeting, label } => {
            let (mark, at) = control.mark(meeting.as_str(), label.as_deref())?;
            json!({ "mark": mark, "at": at })
        }
        CommandBody::MeetingList { limit } => {
            let limit = usize::try_from(limit).unwrap_or(usize::MAX);
            json!({ "meetings": control.list(limit) })
        }
        CommandBody::MeetingGet { meeting } => control.get(meeting.as_str())?,
        CommandBody::FrameGet { meeting, at } => control.frame(meeting.as_str(), at)?,
        CommandBody::MediaOpen(args) => control.media_open(&args)?,
        CommandBody::MediaRead {
            media,
            offset,
            length,
        } => control.media_read(&media, offset, length)?,
        CommandBody::StatusGet => json!(control.status()),
        CommandBody::ClipGet {
            meeting,
            start,
            end,
        } => control.clip(meeting.as_str(), start, end, deadline)?,
        CommandBody::MeetingProtected { meeting, .. } => {
            control.protected(meeting.as_str())?;
            json!({})
        }
        CommandBody::ScreenGet { meeting } => control.screen(meeting.as_str(), deadline)?,
        CommandBody::ScreenSelect { meeting, monitor } => {
            json!({ "monitor": control.select_monitor(meeting.as_str(), &monitor, deadline)? })
        }
        CommandBody::ScreenSet {
            meeting,
            screen,
            monitor,
        } => control.set_screen(
            meeting.as_str(),
            screen,
            monitor.as_deref(),
            received + std::time::Duration::from_secs(15),
        )?,
        CommandBody::Unknown { command, .. } => {
            return Err(ControlError::new(
                ErrorCode::UnknownCommand,
                format!("Voice doesn't know the command `{command}`."),
            ))
        }
    };
    match result {
        Value::Object(result) => Ok(result),
        _ => Err(ControlError::failed(
            "Voice built a result that isn't an object.",
        )),
    }
}

fn reply_error(error: ControlError) -> ReplyError {
    ReplyError {
        code: error.code.as_str().to_owned(),
        message: error.message,
        // A malformed id would make Rukoo drop the whole reply; the code alone still helps.
        meeting: error.meeting.as_deref().and_then(MeetingId::parse),
    }
}

/// The reply as JSON, or a `failed` reply when it is over the 8 MiB Voice may send.
fn within_limit(reply: Reply) -> String {
    let json = Message::Reply(reply.clone()).to_json();
    if json.len() <= MAX_VOICE_MESSAGE {
        return json;
    }
    let message = format!(
        "The reply would be {} bytes, more than a message may hold.",
        json.len()
    );
    Message::Reply(Reply::error(
        reply.id,
        ReplyError::new(ErrorCode::Failed, message),
    ))
    .to_json()
}

fn now_ms() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_commands_with_a_readable_id_get_bad_request() {
        let now = std::time::Instant::now();
        let Some(Job::Answer(reply)) = read_command(
            r#"{"type":"command","id":"c-1","command":"meeting.stop","args":{}}"#,
            now,
        ) else {
            panic!("no answer");
        };
        assert_eq!(reply.id, "c-1");
        assert_eq!(reply.outcome.unwrap_err().code, "bad_request");
        assert!(read_command(r#"{"type":"command","command":"meeting.stop"}"#, now).is_none());
        assert!(read_command("not json", now).is_none());
        assert!(read_command(r#"{"type":"greeting"}"#, now).is_none());
    }

    #[test]
    fn a_command_keeps_the_moment_it_was_read() {
        let now = std::time::Instant::now();
        let received = now.checked_sub(Duration::from_secs(3)).unwrap_or(now);
        let Some(Job::Run(command, stamped)) = read_command(
            r#"{"type":"command","id":"c-1","command":"status.get","args":{}}"#,
            received,
        ) else {
            panic!("no command");
        };
        assert_eq!(command.id, "c-1");
        assert_eq!(stamped, received);
    }

    #[test]
    fn oversized_replies_become_failures() {
        let mut result = Object::new();
        result.insert("blob".into(), "x".repeat(MAX_VOICE_MESSAGE).into());
        let json = within_limit(Reply::ok("c-9", result));
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["id"], "c-9");
        assert_eq!(value["error"]["code"], "failed");
    }
}
