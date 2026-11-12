use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Map, Value};

use crate::{close, MeetingId};

/// A JSON object, as `data`, `args`, `result` and `context` carry.
pub type Object = Map<String, Value>;

/// The event names of PROTOCOL.md.
pub mod events {
    pub const STATUS: &str = "status";
    pub const MEETING_STARTED: &str = "meeting.started";
    pub const MEETING_STARTING: &str = "meeting.starting";
    pub const MEETING_STATE: &str = "meeting.state";
    pub const SPEAKER: &str = "speaker";
    pub const PASSAGE: &str = "passage";
    pub const PASSAGE_REMOVED: &str = "passage.removed";
    pub const MARK: &str = "mark";
    pub const FRAME: &str = "frame";
    pub const MEETING_REMOVED: &str = "meeting.removed";

    /// True for the known events that belong to a meeting and so need `meeting`.
    pub fn is_meeting_event(name: &str) -> bool {
        matches!(
            name,
            MEETING_STARTED
                | MEETING_STARTING
                | MEETING_STATE
                | SPEAKER
                | PASSAGE
                | PASSAGE_REMOVED
                | MARK
                | FRAME
                | MEETING_REMOVED
        )
    }
}

/// The command names of PROTOCOL.md.
pub mod commands {
    pub const MEETING_START: &str = "meeting.start";
    pub const MEETING_STOP: &str = "meeting.stop";
    pub const MEETING_DELETE: &str = "meeting.delete";
    pub const MEETING_PAUSE: &str = "meeting.pause";
    pub const MEETING_RESUME: &str = "meeting.resume";
    pub const MEETING_MARK: &str = "meeting.mark";
    pub const MEETING_LIST: &str = "meeting.list";
    pub const MEETING_GET: &str = "meeting.get";
    pub const FRAME_GET: &str = "frame.get";
    pub const STATUS_GET: &str = "status.get";
    pub const CLIP_GET: &str = "clip.get";
    pub const MEETING_PROTECTED: &str = "meeting.protected";
    pub const SCREEN_GET: &str = "screen.get";
    pub const SCREEN_SELECT: &str = "screen.select";
    pub const SCREEN_SET: &str = "screen.set";
    pub const MEDIA_OPEN: &str = "media.open";
    pub const MEDIA_READ: &str = "media.read";
}

/// Generates `as_str`, `parse`, `Display` and string serde for an enum of fixed names.
macro_rules! named {
    ($ty:ident { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl $ty {
            pub fn as_str(self) -> &'static str {
                match self {
                    $($ty::$variant => $name,)+
                }
            }

            pub fn parse(name: &str) -> Option<$ty> {
                match name {
                    $($name => Some($ty::$variant),)+
                    _ => None,
                }
            }

            /// Every value, in the order PROTOCOL.md lists them.
            pub const ALL: &'static [$ty] = &[$($ty::$variant),+];
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<$ty, D::Error> {
                let name = String::deserialize(deserializer)?;
                $ty::parse(&name).ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        concat!("unknown ", stringify!($ty), " `{}`"),
                        name
                    ))
                })
            }
        }
    };
}

/// An audio track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Track {
    /// The user's microphone.
    Mic,
    /// The computer's audio output, which holds the other people in a call.
    Call,
}

named!(Track { Mic => "mic", Call => "call" });

impl AsRef<str> for Track {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// What a mark is about: a track, the screen, or (as `None` in [`Mark::track`]) the meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkTrack {
    Mic,
    Call,
    Screen,
}

named!(MarkTrack { Mic => "mic", Call => "call", Screen => "screen" });

impl From<Track> for MarkTrack {
    fn from(track: Track) -> MarkTrack {
        match track {
            Track::Mic => MarkTrack::Mic,
            Track::Call => MarkTrack::Call,
        }
    }
}

/// How a meeting records the screen: not at all, as frames with their text, or as frames and
/// video.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScreenMode {
    #[default]
    Off,
    Frames,
    Video,
}

named!(ScreenMode { Off => "off", Frames => "frames", Video => "video" });

/// Which app started a meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartedBy {
    Voice,
    Rukoo,
}

named!(StartedBy { Voice => "voice", Rukoo => "rukoo" });

/// The state of a meeting, as `meeting.state` and `meeting.list` report it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeetingState {
    Recording,
    Paused,
    Processing,
    Done,
    Failed,
}

named!(MeetingState {
    Recording => "recording",
    Paused => "paused",
    Processing => "processing",
    Done => "done",
    Failed => "failed",
});

impl MeetingState {
    /// `endedAt` comes with `processing` and every state after it.
    pub fn has_ended(self) -> bool {
        matches!(
            self,
            MeetingState::Processing | MeetingState::Done | MeetingState::Failed
        )
    }

    /// `reason` comes with `processing` and `failed`.
    pub fn needs_reason(self) -> bool {
        matches!(self, MeetingState::Processing | MeetingState::Failed)
    }
}

/// Which events follow `welcome`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Replay {
    /// The events after Rukoo's `after`.
    Partial,
    /// Every stored event: Rukoo is new, knows another instance, or is ahead of the head.
    Full,
    /// Every stored event: events Rukoo missed were already removed.
    Gap,
}

named!(Replay { Partial => "partial", Full => "full", Gap => "gap" });

/// The error codes of command replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    BadRequest,
    UnknownCommand,
    NotFound,
    NotRecording,
    Busy,
    NotAvailable,
    Failed,
}

named!(ErrorCode {
    BadRequest => "bad_request",
    UnknownCommand => "unknown_command",
    NotFound => "not_found",
    NotRecording => "not_recording",
    Busy => "busy",
    NotAvailable => "not_available",
    Failed => "failed",
});

/// Why Voice refuses a connection before or at `hello`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    BadToken,
    UnsupportedProtocol,
    HelloTimeout,
    TooManyClients,
}

named!(Refusal {
    BadToken => "bad_token",
    UnsupportedProtocol => "unsupported_protocol",
    HelloTimeout => "hello_timeout",
    TooManyClients => "too_many_clients",
});

impl Refusal {
    /// The WebSocket close code that goes with the `error` message.
    pub fn close_code(self) -> u16 {
        match self {
            Refusal::BadToken => close::BAD_TOKEN,
            Refusal::UnsupportedProtocol => close::UNSUPPORTED_PROTOCOL,
            Refusal::HelloTimeout => close::HELLO_TIMEOUT,
            Refusal::TooManyClients => close::TOO_MANY_CLIENTS,
        }
    }

    /// The `error` message Voice sends before it closes the connection.
    pub fn message(self) -> ErrorMessage {
        let text = match self {
            Refusal::BadToken => "The token doesn't match.",
            Refusal::UnsupportedProtocol => "Voice doesn't speak the requested protocol version.",
            Refusal::HelloTimeout => "The first message wasn't a hello within 5 seconds.",
            Refusal::TooManyClients => "Four clients are already connected.",
        };
        ErrorMessage {
            code: self.as_str().to_owned(),
            message: text.to_owned(),
        }
    }
}

/// The kind of a mark. Kinds a newer Voice adds arrive as `Other`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MarkKind {
    /// Someone marked this moment.
    User,
    /// The user dictated into another app.
    Dictation,
    /// The track has no audio.
    Gap,
    /// The track moved to another device.
    Device,
    /// Voice stopped recording the screen, to keep recording audio.
    Dropped,
    Other(String),
}

impl MarkKind {
    pub fn as_str(&self) -> &str {
        match self {
            MarkKind::User => "user",
            MarkKind::Dictation => "dictation",
            MarkKind::Gap => "gap",
            MarkKind::Device => "device",
            MarkKind::Dropped => "dropped",
            MarkKind::Other(kind) => kind,
        }
    }

    pub fn parse(kind: &str) -> MarkKind {
        match kind {
            "user" => MarkKind::User,
            "dictation" => MarkKind::Dictation,
            "gap" => MarkKind::Gap,
            "device" => MarkKind::Device,
            "dropped" => MarkKind::Dropped,
            other => MarkKind::Other(other.to_owned()),
        }
    }
}

impl fmt::Display for MarkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for MarkKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MarkKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<MarkKind, D::Error> {
        Ok(MarkKind::parse(&String::deserialize(deserializer)?))
    }
}

/// A message on the bridge, in either direction.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Message {
    Hello(Hello),
    Welcome(Welcome),
    Error(ErrorMessage),
    Event(Event),
    Command(Command),
    Reply(Reply),
}

impl Message {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("bridge messages serialize to JSON")
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("bridge messages serialize to JSON")
    }
}

/// Rukoo's first message.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    pub token: String,
    pub client: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    /// Rukoo's process id, when it sent one: the process that owns its windows. Voice uses it
    /// to leave Rukoo out of its own screen recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u64>,
    /// The instance Rukoo last saw, or `None` on its first connection.
    pub instance: Option<String>,
    /// The last sequence number Rukoo saw, or 0.
    pub after: u64,
}

impl fmt::Debug for Hello {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Hellos end up in logs; the token must not.
        f.debug_struct("Hello")
            .field("protocol", &self.protocol)
            .field("token", &"<redacted>")
            .field("client", &self.client)
            .field("client_version", &self.client_version)
            .field("pid", &self.pid)
            .field("instance", &self.instance)
            .field("after", &self.after)
            .finish()
    }
}

/// Voice's answer to a good `hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    pub protocol: u32,
    pub voice_version: String,
    pub instance: String,
    pub head: u64,
    pub oldest: u64,
    pub replay: Replay,
    pub status: Status,
}

/// What Voice is doing right now: the `status` event and the status in `welcome`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// The meeting being recorded or paused.
    pub recording: Option<MeetingId>,
    /// True while the user holds a dictation key.
    pub dictating: bool,
    /// The `screen` a meeting gets when `meeting.start` leaves it out. An older Voice leaves it
    /// out.
    #[serde(
        rename = "screenDefault",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub screen_default: Option<ScreenMode>,
}

/// The message Voice sends before it closes a connection it refuses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMessage {
    pub code: String,
    pub message: String,
}

/// A stored event, with its sequence number.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: u64,
    /// There for meeting events, missing for the others.
    pub meeting: Option<MeetingId>,
    pub created_at: u64,
    pub body: EventBody,
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut event = serializer.serialize_struct("Event", 5)?;
        event.serialize_field("seq", &self.seq)?;
        event.serialize_field("event", self.body.name())?;
        match &self.meeting {
            Some(meeting) => event.serialize_field("meeting", meeting)?,
            None => event.skip_field("meeting")?,
        }
        event.serialize_field("createdAt", &self.created_at)?;
        event.serialize_field("data", &self.body.data())?;
        event.end()
    }
}

/// The `event` name and `data` of an event.
#[derive(Clone, Debug, PartialEq)]
pub enum EventBody {
    Status(Status),
    MeetingStarted(MeetingStarted),
    MeetingStarting(MeetingStarting),
    MeetingState(MeetingStateChange),
    Speaker(Speaker),
    Passage(Passage),
    PassageRemoved(PassageRemoved),
    Mark(Mark),
    Frame(Frame),
    /// Voice deleted the meeting and its events.
    MeetingRemoved {
        reason: Option<String>,
    },
    /// An event type this version doesn't know. Rukoo ignores it.
    Unknown {
        event: String,
        data: Object,
    },
}

impl EventBody {
    /// The `event` field.
    pub fn name(&self) -> &str {
        match self {
            EventBody::Status(_) => events::STATUS,
            EventBody::MeetingStarted(_) => events::MEETING_STARTED,
            EventBody::MeetingStarting(_) => events::MEETING_STARTING,
            EventBody::MeetingState(_) => events::MEETING_STATE,
            EventBody::Speaker(_) => events::SPEAKER,
            EventBody::Passage(_) => events::PASSAGE,
            EventBody::PassageRemoved(_) => events::PASSAGE_REMOVED,
            EventBody::Mark(_) => events::MARK,
            EventBody::Frame(_) => events::FRAME,
            EventBody::MeetingRemoved { .. } => events::MEETING_REMOVED,
            EventBody::Unknown { event, .. } => event,
        }
    }

    /// The `data` field.
    pub fn data(&self) -> Value {
        let data = match self {
            EventBody::Status(status) => serde_json::to_value(status),
            EventBody::MeetingStarted(started) => serde_json::to_value(started),
            EventBody::MeetingStarting(starting) => serde_json::to_value(starting),
            EventBody::MeetingState(state) => serde_json::to_value(state),
            EventBody::Speaker(speaker) => serde_json::to_value(speaker),
            EventBody::Passage(passage) => serde_json::to_value(passage),
            EventBody::PassageRemoved(removed) => serde_json::to_value(removed),
            EventBody::Mark(mark) => serde_json::to_value(mark),
            EventBody::Frame(frame) => serde_json::to_value(frame),
            EventBody::MeetingRemoved { reason } => Ok(match reason {
                Some(reason) => json!({"reason": reason}),
                None => json!({}),
            }),
            EventBody::Unknown { data, .. } => Ok(Value::Object(data.clone())),
        };
        data.expect("event data serializes to JSON")
    }

    /// The `event` name and the `data`, as `EventLog::append` takes them.
    pub fn into_parts(self) -> (String, Value) {
        let data = self.data();
        let name = match self {
            EventBody::Unknown { event, .. } => event,
            known => known.name().to_owned(),
        };
        (name, data)
    }
}

/// `meeting.started`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingStarted {
    pub title: String,
    pub started_at: u64,
    pub tracks: Vec<Track>,
    pub screen: ScreenMode,
    pub started_by: StartedBy,
    /// The object `meeting.start` passed, unchanged. Voice stores it and doesn't read it.
    pub context: Option<Object>,
}

/// `meeting.starting`: Voice is about to record the screen in a meeting it started itself, and
/// waits up to [`STARTING_WAIT`](crate::STARTING_WAIT) for every connected Rukoo to answer with
/// `meeting.protected`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingStarting {
    pub screen: ScreenMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
}

/// `meeting.state`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingStateChange {
    pub state: MeetingState,
    /// The meeting time of the change.
    pub at: u64,
    /// Comes with `processing` and every state after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Comes with `processing` and `failed`: `user`, `rukoo`, `interrupted`, `limit`, or an
    /// error text for `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `speaker`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Speaker {
    pub id: String,
    pub track: Track,
    pub label: String,
}

/// `passage`: a stretch of transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passage {
    /// [`passage_id`](crate::passage_id) of the track and the start.
    pub id: String,
    pub track: Track,
    pub speaker: Option<String>,
    pub start: u64,
    pub end: u64,
    pub text: String,
    pub revision: u64,
    pub dictation: bool,
    pub language: Option<String>,
}

/// `passage.removed`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassageRemoved {
    pub id: String,
}

/// `mark`: something that happened at a moment in the meeting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub id: String,
    pub kind: MarkKind,
    pub at: u64,
    pub until: Option<u64>,
    /// `None` when the mark is about the whole meeting.
    pub track: Option<MarkTrack>,
    pub detail: Option<String>,
    pub label: Option<String>,
}

/// `frame`: a moment the screen changed noticeably, in a meeting that records the screen. `text`
/// is what Voice read on it, at most [`MAX_FRAME_TEXT_CHARS`](crate::MAX_FRAME_TEXT_CHARS).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub id: String,
    pub at: u64,
    pub width: u32,
    pub height: u32,
    pub text: String,
}

/// A command from Rukoo.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// Rukoo's own id, returned unchanged in the reply.
    pub id: String,
    pub body: CommandBody,
}

impl Serialize for Command {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut command = serializer.serialize_struct("Command", 3)?;
        command.serialize_field("id", &self.id)?;
        command.serialize_field("command", self.body.name())?;
        command.serialize_field("args", &self.body.args())?;
        command.end()
    }
}

/// The `command` name and `args` of a command.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandBody {
    MeetingStart(StartArgs),
    MeetingStop {
        meeting: MeetingId,
    },
    MeetingDelete {
        meeting: MeetingId,
    },
    MeetingPause {
        meeting: MeetingId,
    },
    MeetingResume {
        meeting: MeetingId,
    },
    MeetingMark {
        meeting: MeetingId,
        label: Option<String>,
    },
    MeetingList {
        /// With the default of 50 filled in.
        limit: u64,
    },
    MeetingGet {
        meeting: MeetingId,
    },
    FrameGet {
        meeting: MeetingId,
        at: u64,
    },
    StatusGet,
    /// Cuts the meeting's video between `start` and `end` (meeting ms, `end >= start`).
    ClipGet {
        meeting: MeetingId,
        start: u64,
        end: u64,
    },
    /// Rukoo applied its window protection after `meeting.starting`.
    MeetingProtected {
        meeting: MeetingId,
        request: Option<String>,
    },
    ScreenGet {
        meeting: MeetingId,
    },
    /// Moves the screen recording to the monitor with that id from `screen.get`.
    ScreenSelect {
        meeting: MeetingId,
        monitor: String,
    },
    ScreenSet {
        meeting: MeetingId,
        screen: ScreenMode,
        monitor: Option<String>,
    },
    MediaOpen(MediaOpenArgs),
    MediaRead {
        /// An id `media.open` gave.
        media: String,
        offset: u64,
        /// 1 to [`crate::MAX_MEDIA_READ`].
        length: u64,
    },
    /// A command this version doesn't know. Voice answers `unknown_command`.
    Unknown {
        command: String,
        args: Object,
    },
}

impl CommandBody {
    /// The `command` field.
    pub fn name(&self) -> &str {
        match self {
            CommandBody::MeetingStart(_) => commands::MEETING_START,
            CommandBody::MeetingStop { .. } => commands::MEETING_STOP,
            CommandBody::MeetingDelete { .. } => commands::MEETING_DELETE,
            CommandBody::MeetingPause { .. } => commands::MEETING_PAUSE,
            CommandBody::MeetingResume { .. } => commands::MEETING_RESUME,
            CommandBody::MeetingMark { .. } => commands::MEETING_MARK,
            CommandBody::MeetingList { .. } => commands::MEETING_LIST,
            CommandBody::MeetingGet { .. } => commands::MEETING_GET,
            CommandBody::FrameGet { .. } => commands::FRAME_GET,
            CommandBody::StatusGet => commands::STATUS_GET,
            CommandBody::ClipGet { .. } => commands::CLIP_GET,
            CommandBody::MeetingProtected { .. } => commands::MEETING_PROTECTED,
            CommandBody::ScreenGet { .. } => commands::SCREEN_GET,
            CommandBody::ScreenSelect { .. } => commands::SCREEN_SELECT,
            CommandBody::ScreenSet { .. } => commands::SCREEN_SET,
            CommandBody::MediaOpen(_) => commands::MEDIA_OPEN,
            CommandBody::MediaRead { .. } => commands::MEDIA_READ,
            CommandBody::Unknown { command, .. } => command,
        }
    }

    /// The `args` field.
    pub fn args(&self) -> Value {
        let mut args = Object::new();
        match self {
            CommandBody::MeetingStart(start) => {
                return serde_json::to_value(start).expect("start args serialize to JSON")
            }
            CommandBody::MeetingStop { meeting }
            | CommandBody::MeetingDelete { meeting }
            | CommandBody::MeetingPause { meeting }
            | CommandBody::MeetingResume { meeting }
            | CommandBody::MeetingGet { meeting }
            | CommandBody::ScreenGet { meeting } => {
                args.insert("meeting".into(), meeting.as_str().into());
            }
            CommandBody::MeetingProtected { meeting, request } => {
                args.insert("meeting".into(), meeting.as_str().into());
                if let Some(request) = request {
                    args.insert("request".into(), request.as_str().into());
                }
            }
            CommandBody::ScreenSet {
                meeting,
                screen,
                monitor,
            } => {
                args.insert("meeting".into(), meeting.as_str().into());
                args.insert("screen".into(), screen.as_str().into());
                if let Some(monitor) = monitor {
                    args.insert("monitor".into(), monitor.as_str().into());
                }
            }
            CommandBody::MeetingMark { meeting, label } => {
                args.insert("meeting".into(), meeting.as_str().into());
                if let Some(label) = label {
                    args.insert("label".into(), label.as_str().into());
                }
            }
            CommandBody::MeetingList { limit } => {
                args.insert("limit".into(), (*limit).into());
            }
            CommandBody::FrameGet { meeting, at } => {
                args.insert("meeting".into(), meeting.as_str().into());
                args.insert("at".into(), (*at).into());
            }
            CommandBody::StatusGet => {}
            CommandBody::ClipGet {
                meeting,
                start,
                end,
            } => {
                args.insert("meeting".into(), meeting.as_str().into());
                args.insert("start".into(), (*start).into());
                args.insert("end".into(), (*end).into());
            }
            CommandBody::ScreenSelect { meeting, monitor } => {
                args.insert("meeting".into(), meeting.as_str().into());
                args.insert("monitor".into(), monitor.as_str().into());
            }
            CommandBody::MediaOpen(open) => {
                args.insert("meeting".into(), open.meeting.as_str().into());
                args.insert("kind".into(), open.kind.as_str().into());
                args.insert("start".into(), open.start.into());
                if let Some(end) = open.end {
                    args.insert("end".into(), end.into());
                }
            }
            CommandBody::MediaRead {
                media,
                offset,
                length,
            } => {
                args.insert("media".into(), media.as_str().into());
                args.insert("offset".into(), (*offset).into());
                args.insert("length".into(), (*length).into());
            }
            CommandBody::Unknown { args: unknown, .. } => return Value::Object(unknown.clone()),
        }
        Value::Object(args)
    }
}

/// The arguments of `meeting.start`. Voice uses the Settings default for each one left out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StartArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<Track>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<ScreenMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Object>,
}

/// What `media.open` prepares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// A WAV of all the meeting's tracks mixed, 16 kHz mono 16-bit, from `start` to `end`.
    Audio,
    /// The JPEG of the frame at or before `start`.
    Frame,
    /// An MP4 of one stretch of continuous video from `start` to `end`.
    Video,
}

named!(MediaKind { Audio => "audio", Frame => "frame", Video => "video" });

/// The arguments of `media.open`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MediaOpenArgs {
    pub meeting: MeetingId,
    pub kind: MediaKind,
    /// Meeting ms.
    pub start: u64,
    /// Meeting ms; Voice requires it for audio and video.
    pub end: Option<u64>,
}

/// The result of `media.read`: `{ "offset", "data" }`, the bytes in standard base64 with padding
/// and without line breaks, as Rukoo reads them.
pub fn media_read_result(offset: u64, bytes: &[u8]) -> Value {
    let mut result = Object::new();
    result.insert("offset".into(), offset.into());
    result.insert("data".into(), STANDARD.encode(bytes).into());
    Value::Object(result)
}

/// Voice's answer to one command.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub id: String,
    pub outcome: Result<Object, ReplyError>,
}

impl Reply {
    pub fn ok(id: impl Into<String>, result: Object) -> Reply {
        Reply {
            id: id.into(),
            outcome: Ok(result),
        }
    }

    pub fn error(id: impl Into<String>, error: ReplyError) -> Reply {
        Reply {
            id: id.into(),
            outcome: Err(error),
        }
    }
}

impl Serialize for Reply {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut reply = serializer.serialize_struct("Reply", 3)?;
        reply.serialize_field("id", &self.id)?;
        match &self.outcome {
            Ok(result) => {
                reply.serialize_field("ok", &true)?;
                reply.serialize_field("result", result)?;
            }
            Err(error) => {
                reply.serialize_field("ok", &false)?;
                reply.serialize_field("error", error)?;
            }
        }
        reply.end()
    }
}

/// The `error` of a failed reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyError {
    /// An [`ErrorCode`], kept as text so codes from a newer Voice survive.
    pub code: String,
    /// English, for logs. Rukoo shows its own text for a code.
    pub message: String,
    /// For `busy`: the meeting that is recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting: Option<MeetingId>,
}

impl ReplyError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> ReplyError {
        ReplyError {
            code: code.as_str().to_owned(),
            message: message.into(),
            meeting: None,
        }
    }
}

/// One meeting in the result of `meeting.list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: MeetingId,
    pub title: String,
    pub started_at: u64,
    /// `None` while recording.
    pub ended_at: Option<u64>,
    pub state: MeetingState,
    /// In milliseconds.
    pub duration: u64,
}

/// The discovery file Voice writes when it starts.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub protocol: u32,
    pub min_protocol: u32,
    /// The WebSocket port on `127.0.0.1`.
    pub port: u16,
    /// 32 random bytes, fresh every time Voice starts.
    pub token: String,
    pub pid: u32,
    /// 16 random bytes, made once per Voice data folder.
    pub instance: String,
    pub voice_version: String,
    pub started_at: u64,
}

impl fmt::Debug for Discovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Discovery")
            .field("protocol", &self.protocol)
            .field("min_protocol", &self.min_protocol)
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .field("pid", &self.pid)
            .field("instance", &self.instance)
            .field("voice_version", &self.voice_version)
            .field("started_at", &self.started_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn named_values_round_trip() {
        fn check<T>(all: &[T])
        where
            T: Serialize + for<'de> Deserialize<'de> + PartialEq + fmt::Debug + Copy,
        {
            for value in all {
                let text = serde_json::to_value(value).unwrap();
                assert_eq!(serde_json::from_value::<T>(text).unwrap(), *value);
            }
        }
        check(Track::ALL);
        check(MarkTrack::ALL);
        check(ScreenMode::ALL);
        check(StartedBy::ALL);
        check(MeetingState::ALL);
        check(Replay::ALL);
        check(ErrorCode::ALL);
        check(Refusal::ALL);
        check(MediaKind::ALL);
        assert!(serde_json::from_value::<Track>(json!("room")).is_err());
    }

    #[test]
    fn unknown_mark_kinds_survive() {
        let kind: MarkKind = serde_json::from_value(json!("laughter")).unwrap();
        assert_eq!(kind, MarkKind::Other("laughter".into()));
        assert_eq!(serde_json::to_value(&kind).unwrap(), json!("laughter"));
        assert_eq!(MarkKind::parse("gap"), MarkKind::Gap);
    }

    #[test]
    fn refusals_carry_their_close_codes() {
        assert_eq!(Refusal::BadToken.close_code(), 4001);
        assert_eq!(Refusal::UnsupportedProtocol.close_code(), 4002);
        assert_eq!(Refusal::HelloTimeout.close_code(), 4003);
        assert_eq!(Refusal::TooManyClients.close_code(), 4004);
        assert_eq!(
            serde_json::to_value(Message::Error(Refusal::BadToken.message())).unwrap(),
            json!({ "type": "error", "code": "bad_token", "message": "The token doesn't match." })
        );
    }

    #[test]
    fn debug_output_hides_tokens() {
        let token = "q7hR0lJ3x2b8Yc5vN1mK4sT9wE6uA0dF3gH2jL5pZ8o";
        let hello = Hello {
            protocol: 1,
            token: token.into(),
            client: "rukoo".into(),
            client_version: None,
            pid: Some(4821),
            instance: None,
            after: 0,
        };
        let discovery = Discovery {
            protocol: 1,
            min_protocol: 1,
            port: 1,
            token: token.into(),
            pid: 1,
            instance: "Vb3kQ9m2X7pL1sR4tY8wZQ".into(),
            voice_version: "0.1.0".into(),
            started_at: 0,
        };
        assert!(!format!("{hello:?}").contains(token));
        assert!(!format!("{discovery:?}").contains(token));
    }

    #[test]
    fn events_leave_out_a_missing_meeting_and_keep_nulls_in_data() {
        let event = Event {
            seq: 3,
            meeting: None,
            created_at: 10,
            body: EventBody::Status(Status::default()),
        };
        assert_eq!(
            Message::Event(event).to_value(),
            json!({ "type": "event", "seq": 3, "event": "status", "createdAt": 10, "data": { "recording": null, "dictating": false } })
        );
    }

    #[test]
    fn into_parts_gives_the_name_and_data() {
        let (name, data) = EventBody::PassageRemoved(PassageRemoved {
            id: "mic-000000000".into(),
        })
        .into_parts();
        assert_eq!(name, "passage.removed");
        assert_eq!(data, json!({ "id": "mic-000000000" }));
        let (name, data) = EventBody::MeetingRemoved { reason: None }.into_parts();
        assert_eq!((name.as_str(), data), ("meeting.removed", json!({})));
    }

    #[test]
    fn media_pieces_are_padded_standard_base64() {
        assert_eq!(
            media_read_result(1_048_576, b"RIFF$\0\0\0WAVE"),
            json!({ "offset": 1_048_576, "data": "UklGRiQAAABXQVZF" })
        );
        // Bytes that differ between base64 and base64url, and padding.
        assert_eq!(
            media_read_result(0, &[0xfb, 0xff, 0xbf, 0xfe])["data"],
            "+/+//g=="
        );
        assert_eq!(
            media_read_result(7, &[]),
            json!({ "offset": 7, "data": "" })
        );
    }
}
