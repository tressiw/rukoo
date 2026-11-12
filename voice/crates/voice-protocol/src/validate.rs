//! "Checking messages" in PROTOCOL.md.
//!
//! Each check fails with the name of the field it found wrong, which is what the fixtures call
//! the `reason`. Fields are checked in the order PROTOCOL.md lists them, so a message with one
//! defect always fails with that defect's field.

use std::fmt;

use serde_json::Value;

use crate::message::{
    commands, events, Command, CommandBody, Discovery, ErrorMessage, Event, EventBody, Frame,
    Hello, Mark, MarkKind, MarkTrack, MediaKind, MediaOpenArgs, MeetingStarted, MeetingStarting,
    MeetingState, MeetingStateChange, Message, Object, Passage, PassageRemoved, Replay, Reply,
    ReplyError, ScreenMode, Speaker, StartArgs, StartedBy, Status, Track, Welcome,
};
use crate::{
    is_instance, is_media_id, is_token, passage_id, MeetingId, Side, DEFAULT_LIST_LIMIT,
    MAX_CONTEXT_BYTES, MAX_ID_CHARS, MAX_LABEL_CHARS, MAX_MEDIA_READ, MAX_MEDIA_SPAN_MS,
    MAX_TITLE_CHARS,
};

/// A message that breaks a rule. `reason` names the field, as the fixtures do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    pub reason: String,
}

impl Invalid {
    pub fn new(reason: impl Into<String>) -> Invalid {
        Invalid {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is missing or invalid", self.reason)
    }
}

impl std::error::Error for Invalid {}

type Checked<T> = Result<T, Invalid>;

fn invalid<T>(reason: &str) -> Checked<T> {
    Err(Invalid::new(reason))
}

fn optional_short(fields: &Fields, key: &str) -> Checked<Option<String>> {
    let value = fields.optional_string(key)?;
    if value
        .as_ref()
        .is_some_and(|v| v.is_empty() || v.chars().count() > MAX_ID_CHARS)
    {
        return invalid(key);
    }
    Ok(value)
}

/// Checks a message that `from` sent and builds the typed message.
///
/// Rukoo sends `hello` and `command`; Voice sends `welcome`, `error`, `event` and `reply`.
/// Anything else fails with the reason `type`. An unknown event type or command name is not an
/// error: it comes back as [`EventBody::Unknown`] or [`CommandBody::Unknown`].
pub fn validate_incoming(from: Side, value: &Value) -> Result<Message, Invalid> {
    let Some(object) = value.as_object() else {
        return invalid("type");
    };
    let fields = Fields(object);
    let kind = fields.string("type")?;
    match (from, kind) {
        (Side::Rukoo, "hello") => hello(&fields).map(Message::Hello),
        (Side::Rukoo, "command") => command(&fields).map(Message::Command),
        (Side::Voice, "welcome") => welcome(&fields).map(Message::Welcome),
        (Side::Voice, "error") => error(&fields).map(Message::Error),
        (Side::Voice, "event") => event(&fields).map(Message::Event),
        (Side::Voice, "reply") => reply(&fields).map(Message::Reply),
        _ => invalid("type"),
    }
}

/// Checks a discovery file.
pub fn validate_discovery(value: &Value) -> Result<Discovery, Invalid> {
    let Some(object) = value.as_object() else {
        return invalid("protocol");
    };
    let f = Fields(object);
    let protocol = f.version("protocol")?;
    let min_protocol = f.version("minProtocol")?;
    if min_protocol > protocol {
        return invalid("minProtocol");
    }
    let port = f.uint("port")?;
    let Ok(port) = u16::try_from(port) else {
        return invalid("port");
    };
    if port == 0 {
        return invalid("port");
    }
    let token = f.token("token")?;
    let pid = f.u32("pid")?;
    let instance = f.instance("instance")?;
    let voice_version = f.string("voiceVersion")?.to_owned();
    let started_at = f.uint("startedAt")?;
    Ok(Discovery {
        protocol,
        min_protocol,
        port,
        token,
        pid,
        instance,
        voice_version,
        started_at,
    })
}

fn hello(f: &Fields) -> Checked<Hello> {
    let protocol = f.version("protocol")?;
    let token = f.token("token")?;
    let client = f.short_text("client")?;
    let client_version = f.optional_string("clientVersion")?;
    let pid = match f.optional_uint("pid")? {
        Some(0) => return invalid("pid"),
        pid => pid,
    };
    let instance = match f.nullable_string("instance")? {
        Some(instance) if !is_instance(instance) => return invalid("instance"),
        instance => instance.map(str::to_owned),
    };
    let after = f.uint("after")?;
    Ok(Hello {
        protocol,
        token,
        client,
        client_version,
        pid,
        instance,
        after,
    })
}

fn welcome(f: &Fields) -> Checked<Welcome> {
    let protocol = f.version("protocol")?;
    let voice_version = f.string("voiceVersion")?.to_owned();
    let instance = f.instance("instance")?;
    let head = f.uint("head")?;
    let oldest = f.uint("oldest")?;
    let replay = f.named("replay", Replay::parse)?;
    let status = status(&Fields(f.object("status")?))?;
    Ok(Welcome {
        protocol,
        voice_version,
        instance,
        head,
        oldest,
        replay,
        status,
    })
}

fn error(f: &Fields) -> Checked<ErrorMessage> {
    Ok(ErrorMessage {
        code: f.id_text("code")?,
        message: f.string("message")?.to_owned(),
    })
}

fn status(f: &Fields) -> Checked<Status> {
    let recording = f.nullable_meeting("recording")?;
    let dictating = f.boolean("dictating")?;
    let screen_default = match f.optional_string("screenDefault")? {
        None => None,
        Some(screen) => match ScreenMode::parse(&screen) {
            Some(screen) => Some(screen),
            None => return invalid("screenDefault"),
        },
    };
    Ok(Status {
        recording,
        dictating,
        screen_default,
    })
}

fn event(f: &Fields) -> Checked<Event> {
    let seq = f.positive("seq")?;
    let name = f.string("event")?;
    if name.is_empty() {
        return invalid("event");
    }
    let created_at = f.uint("createdAt")?;
    let meeting = f.optional_meeting("meeting")?;
    let data = Fields(f.object("data")?);
    if events::is_meeting_event(name) && meeting.is_none() {
        return invalid("meeting");
    }
    let body = match name {
        events::STATUS => EventBody::Status(status(&data)?),
        events::MEETING_STARTED => EventBody::MeetingStarted(meeting_started(&data)?),
        events::MEETING_STARTING => EventBody::MeetingStarting(MeetingStarting {
            screen: data.named("screen", ScreenMode::parse)?,
            request: optional_short(&data, "request")?,
        }),
        events::MEETING_STATE => EventBody::MeetingState(meeting_state(&data)?),
        events::SPEAKER => EventBody::Speaker(Speaker {
            id: data.id_text("id")?,
            track: data.named("track", Track::parse)?,
            label: data.string("label")?.to_owned(),
        }),
        events::PASSAGE => EventBody::Passage(passage(&data)?),
        events::PASSAGE_REMOVED => EventBody::PassageRemoved(PassageRemoved {
            id: data.id_text("id")?,
        }),
        events::MARK => EventBody::Mark(mark(&data)?),
        events::FRAME => EventBody::Frame(Frame {
            id: data.id_text("id")?,
            at: data.uint("at")?,
            width: data.u32("width")?,
            height: data.u32("height")?,
            text: data.string("text")?.to_owned(),
        }),
        events::MEETING_REMOVED => EventBody::MeetingRemoved {
            reason: optional_short(&data, "reason")?,
        },
        unknown => EventBody::Unknown {
            event: unknown.to_owned(),
            data: data.0.clone(),
        },
    };
    Ok(Event {
        seq,
        meeting,
        created_at,
        body,
    })
}

fn meeting_started(f: &Fields) -> Checked<MeetingStarted> {
    Ok(MeetingStarted {
        title: f.string("title")?.to_owned(),
        started_at: f.uint("startedAt")?,
        tracks: f.tracks("tracks")?,
        screen: f.named("screen", ScreenMode::parse)?,
        started_by: f.named("startedBy", StartedBy::parse)?,
        context: f.nullable_object("context")?.cloned(),
    })
}

fn meeting_state(f: &Fields) -> Checked<MeetingStateChange> {
    let state = f.named("state", MeetingState::parse)?;
    let at = f.uint("at")?;
    let ended_at = f.optional_uint("endedAt")?;
    let reason = f.optional_string("reason")?;
    if state.has_ended() && ended_at.is_none() {
        return invalid("endedAt");
    }
    // An empty reason says nothing; Rukoo refuses it as it refuses a missing one.
    if state.needs_reason() && reason.as_deref().is_none_or(str::is_empty) {
        return invalid("reason");
    }
    Ok(MeetingStateChange {
        state,
        at,
        ended_at,
        reason,
    })
}

fn passage(f: &Fields) -> Checked<Passage> {
    let id = f.string("id")?;
    let track = f.named("track", Track::parse)?;
    // A speaker is an id, and ids aren't empty.
    let speaker = match f.nullable_string("speaker")? {
        Some("") => return invalid("speaker"),
        speaker => speaker.map(str::to_owned),
    };
    let start = f.uint("start")?;
    let end = f.uint("end")?;
    let text = f.string("text")?.to_owned();
    let revision = f.positive("revision")?;
    let dictation = f.boolean("dictation")?;
    let language = f.nullable_string("language")?.map(str::to_owned);
    if end < start {
        return invalid("end");
    }
    if id != passage_id(track, start) {
        return invalid("id");
    }
    Ok(Passage {
        id: id.to_owned(),
        track,
        speaker,
        start,
        end,
        text,
        revision,
        dictation,
        language,
    })
}

fn mark(f: &Fields) -> Checked<Mark> {
    let id = f.id_text("id")?;
    let kind = MarkKind::parse(&f.id_text("kind")?);
    let at = f.uint("at")?;
    let until = f.nullable_uint("until")?;
    let track = match f.nullable_string("track")? {
        None => None,
        Some(track) => match MarkTrack::parse(track) {
            Some(track) => Some(track),
            None => return invalid("track"),
        },
    };
    let detail = f.nullable_string("detail")?.map(str::to_owned);
    let label = f.nullable_string("label")?.map(str::to_owned);
    if until.is_some_and(|until| until < at) {
        return invalid("until");
    }
    Ok(Mark {
        id,
        kind,
        at,
        until,
        track,
        detail,
        label,
    })
}

fn command(f: &Fields) -> Checked<Command> {
    let id = f.short_text("id")?;
    let name = f.string("command")?;
    if name.is_empty() {
        return invalid("command");
    }
    let empty = Object::new();
    let args = match f.0.get("args") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(args)) => args,
        Some(_) => return invalid("args"),
    };
    let a = Fields(args);
    let body = match name {
        commands::MEETING_START => CommandBody::MeetingStart(start_args(&a)?),
        commands::MEETING_STOP => CommandBody::MeetingStop {
            meeting: a.meeting("meeting")?,
        },
        commands::MEETING_DELETE => CommandBody::MeetingDelete {
            meeting: a.meeting("meeting")?,
        },
        commands::MEETING_PAUSE => CommandBody::MeetingPause {
            meeting: a.meeting("meeting")?,
        },
        commands::MEETING_RESUME => CommandBody::MeetingResume {
            meeting: a.meeting("meeting")?,
        },
        commands::MEETING_MARK => CommandBody::MeetingMark {
            meeting: a.meeting("meeting")?,
            label: match a.optional_string("label")? {
                Some(label) if label.chars().count() > MAX_LABEL_CHARS => return invalid("label"),
                label => label,
            },
        },
        commands::MEETING_LIST => CommandBody::MeetingList {
            limit: match a.optional_uint("limit")? {
                Some(0) => return invalid("limit"),
                Some(limit) => limit,
                None => DEFAULT_LIST_LIMIT,
            },
        },
        commands::MEETING_GET => CommandBody::MeetingGet {
            meeting: a.meeting("meeting")?,
        },
        commands::FRAME_GET => CommandBody::FrameGet {
            meeting: a.meeting("meeting")?,
            at: a.uint("at")?,
        },
        commands::STATUS_GET => CommandBody::StatusGet,
        commands::CLIP_GET => {
            let meeting = a.meeting("meeting")?;
            let start = a.uint("start")?;
            let end = a.uint("end")?;
            if end < start {
                return invalid("end");
            }
            CommandBody::ClipGet {
                meeting,
                start,
                end,
            }
        }
        commands::MEETING_PROTECTED => CommandBody::MeetingProtected {
            meeting: a.meeting("meeting")?,
            request: optional_short(&a, "request")?,
        },
        commands::SCREEN_GET => CommandBody::ScreenGet {
            meeting: a.meeting("meeting")?,
        },
        commands::SCREEN_SELECT => CommandBody::ScreenSelect {
            meeting: a.meeting("meeting")?,
            monitor: a.short_text("monitor")?,
        },
        commands::SCREEN_SET => CommandBody::ScreenSet {
            meeting: a.meeting("meeting")?,
            screen: a.named("screen", ScreenMode::parse)?,
            monitor: optional_short(&a, "monitor")?,
        },
        commands::MEDIA_OPEN => {
            let meeting = a.meeting("meeting")?;
            let kind = a.named("kind", MediaKind::parse)?;
            let start = a.uint("start")?;
            let end = a.optional_uint("end")?;
            if end.is_some_and(|end| end < start || end - start > MAX_MEDIA_SPAN_MS) {
                return invalid("end");
            }
            CommandBody::MediaOpen(MediaOpenArgs {
                meeting,
                kind,
                start,
                end,
            })
        }
        commands::MEDIA_READ => {
            let media = a.string("media")?;
            if !is_media_id(media) {
                return invalid("media");
            }
            let offset = a.uint("offset")?;
            let length = a.uint("length")?;
            if length == 0 || length > MAX_MEDIA_READ {
                return invalid("length");
            }
            CommandBody::MediaRead {
                media: media.to_owned(),
                offset,
                length,
            }
        }
        unknown => CommandBody::Unknown {
            command: unknown.to_owned(),
            args: args.clone(),
        },
    };
    Ok(Command { id, body })
}

fn start_args(f: &Fields) -> Checked<StartArgs> {
    let title = f.optional_string("title")?;
    if title
        .as_ref()
        .is_some_and(|title| title.chars().count() > MAX_TITLE_CHARS)
    {
        return invalid("title");
    }
    let tracks = match f.0.get("tracks") {
        None | Some(Value::Null) => None,
        Some(_) => Some(f.tracks("tracks")?),
    };
    let screen = match f.optional_string("screen")? {
        None => None,
        Some(screen) => match ScreenMode::parse(&screen) {
            Some(screen) => Some(screen),
            None => return invalid("screen"),
        },
    };
    let context = match f.0.get("context") {
        None | Some(Value::Null) => None,
        Some(Value::Object(context)) => {
            let size = serde_json::to_vec(context).map_or(usize::MAX, |json| json.len());
            if size > MAX_CONTEXT_BYTES {
                return invalid("context");
            }
            Some(context.clone())
        }
        Some(_) => return invalid("context"),
    };
    Ok(StartArgs {
        title,
        tracks,
        screen,
        context,
    })
}

fn reply(f: &Fields) -> Checked<Reply> {
    let id = f.short_text("id")?;
    let ok = f.boolean("ok")?;
    let outcome = if ok {
        Ok(f.object("result")?.clone())
    } else {
        Err(reply_error(f.object("error")?).ok_or_else(|| Invalid::new("error"))?)
    };
    Ok(Reply { id, outcome })
}

fn reply_error(error: &Object) -> Option<ReplyError> {
    let f = Fields(error);
    Some(ReplyError {
        code: f.id_text("code").ok()?,
        message: f.string("message").ok()?.to_owned(),
        meeting: f.optional_meeting("meeting").ok()?,
    })
}

/// Typed reads of an object's fields. Every failure names the field.
///
/// "Required" fields must be there; "nullable" ones must be there and may be `null`; "optional"
/// ones may be left out, and `null` counts as left out.
struct Fields<'a>(&'a Object);

impl<'a> Fields<'a> {
    fn present(&self, key: &str) -> Checked<&'a Value> {
        self.0.get(key).ok_or_else(|| Invalid::new(key))
    }

    fn optional(&self, key: &str) -> Option<&'a Value> {
        self.0.get(key).filter(|value| !value.is_null())
    }

    /// An integer of 0 or more.
    ///
    /// Integers are JSON numbers without a fraction, so `1.0` fails as well as `1.5`.
    fn uint(&self, key: &str) -> Checked<u64> {
        uint_value(self.present(key)?).ok_or_else(|| Invalid::new(key))
    }

    /// An integer of 1 or more, as `seq` and `revision` are.
    fn positive(&self, key: &str) -> Checked<u64> {
        match self.uint(key)? {
            0 => invalid(key),
            value => Ok(value),
        }
    }

    fn u32(&self, key: &str) -> Checked<u32> {
        u32::try_from(self.uint(key)?).map_err(|_| Invalid::new(key))
    }

    /// A protocol version: versions count from 1.
    fn version(&self, key: &str) -> Checked<u32> {
        match self.u32(key)? {
            0 => invalid(key),
            version => Ok(version),
        }
    }

    fn optional_uint(&self, key: &str) -> Checked<Option<u64>> {
        self.optional(key)
            .map(|value| uint_value(value).ok_or_else(|| Invalid::new(key)))
            .transpose()
    }

    fn nullable_uint(&self, key: &str) -> Checked<Option<u64>> {
        self.present(key)?;
        self.optional_uint(key)
    }

    fn boolean(&self, key: &str) -> Checked<bool> {
        self.present(key)?
            .as_bool()
            .ok_or_else(|| Invalid::new(key))
    }

    fn string(&self, key: &str) -> Checked<&'a str> {
        self.present(key)?.as_str().ok_or_else(|| Invalid::new(key))
    }

    fn optional_string(&self, key: &str) -> Checked<Option<String>> {
        match self.optional(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => invalid(key),
        }
    }

    fn nullable_string(&self, key: &str) -> Checked<Option<&'a str>> {
        match self.present(key)? {
            Value::Null => Ok(None),
            Value::String(text) => Ok(Some(text)),
            _ => invalid(key),
        }
    }

    /// A non-empty string, for ids and codes.
    fn id_text(&self, key: &str) -> Checked<String> {
        match self.string(key)? {
            "" => invalid(key),
            text => Ok(text.to_owned()),
        }
    }

    /// 1 to 64 characters, as a command id and the `client` name are.
    fn short_text(&self, key: &str) -> Checked<String> {
        let text = self.string(key)?;
        if text.is_empty() || text.chars().count() > MAX_ID_CHARS {
            return invalid(key);
        }
        Ok(text.to_owned())
    }

    fn object(&self, key: &str) -> Checked<&'a Object> {
        self.present(key)?
            .as_object()
            .ok_or_else(|| Invalid::new(key))
    }

    fn nullable_object(&self, key: &str) -> Checked<Option<&'a Object>> {
        match self.present(key)? {
            Value::Null => Ok(None),
            Value::Object(object) => Ok(Some(object)),
            _ => invalid(key),
        }
    }

    fn token(&self, key: &str) -> Checked<String> {
        match self.string(key)? {
            token if is_token(token) => Ok(token.to_owned()),
            _ => invalid(key),
        }
    }

    fn instance(&self, key: &str) -> Checked<String> {
        match self.string(key)? {
            instance if is_instance(instance) => Ok(instance.to_owned()),
            _ => invalid(key),
        }
    }

    fn meeting(&self, key: &str) -> Checked<MeetingId> {
        MeetingId::parse(self.string(key)?).ok_or_else(|| Invalid::new(key))
    }

    fn optional_meeting(&self, key: &str) -> Checked<Option<MeetingId>> {
        match self.optional(key) {
            None => Ok(None),
            Some(_) => self.meeting(key).map(Some),
        }
    }

    fn nullable_meeting(&self, key: &str) -> Checked<Option<MeetingId>> {
        self.present(key)?;
        self.optional_meeting(key)
    }

    fn named<T>(&self, key: &str, parse: fn(&str) -> Option<T>) -> Checked<T> {
        parse(self.string(key)?).ok_or_else(|| Invalid::new(key))
    }

    /// A non-empty list of `mic` and `call` without repeats.
    fn tracks(&self, key: &str) -> Checked<Vec<Track>> {
        let Some(list) = self.present(key)?.as_array() else {
            return invalid(key);
        };
        let mut tracks = Vec::with_capacity(list.len());
        for item in list {
            match item.as_str().and_then(Track::parse) {
                Some(track) if !tracks.contains(&track) => tracks.push(track),
                _ => return invalid(key),
            }
        }
        if tracks.is_empty() {
            return invalid(key);
        }
        Ok(tracks)
    }
}

fn uint_value(value: &Value) -> Option<u64> {
    // serde_json keeps a number written with a fraction or an exponent as a float, so as_u64
    // accepts exactly the JSON integers of 0 or more.
    value.as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOKEN: &str = "q7hR0lJ3x2b8Yc5vN1mK4sT9wE6uA0dF3gH2jL5pZ8o";

    fn reason(from: Side, value: Value) -> String {
        validate_incoming(from, &value).unwrap_err().reason
    }

    fn command(id: &str, name: &str, args: Value) -> Value {
        json!({ "type": "command", "id": id, "command": name, "args": args })
    }

    #[test]
    fn messages_go_one_way() {
        let hello = json!({ "type": "hello", "protocol": 1, "token": TOKEN, "client": "rukoo", "instance": null, "after": 0 });
        assert!(validate_incoming(Side::Rukoo, &hello).is_ok());
        assert_eq!(reason(Side::Voice, hello), "type");
        let reply = json!({ "type": "reply", "id": "c-1", "ok": true, "result": {} });
        assert!(validate_incoming(Side::Voice, &reply).is_ok());
        assert_eq!(reason(Side::Rukoo, reply), "type");
        assert_eq!(reason(Side::Rukoo, json!([1, 2])), "type");
        assert_eq!(reason(Side::Rukoo, json!({ "protocol": 1 })), "type");
    }

    #[test]
    fn integers_have_no_fraction() {
        let hello = |after: Value| json!({ "type": "hello", "protocol": 1, "token": TOKEN, "client": "rukoo", "instance": null, "after": after });
        assert_eq!(reason(Side::Rukoo, hello(json!(1.0))), "after");
        assert_eq!(reason(Side::Rukoo, hello(json!(-1))), "after");
        assert_eq!(reason(Side::Rukoo, hello(json!("1"))), "after");
        assert!(validate_incoming(Side::Rukoo, &hello(json!(u64::MAX))).is_ok());
        let too_big = json!({ "type": "hello", "protocol": 4_294_967_296u64, "token": TOKEN, "client": "rukoo", "instance": null, "after": 0 });
        assert_eq!(reason(Side::Rukoo, too_big), "protocol");
    }

    #[test]
    fn hello_needs_the_instance_key() {
        let hello = json!({ "type": "hello", "protocol": 1, "token": TOKEN, "client": "rukoo", "after": 0 });
        assert_eq!(reason(Side::Rukoo, hello), "instance");
        let long_client = json!({ "type": "hello", "protocol": 1, "token": TOKEN, "client": "r".repeat(65), "instance": null, "after": 0 });
        assert_eq!(reason(Side::Rukoo, long_client), "client");
    }

    #[test]
    fn hello_takes_an_optional_pid_of_one_or_more() {
        let hello = |pid: Value| json!({ "type": "hello", "protocol": 1, "token": TOKEN, "client": "rukoo", "pid": pid, "instance": null, "after": 0 });
        assert_eq!(reason(Side::Rukoo, hello(json!(0))), "pid");
        assert_eq!(reason(Side::Rukoo, hello(json!(1.5))), "pid");
        assert_eq!(reason(Side::Rukoo, hello(json!(-3))), "pid");
        assert_eq!(reason(Side::Rukoo, hello(json!("4821"))), "pid");
        let parsed = validate_incoming(Side::Rukoo, &hello(json!(4821))).unwrap();
        assert_eq!(
            parsed,
            Message::Hello(Hello {
                protocol: 1,
                token: TOKEN.into(),
                client: "rukoo".into(),
                client_version: None,
                pid: Some(4821),
                instance: None,
                after: 0,
            })
        );
        // An older Rukoo leaves the pid out, and null counts as left out.
        let no_pid = json!({ "type": "hello", "protocol": 1, "token": TOKEN, "client": "rukoo", "instance": null, "after": 0 });
        for value in [no_pid, hello(Value::Null)] {
            let Message::Hello(hello) = validate_incoming(Side::Rukoo, &value).unwrap() else {
                panic!("not a hello")
            };
            assert_eq!(hello.pid, None);
        }
    }

    #[test]
    fn start_arguments_have_limits() {
        let meeting = "m-20261030-171203-4f2a";
        let ok =
            |args: Value| validate_incoming(Side::Rukoo, &command("c-1", "meeting.start", args));
        assert!(ok(json!({ "title": "t".repeat(200) })).is_ok());
        assert_eq!(
            ok(json!({ "title": "t".repeat(201) })).unwrap_err().reason,
            "title"
        );
        assert_eq!(ok(json!({ "tracks": [] })).unwrap_err().reason, "tracks");
        assert_eq!(
            ok(json!({ "tracks": ["call", "call"] }))
                .unwrap_err()
                .reason,
            "tracks"
        );
        assert_eq!(ok(json!({ "tracks": "mic" })).unwrap_err().reason, "tracks");
        let big = json!({ "context": { "blob": "x".repeat(MAX_CONTEXT_BYTES) } });
        assert_eq!(ok(big).unwrap_err().reason, "context");
        let fits = json!({ "context": { "x": "y".repeat(MAX_CONTEXT_BYTES - 8) } });
        assert!(ok(fits).is_ok());
        assert!(ok(json!({ "title": null, "context": null })).is_ok());
        let args = command("c-1", "meeting.start", json!("nope"));
        assert_eq!(reason(Side::Rukoo, args), "args");
        let mark = command(
            "c-2",
            "meeting.mark",
            json!({ "meeting": meeting, "label": 5 }),
        );
        assert_eq!(reason(Side::Rukoo, mark), "label");
    }

    #[test]
    fn screen_commands_have_limits() {
        let meeting = "m-20261030-171203-4f2a";
        let check = |name: &str, args: Value| {
            validate_incoming(Side::Rukoo, &command("c-1", name, args)).map(|m| match m {
                Message::Command(Command { body, .. }) => body,
                other => panic!("not a command: {other:?}"),
            })
        };
        let clip = check(
            "clip.get",
            json!({ "meeting": meeting, "start": 5, "end": 5 }),
        )
        .unwrap();
        assert_eq!(
            clip.args(),
            json!({ "meeting": meeting, "start": 5, "end": 5 })
        );
        let reason = |name: &str, args: Value| check(name, args).unwrap_err().reason;
        assert_eq!(
            reason(
                "clip.get",
                json!({ "meeting": meeting, "start": 6, "end": 5 })
            ),
            "end"
        );
        assert_eq!(
            reason(
                "clip.get",
                json!({ "meeting": meeting, "start": 1.5, "end": 5 })
            ),
            "start"
        );
        assert_eq!(
            reason("clip.get", json!({ "start": 1, "end": 5 })),
            "meeting"
        );
        assert_eq!(reason("meeting.protected", json!({})), "meeting");
        assert_eq!(reason("screen.get", json!({})), "meeting");
        let select = |monitor: Value| {
            check(
                "screen.select",
                json!({ "meeting": meeting, "monitor": monitor }),
            )
        };
        assert_eq!(
            select(json!(r"\\.\DISPLAY2")).unwrap().args()["monitor"],
            r"\\.\DISPLAY2"
        );
        assert!(select(json!("m".repeat(64))).is_ok());
        // Characters, as Rukoo counts them: 64 emoji fit.
        assert!(select(json!("😀".repeat(64))).is_ok());
        assert_eq!(select(json!("")).unwrap_err().reason, "monitor");
        assert_eq!(select(json!("m".repeat(65))).unwrap_err().reason, "monitor");
    }

    #[test]
    fn status_screen_default_is_optional() {
        let status = |data: Value| json!({ "type": "event", "seq": 5, "event": "status", "createdAt": 0, "data": data });
        let read = |data: Value| match validate_incoming(Side::Voice, &status(data)).unwrap() {
            Message::Event(Event {
                body: EventBody::Status(status),
                ..
            }) => status,
            other => panic!("not a status: {other:?}"),
        };
        let given =
            read(json!({ "recording": null, "dictating": false, "screenDefault": "video" }));
        assert_eq!(given.screen_default, Some(ScreenMode::Video));
        let null = read(json!({ "recording": null, "dictating": false, "screenDefault": null }));
        assert_eq!(null.screen_default, None);
        assert_eq!(
            reason(
                Side::Voice,
                status(json!({ "recording": null, "dictating": false, "screenDefault": "4k" }))
            ),
            "screenDefault"
        );
        let starting = json!({ "type": "event", "seq": 5, "event": "meeting.starting", "createdAt": 0,
            "data": { "screen": "frames" } });
        assert_eq!(reason(Side::Voice, starting), "meeting");
    }

    #[test]
    fn list_fills_in_the_default_limit() {
        let list = command("c-1", "meeting.list", json!({}));
        let Message::Command(Command { body, .. }) = validate_incoming(Side::Rukoo, &list).unwrap()
        else {
            panic!("not a command");
        };
        assert_eq!(body, CommandBody::MeetingList { limit: 50 });
    }

    #[test]
    fn unknown_commands_keep_their_arguments() {
        let unknown = command("c-1", "example.unknown", json!({ "from": 1 }));
        let Message::Command(Command { body, .. }) =
            validate_incoming(Side::Rukoo, &unknown).unwrap()
        else {
            panic!("not a command");
        };
        assert_eq!(body.name(), "example.unknown");
        assert_eq!(body.args(), json!({ "from": 1 }));
    }

    /// The body of a command from Rukoo, or the reason it was refused.
    fn command_body(name: &str, args: Value) -> Result<CommandBody, String> {
        match validate_incoming(Side::Rukoo, &command("c-1", name, args)) {
            Ok(Message::Command(Command { body, .. })) => Ok(body),
            Ok(other) => panic!("not a command: {other:?}"),
            Err(invalid) => Err(invalid.reason),
        }
    }

    #[test]
    fn media_open_takes_a_range_of_an_hour_at_most() {
        let meeting = "m-20261030-171203-4f2a";
        let open = |kind, start, end| {
            Ok(CommandBody::MediaOpen(MediaOpenArgs {
                meeting: MeetingId::parse(meeting).unwrap(),
                kind,
                start,
                end,
            }))
        };
        let args = |kind: &str, start: Value, end: Option<Value>| {
            let mut args = json!({ "meeting": meeting, "kind": kind, "start": start });
            if let Some(end) = end {
                args["end"] = end;
            }
            command_body("media.open", args)
        };

        assert_eq!(
            args("audio", json!(600_000), Some(json!(1_200_000))),
            open(MediaKind::Audio, 600_000, Some(1_200_000))
        );
        // `end` is optional, and null counts as left out.
        assert_eq!(
            args("frame", json!(412_500), None),
            open(MediaKind::Frame, 412_500, None)
        );
        assert_eq!(
            args("frame", json!(0), Some(Value::Null)),
            open(MediaKind::Frame, 0, None)
        );
        // An empty range and exactly an hour pass; the 300 s video cap is Voice's answer, not a check.
        assert_eq!(
            args("audio", json!(5_000), Some(json!(5_000))),
            open(MediaKind::Audio, 5_000, Some(5_000))
        );
        assert_eq!(
            args("video", json!(0), Some(json!(MAX_MEDIA_SPAN_MS))),
            open(MediaKind::Video, 0, Some(MAX_MEDIA_SPAN_MS))
        );
        assert_eq!(
            args("audio", json!(1), Some(json!(MAX_MEDIA_SPAN_MS + 2))).unwrap_err(),
            "end"
        );
        assert_eq!(
            args("audio", json!(5_000), Some(json!(4_999))).unwrap_err(),
            "end"
        );
        assert_eq!(
            args("audio", json!(0), Some(json!(1.5))).unwrap_err(),
            "end"
        );
        assert_eq!(
            args("audio", json!(-1), Some(json!(10))).unwrap_err(),
            "start"
        );
        assert_eq!(args("audio", json!("0"), None).unwrap_err(), "start");
        assert_eq!(args("transcript", json!(0), None).unwrap_err(), "kind");
        assert_eq!(args("Audio", json!(0), None).unwrap_err(), "kind");
        // A video range far past the end of any meeting still passes: Voice cuts it to what it has.
        assert_eq!(
            args("video", json!(u64::MAX - 1), Some(json!(u64::MAX))),
            open(MediaKind::Video, u64::MAX - 1, Some(u64::MAX))
        );

        // Fields are checked in the order meeting, kind, start, end, as Rukoo checks them.
        let bad = |value: Value| command_body("media.open", value).unwrap_err();
        assert_eq!(
            bad(json!({ "meeting": "m-1", "kind": "x", "end": 1 })),
            "meeting"
        );
        assert_eq!(bad(json!({ "meeting": meeting, "end": 1 })), "kind");
        assert_eq!(
            bad(json!({ "meeting": meeting, "kind": "audio", "end": -1 })),
            "start"
        );
    }

    #[test]
    fn media_read_reads_one_mebibyte_at_most() {
        let media = "md-a1B2c3D4e5F6g7H8i9J0kL";
        let read = |args: Value| command_body("media.read", args);
        let body = |offset, length| {
            Ok(CommandBody::MediaRead {
                media: media.to_owned(),
                offset,
                length,
            })
        };

        assert_eq!(
            read(json!({ "media": media, "offset": 0, "length": 1 })),
            body(0, 1)
        );
        assert_eq!(
            read(json!({ "media": media, "offset": u64::MAX, "length": MAX_MEDIA_READ })),
            body(u64::MAX, MAX_MEDIA_READ)
        );
        let reason = |args: Value| read(args).unwrap_err();
        assert_eq!(
            reason(json!({ "media": media, "offset": 0, "length": 0 })),
            "length"
        );
        assert_eq!(
            reason(json!({ "media": media, "offset": 0, "length": MAX_MEDIA_READ + 1 })),
            "length"
        );
        assert_eq!(reason(json!({ "media": media, "offset": 0 })), "length");
        assert_eq!(
            reason(json!({ "media": media, "offset": -1, "length": 1 })),
            "offset"
        );
        assert_eq!(
            reason(json!({ "media": media, "offset": null, "length": 1 })),
            "offset"
        );
        for id in [
            "md-short",
            "MD-a1B2c3D4e5F6g7H8i9J0kL",
            "md-a1B2c3D4e5F6g7H8i9J0k+",
            "md-a1B2c3D4e5F6g7H8i9J0kLx",
            "a1B2c3D4e5F6g7H8i9J0kL",
        ] {
            assert_eq!(
                reason(json!({ "media": id, "offset": 0, "length": 1 })),
                "media",
                "{id}"
            );
        }
        assert_eq!(
            reason(json!({ "media": 5, "offset": 0, "length": 1 })),
            "media"
        );

        // Fields are checked in the order media, offset, length.
        assert_eq!(reason(json!({ "offset": -1, "length": 0 })), "media");
        assert_eq!(
            reason(json!({ "media": media, "offset": -1, "length": 0 })),
            "offset"
        );
    }

    #[test]
    fn media_commands_serialize_back_to_their_arguments() {
        let meeting = "m-20261030-171203-4f2a";
        for args in [
            json!({ "meeting": meeting, "kind": "audio", "start": 600_000, "end": 1_200_000 }),
            json!({ "meeting": meeting, "kind": "frame", "start": 412_500 }),
            json!({ "meeting": meeting, "kind": "video", "start": 0, "end": 300_000 }),
        ] {
            let body = command_body("media.open", args.clone()).unwrap();
            assert_eq!(body.name(), "media.open");
            assert_eq!(body.args(), args);
        }
        // A null `end` is left out when the command is written again.
        let body = command_body(
            "media.open",
            json!({ "meeting": meeting, "kind": "frame", "start": 0, "end": null }),
        )
        .unwrap();
        assert_eq!(
            body.args(),
            json!({ "meeting": meeting, "kind": "frame", "start": 0 })
        );

        let args = json!({ "media": "md-a1B2c3D4e5F6g7H8i9J0kL", "offset": 1_048_576, "length": 1_048_576 });
        let body = command_body("media.read", args.clone()).unwrap();
        assert_eq!(body.name(), "media.read");
        assert_eq!(body.args(), args);
    }

    #[test]
    fn marks_name_a_known_track_or_none() {
        let mark = |track: Value| {
            json!({ "type": "event", "seq": 5, "event": "mark", "meeting": "m-20261030-171203-4f2a", "createdAt": 0,
                    "data": { "id": "k-1", "kind": "dropped", "at": 5, "until": null, "track": track, "detail": "disk", "label": null } })
        };
        assert!(validate_incoming(Side::Voice, &mark(json!("screen"))).is_ok());
        assert!(validate_incoming(Side::Voice, &mark(json!(null))).is_ok());
        assert_eq!(reason(Side::Voice, mark(json!("room"))), "track");
    }

    #[test]
    fn listed_fields_must_be_there_even_when_null() {
        let passage = json!({ "type": "event", "seq": 5, "event": "passage", "meeting": "m-20261030-171203-4f2a", "createdAt": 0,
            "data": { "id": "mic-000000010", "track": "mic", "start": 10, "end": 10, "text": "", "revision": 1, "dictation": false, "language": null } });
        assert_eq!(reason(Side::Voice, passage), "speaker");
    }

    #[test]
    fn reply_errors_need_a_code_and_message() {
        let reply =
            json!({ "type": "reply", "id": "c-1", "ok": false, "error": { "code": "busy" } });
        assert_eq!(reason(Side::Voice, reply), "error");
        let reply = json!({ "type": "reply", "id": "c-1", "ok": false, "error": { "code": "busy", "message": "x", "meeting": "bad" } });
        assert_eq!(reason(Side::Voice, reply), "error");
    }

    #[test]
    fn discovery_ports_and_pids() {
        let file = json!({ "protocol": 1, "minProtocol": 1, "port": 65535, "token": TOKEN, "pid": 1, "instance": "Vb3kQ9m2X7pL1sR4tY8wZQ", "voiceVersion": "0.1.0", "startedAt": 0 });
        assert_eq!(validate_discovery(&file).unwrap().port, 65535);
        let mut no_pid = file.clone();
        no_pid.as_object_mut().unwrap().remove("pid");
        assert_eq!(validate_discovery(&no_pid).unwrap_err().reason, "pid");
        assert_eq!(
            validate_discovery(&json!("x")).unwrap_err().reason,
            "protocol"
        );
    }
}
