//! Hosts the bridge for Rukoo (voice/PROTOCOL.md): the server, `MeetingControl` over the
//! controller and the meeting library, and what Settings > Rukoo shows about the client.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use serde_json::Value;
use voice_bridge::{
    BridgeServer, Clients, ControlError, EventLog, Handle, MediaOpenArgs, MeetingControl, Refused,
};
use voice_protocol::{MeetingId, MeetingSummary, Refusal, StartArgs, StartedBy, Status};
use voice_screen_files::frames::FrameIndex;
use voice_screen_files::media::{self, ClipCheck, MediaError};
use voice_screen_files::{paths, policy};

use super::controller::Controller;
use super::screen::ScreenSupervisor;
use crate::media::MediaService;

/// What Rukoo's commands do.
pub struct Control {
    pub controller: Arc<Controller>,
    pub log: EventLog,
    pub dictating: Arc<AtomicBool>,
    pub screen: Arc<ScreenSupervisor>,
    /// Each meeting's frame index, read again when its journal grew.
    frames: Mutex<HashMap<String, (u64, Arc<FrameIndex>)>>,
    /// `media.open` and `media.read`.
    pub media: Arc<MediaService>,
    deleting: Mutex<()>,
}

impl Control {
    pub fn new(
        controller: Arc<Controller>,
        screen: Arc<ScreenSupervisor>,
        log: EventLog,
        dictating: Arc<AtomicBool>,
        media: Arc<MediaService>,
    ) -> Control {
        Control {
            controller,
            log,
            dictating,
            screen,
            frames: Mutex::new(HashMap::new()),
            media,
            deleting: Mutex::new(()),
        }
    }

    /// A folder renamed out of the library is a durable deletion intent. Finish it after a
    /// crash before the bridge accepts clients; only validated ids under the library qualify.
    pub fn recover_deletions(&self) {
        let Ok(entries) = fs::read_dir(self.controller.library().root()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_prefix(".deleted-"))
            else {
                continue;
            };
            if MeetingId::parse(id).is_some() {
                if let Err(error) = self.delete_meeting(id) {
                    log::warn!("meeting: {id}: couldn't finish deletion: {}", error.message);
                }
            }
        }
    }

    fn delete_meeting(&self, id: &str) -> Result<(), ControlError> {
        let _deleting = self.deleting.lock().unwrap_or_else(PoisonError::into_inner);
        if MeetingId::parse(id).is_none() {
            return Err(ControlError::not_found("There is no meeting with that id."));
        }
        let root = self.controller.library().root();
        let source = root.join(id);
        let pending = root.join(format!(".deleted-{id}"));
        if !pending.is_dir() {
            let snapshot = self
                .controller
                .library()
                .get(id, &self.log)
                .ok_or_else(|| ControlError::not_found("There is no meeting with that id."))?;
            if self.controller.current().is_some_and(|r| r.id == id)
                || !matches!(
                    snapshot["meeting"]["state"].as_str(),
                    Some("done" | "failed")
                )
            {
                return Err(ControlError::new(
                    voice_protocol::ErrorCode::Busy,
                    "Stop the meeting and let it finish saving before deleting it.",
                ));
            }
        }
        if !media::forget_meeting(&source, || self.media.forget_meeting(id))? {
            return Err(ControlError::new(voice_protocol::ErrorCode::Busy, "A video clip is still being prepared. Try deleting the meeting again when it finishes."));
        }
        if let Some(sync) = self.media.bucket_sync() {
            sync.mark_deleted(id)
                .map_err(|e| ControlError::failed(e.message))?;
        }
        if !pending.is_dir() {
            fs::rename(&source, &pending).map_err(|e| {
                ControlError::failed(format!(
                    "Voice couldn't remove the meeting from its library: {e}"
                ))
            })?;
        }
        self.log
            .remove_meeting_by_user(id)
            .map_err(|e| ControlError::failed(e.to_string()))?;
        fs::remove_dir_all(&pending).map_err(|e| {
            ControlError::failed(format!("Voice couldn't finish deleting the meeting: {e}"))
        })?;
        self.frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        Ok(())
    }

    pub fn status(&self) -> Status {
        Status {
            recording: self
                .controller
                .current()
                .and_then(|r| MeetingId::parse(&r.id)),
            dictating: self.dictating.load(Ordering::SeqCst),
            screen_default: Some(self.controller.screen_default()),
        }
    }

    fn meeting_dir(&self, id: &str) -> Result<PathBuf, ControlError> {
        self.controller
            .library()
            .dir(id)
            .map(|dir| dir.path().to_path_buf())
            .ok_or_else(|| ControlError::not_found("There is no meeting with that id."))
    }

    /// The meeting's frame index, read again only when its journal grew.
    fn frame_index(&self, id: &str, dir: &Path) -> Result<Arc<FrameIndex>, ControlError> {
        let length = fs::metadata(paths::journal_path(dir)).map_or(0, |m| m.len());
        let mut cache = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((known, index)) = cache.get(id) {
            if *known == length {
                return Ok(Arc::clone(index));
            }
        }
        let index =
            Arc::new(FrameIndex::load(dir).map_err(|e| {
                ControlError::failed(format!("The screen journal can't be read: {e}"))
            })?);
        // A handful of meetings is plenty: Rukoo asks about the one it shows.
        if cache.len() >= 8 {
            cache.clear();
        }
        cache.insert(id.to_string(), (length, Arc::clone(&index)));
        Ok(index)
    }
}

fn media_error(error: MediaError) -> ControlError {
    match error {
        MediaError::NotFound(message) => ControlError::not_found(message),
        MediaError::NotAvailable(message) => ControlError::not_available(message),
        MediaError::Failed(message) => ControlError::failed(message),
    }
}

impl MeetingControl for Control {
    fn delete(&self, id: &str) -> Result<(), ControlError> {
        self.delete_meeting(id)
    }
    fn start(&self, args: StartArgs) -> Result<String, ControlError> {
        self.controller.start(args, StartedBy::Rukoo)
    }

    fn stop(&self, id: &str) -> Result<(), ControlError> {
        self.controller.stop(Some(id), "rukoo")
    }

    fn pause(&self, id: &str) -> Result<(), ControlError> {
        self.controller.pause(Some(id))
    }

    fn resume(&self, id: &str) -> Result<(), ControlError> {
        self.controller.resume(Some(id))
    }

    fn mark(&self, id: &str, label: Option<&str>) -> Result<(String, u64), ControlError> {
        self.controller.mark(Some(id), label)
    }

    fn list(&self, limit: usize) -> Vec<MeetingSummary> {
        self.controller.library().list(limit)
    }

    fn get(&self, id: &str) -> Result<Value, ControlError> {
        let mut snapshot = self
            .controller
            .library()
            .get(id, &self.log)
            .ok_or_else(|| ControlError::not_found("There is no meeting with that id."))?;
        snapshot["meeting"]["initialScreen"] = snapshot["meeting"]["screen"].clone();
        snapshot["meeting"]["screen"] =
            serde_json::to_value(self.screen.mode_of(id)).unwrap_or(Value::Null);
        Ok(snapshot)
    }

    fn frame(&self, id: &str, at: u64) -> Result<Value, ControlError> {
        let dir = self.meeting_dir(id)?;
        let (frames, _) = media::recorded(&dir);
        if !frames {
            return Err(ControlError::not_available(
                "The meeting didn't record the screen.",
            ));
        }
        let index = self.frame_index(id, &dir)?;
        media::frame_reply(&dir, frames, at, &index).map_err(media_error)
    }

    fn clip(
        &self,
        id: &str,
        start: u64,
        end: u64,
        deadline: Instant,
    ) -> Result<Value, ControlError> {
        let dir = self.meeting_dir(id)?;
        let (_, video) = media::recorded(&dir);
        // A cut never eats into the free space the screen recording keeps (RUK-108 2.6). Its
        // input is known only once the journal is read and the video files are indexed, which
        // takes a while for a long meeting on a slow disk, so the check runs on the cut's thread
        // and this answers by `deadline` all the same.
        let screen = Arc::clone(&self.screen);
        let controller = Arc::clone(&self.controller);
        let check: ClipCheck = Box::new(move |bytes| match screen.free_bytes() {
            Some(free) if !policy::reserve_ok(free, bytes, controller.current().is_some()) => Err(
                MediaError::Failed("Not enough free disk space to cut a clip.".into()),
            ),
            _ => Ok(()),
        });
        media::clip_reply(&dir, video, start, end, deadline, Some(check)).map_err(media_error)
    }

    fn protected(&self, id: &str) -> Result<(), ControlError> {
        self.meeting_dir(id)?;
        self.screen.ack(id);
        Ok(())
    }

    fn protected_for(
        &self,
        id: &str,
        request: Option<&str>,
        connection: u64,
    ) -> Result<(), ControlError> {
        self.meeting_dir(id)?;
        match request {
            Some(request) => self.screen.ack_mode(id, request, connection),
            None => self.screen.ack(id),
        }
        Ok(())
    }

    fn set_screen(
        &self,
        id: &str,
        mode: voice_protocol::ScreenMode,
        monitor: Option<&str>,
        deadline: Instant,
    ) -> Result<Value, ControlError> {
        self.screen.set_mode(id, mode, monitor, deadline)
    }

    fn screen(&self, id: &str, deadline: Instant) -> Result<Value, ControlError> {
        self.screen.describe(id, deadline)
    }

    fn select_monitor(
        &self,
        id: &str,
        monitor: &str,
        deadline: Instant,
    ) -> Result<String, ControlError> {
        self.screen.select(id, monitor, deadline)
    }

    fn status(&self) -> Status {
        Control::status(self)
    }

    fn media_open(&self, args: &MediaOpenArgs) -> Result<Value, ControlError> {
        self.media.open(args)
    }

    fn media_read(&self, media: &str, offset: u64, length: u64) -> Result<Value, ControlError> {
        self.media.read(media, offset, length)
    }
}

/// Settings > Rukoo: who is connected, or why nobody is (voice/ui/API.md `rukoo`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RukooInfo {
    /// `connected`, `notRunning`, `rukooOutdated`, `voiceOutdated` or `refused`.
    pub state: &'static str,
    pub client_version: Option<String>,
    pub protocol: Option<u32>,
    /// The log's head minus the last event Rukoo has.
    pub behind: Option<u64>,
}

impl RukooInfo {
    pub fn connected(&self) -> bool {
        self.state == "connected"
    }
}

/// What the bridge's clients say about Rukoo. Pure, so it is tested without a server.
pub fn rukoo_info(clients: &Clients, head: u64) -> RukooInfo {
    let rukoo = clients
        .connected
        .iter()
        .filter(|c| c.client == "rukoo")
        .max_by_key(|c| c.connected_at);
    if let Some(client) = rukoo {
        return RukooInfo {
            state: "connected",
            client_version: client.client_version.clone().filter(|v| !v.is_empty()),
            protocol: Some(client.protocol),
            behind: Some(head.saturating_sub(client.sent)),
        };
    }
    // A refusal only counts until a client gets in again.
    let refused = clients
        .refused
        .as_ref()
        .filter(|r| clients.last_admitted_at.is_none_or(|at| r.at > at));
    let state = match refused {
        Some(Refused {
            refusal: Refusal::BadToken,
            ..
        }) => "refused",
        // PROTOCOL.md, Versioning: the side with the older range needs the update.
        Some(Refused {
            refusal: Refusal::UnsupportedProtocol,
            protocol: Some(asked),
            ..
        }) if *asked > u64::from(voice_protocol::PROTOCOL) => "voiceOutdated",
        Some(Refused {
            refusal: Refusal::UnsupportedProtocol,
            ..
        }) => "rukooOutdated",
        _ => "notRunning",
    };
    RukooInfo {
        state,
        client_version: refused.and_then(|r| r.client_version.clone()),
        protocol: None,
        behind: None,
    }
}

/// Rukoo clients whose `welcome` went out. A `meeting.starting` appended before a client's
/// welcome can land in its replay, which Rukoo treats as history and never acks, so the start
/// handshake counts a client only from its welcome on.
fn rukoo_count(clients: &Clients) -> usize {
    clients
        .connected
        .iter()
        .filter(|c| c.client == "rukoo" && c.welcomed)
        .count()
}

/// The running server, which a new token replaces.
pub struct Bridge {
    log: EventLog,
    control: Arc<Control>,
    path: PathBuf,
    version: String,
    handle: Mutex<Option<Handle>>,
    error: Mutex<Option<String>>,
    /// A Rukoo got in during this app run, also through a server a new token replaced.
    seen: AtomicBool,
}

impl Bridge {
    pub fn protection_clients(&self) -> Vec<u64> {
        self.handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or_else(Vec::new, Handle::protection_clients)
    }

    /// The pids of the connected Rukoo clients that sent one, for the screen process's
    /// `--exclude-pid`.
    pub fn rukoo_pids(&self) -> Vec<u32> {
        self.handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or_else(Vec::new, Handle::rukoo_pids)
    }
    pub fn start(log: EventLog, control: Arc<Control>, path: PathBuf, version: &str) -> Bridge {
        let bridge = Bridge {
            log,
            control,
            path,
            version: version.to_string(),
            handle: Mutex::new(None),
            error: Mutex::new(None),
            seen: AtomicBool::new(false),
        };
        let _ = bridge.listen();
        bridge
    }

    fn listen(&self) -> io::Result<()> {
        let control: Arc<dyn MeetingControl> = self.control.clone();
        let result = BridgeServer::start(self.log.clone(), control, &self.path, &self.version);
        let mut handle = self.handle.lock().unwrap_or_else(PoisonError::into_inner);
        let mut error = self.error.lock().unwrap_or_else(PoisonError::into_inner);
        match result {
            Ok(started) => {
                *handle = Some(started);
                *error = None;
                Ok(())
            }
            Err(e) => {
                *error = Some(e.to_string());
                Err(e)
            }
        }
    }

    pub fn port(&self) -> Option<u16> {
        self.handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Handle::port)
    }

    pub fn discovery_path(&self) -> &Path {
        &self.path
    }

    /// The clients the server has now; empty while it doesn't run.
    pub fn clients(&self) -> Clients {
        self.handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Handle::clients)
            .unwrap_or_default()
    }

    pub fn rukoo(&self) -> RukooInfo {
        let clients = self.clients();
        self.note_seen(&clients);
        rukoo_info(&clients, self.log.head())
    }

    /// How many Rukoo clients are connected now and have their `welcome`.
    pub fn rukoo_clients(&self) -> usize {
        let clients = self.clients();
        self.note_seen(&clients);
        rukoo_count(&clients)
    }

    /// Whether a Rukoo completed `hello` earlier in this app run: one that isn't connected now
    /// is probably reconnecting.
    pub fn rukoo_seen(&self) -> bool {
        let clients = self.clients();
        self.note_seen(&clients);
        self.seen.load(Ordering::SeqCst)
    }

    fn note_seen(&self, clients: &Clients) {
        // A client with the token got in. Only Rukoo connects in normal use; another client
        // costs a meeting Voice starts at most the reconnect wait.
        if rukoo_count(clients) > 0 || clients.last_admitted_at.is_some() {
            self.seen.store(true, Ordering::SeqCst);
        }
    }

    /// A new token: the server restarts, which writes a new discovery file and closes every
    /// client; Rukoo reconnects with the new file (PROTOCOL.md, Connecting).
    pub fn new_token(&self) -> io::Result<()> {
        // The new server forgets who connected; this app run doesn't.
        self.note_seen(&self.clients());
        self.stop();
        self.listen()
    }

    pub fn stop(&self) {
        let handle = self
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            handle.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_bridge::Client;

    fn client(name: &str, at: u64, sent: u64) -> Client {
        Client {
            client: name.into(),
            client_version: Some("2.2.0".into()),
            pid: Some(4821),
            protocol: 1,
            after: 0,
            sent,
            welcomed: true,
            connected_at: at,
        }
    }

    #[test]
    fn the_handshake_counts_rukoo_from_its_welcome_on() {
        let mut admitted = client("rukoo", 3, 0);
        admitted.welcomed = false;
        let clients = Clients {
            connected: vec![admitted, client("probe", 4, 0)],
            last_admitted_at: Some(4),
            ..Clients::default()
        };
        assert_eq!(rukoo_count(&clients), 0);
        let clients = Clients {
            connected: vec![client("rukoo", 3, 0), client("probe", 4, 0)],
            last_admitted_at: Some(4),
            ..Clients::default()
        };
        assert_eq!(rukoo_count(&clients), 1);
    }

    fn refused(refusal: Refusal, protocol: Option<u64>, at: u64) -> Option<Refused> {
        Some(Refused {
            refusal,
            protocol,
            client: Some("rukoo".into()),
            client_version: Some("2.3.0".into()),
            at,
        })
    }

    #[test]
    fn settings_say_who_is_connected_or_why_nobody_is() {
        let none = rukoo_info(&Clients::default(), 10);
        assert_eq!((none.state, none.behind), ("notRunning", None));

        let mut clients = Clients {
            connected: vec![client("probe", 5, 0), client("rukoo", 3, 7)],
            ..Clients::default()
        };
        let connected = rukoo_info(&clients, 10);
        assert_eq!(
            connected,
            RukooInfo {
                state: "connected",
                client_version: Some("2.2.0".into()),
                protocol: Some(1),
                behind: Some(3),
            }
        );
        assert!(connected.connected());

        // Another program on the bridge isn't Rukoo.
        clients.connected.retain(|c| c.client != "rukoo");
        assert_eq!(rukoo_info(&clients, 10).state, "notRunning");

        clients.refused = refused(Refusal::BadToken, Some(1), 20);
        assert_eq!(rukoo_info(&clients, 10).state, "refused");
        clients.refused = refused(
            Refusal::UnsupportedProtocol,
            Some(u64::from(voice_protocol::PROTOCOL) + 1),
            20,
        );
        let newer = rukoo_info(&clients, 10);
        assert_eq!(
            (newer.state, newer.client_version.as_deref()),
            ("voiceOutdated", Some("2.3.0"))
        );
        clients.refused = refused(Refusal::UnsupportedProtocol, Some(1), 20);
        assert_eq!(rukoo_info(&clients, 10).state, "rukooOutdated");
        // Once a client got in after the refusal, the refusal is history.
        clients.last_admitted_at = Some(25);
        assert_eq!(rukoo_info(&clients, 10).state, "notRunning");
    }
}
