# Rukoo Voice bridge protocol, version 2

Rukoo Voice records meetings and Rukoo shows them. This document is the contract between the two: how Rukoo finds Voice, how it connects, and the events and commands they exchange. Voice implements it in `voice/crates/voice-protocol` and `voice/crates/voice-bridge`, Rukoo in `src/main/voice/`. A change here needs the matching change on both sides in the same pull request, and both sides test against the same examples in [fixtures/protocol-vectors.json](fixtures/protocol-vectors.json).

Your agent never talks to Voice. It reaches a meeting through Rukoo's MCP tools.

## Encoding

- Every message is one UTF-8 JSON object in one WebSocket text frame. Field names are camelCase.
- Wall times are milliseconds since the Unix epoch, in fields ending in `At`, such as `startedAt`.
- Meeting time is milliseconds since the meeting started, as an integer, in the fields `at`, `start`, `end` and `until`. Audio and screen share this clock, so a passage and a frame with the same meeting time happened together. Pauses don't stop the clock.
- A meeting id is `m-`, the start time in UTC as `YYYYMMDD-HHMMSS`, a dash and four lowercase hex digits: `m-20261030-171203-4f2a`.
- Tokens and instance ids are base64url without padding (RFC 4648 §5).
- A peer ignores fields it doesn't know.

## Finding Voice

Voice writes a discovery file when it starts and removes it when it quits:

```json
{
  "protocol": 2,
  "minProtocol": 2,
  "port": 49821,
  "token": "q7hR0lJ3x2b8Yc5vN1mK4sT9wE6uA0dF3gH2jL5pZ8o",
  "pid": 12345,
  "instance": "Vb3kQ9m2X7pL1sR4tY8wZQ",
  "voiceVersion": "0.1.0",
  "startedAt": 1793000000000
}
```

- `port` is the WebSocket port on `127.0.0.1`. Voice never listens on another address.
- `token` is 32 random bytes, made fresh every time Voice starts.
- `instance` is 16 random bytes, made once per Voice data folder. It changes only when the data folder is new, which also restarts the sequence numbers.
- `pid` is Voice's process id. Rukoo may use it to tell a stale file from a running Voice, but it decides by connecting.

Only the user's account can read the file. On Windows it lives in the user's profile and Voice gives it an access list that grants only that user. On macOS and Linux it has mode `0600` in a folder with mode `0700`. Voice writes it to a temporary name and renames it into place, so Rukoo never reads half a file.

### Where the file is

Both apps work out the same path.

1. If `RUKOO_VOICE_BRIDGE` is set, it is the full path of the file. Both apps use it.
2. Otherwise Voice uses `bridge.json` in its data folder: `RUKOO_VOICE_DATA_DIR` when set, otherwise the default folder: `%LOCALAPPDATA%\Rukoo Voice` on Windows, `~/Library/Application Support/Rukoo Voice` on macOS and `$XDG_STATE_HOME/rukoo-voice` on Linux, where `XDG_STATE_HOME` defaults to `~/.local/state`.
3. Otherwise Rukoo uses `voice-bridge.json` in its profile when `SEM_DATA_DIR` is set, and `bridge.json` in Voice's default folder when it isn't. Rukoo doesn't read `RUKOO_VOICE_DATA_DIR`.

An empty variable counts as unset. A Voice started with `RUKOO_VOICE_DATA_DIR` and a Rukoo started with `SEM_DATA_DIR` never find the installed apps. To pair two development copies, give both the same `RUKOO_VOICE_BRIDGE`.

## Connecting

Rukoo opens `ws://127.0.0.1:<port>/bridge` and sends `hello` as its first message, within 5 seconds. Voice refuses the upgrade with HTTP 403 when the request carries an `Origin` header, whatever its value. Any web page in a browser can try to open a socket on `127.0.0.1`, and a browser sends `Origin` with every WebSocket request, `null` from a sandboxed frame or a local file among them. Rukoo connects from its main process, which sends no `Origin`.

At most 16 connections wait for their upgrade or `hello` at a time. A new connection closes the oldest of them, so connections that never say `hello` can't keep Rukoo out: Rukoo sends `hello` as soon as the upgrade is done.

```json
{ "type": "hello", "protocol": 2, "token": "…", "client": "rukoo", "clientVersion": "2.2.0", "pid": 4821, "instance": "Vb3kQ9m2X7pL1sR4tY8wZQ", "after": 1041 }
```

- `protocol` is the highest version Rukoo speaks that is also within the file's `minProtocol` to `protocol`.
- `pid` is Rukoo's process id: the process that owns its windows. It is optional, and it is not the discovery file's `pid`, which is Voice's. Voice passes it to its screen process, which leaves that process's windows out of the recording where the system can filter by application (macOS). Without a `pid`, Voice relies on Rukoo hiding its own windows, as it does on Windows.
- `instance` and `after` are the instance and the last sequence number Rukoo saw. On its first connection Rukoo sends `null` and `0`.

Voice compares the token in constant time and answers with `welcome`:

```json
{ "type": "welcome", "protocol": 2, "voiceVersion": "0.1.0", "instance": "Vb3kQ9m2X7pL1sR4tY8wZQ", "head": 1052, "oldest": 1, "replay": "partial", "status": { "recording": null, "dictating": false } }
```

Then Voice sends the events Rukoo missed, oldest first, and after them every new event as it happens. `replay` says which events follow:

| `replay` | When | What Voice sends |
|---|---|---|
| `partial` | `instance` matches and `oldest` − 1 ≤ `after` ≤ `head` | the events after `after` |
| `full` | `instance` is `null` or another instance, or `after` is above `head` | every stored event, from `oldest` |
| `gap` | `instance` matches, but `after` < `oldest` − 1: events Rukoo missed were already removed | every stored event, from `oldest` |

With no stored events, `head` is the last number Voice gave out, or 0, and `oldest` is `head` + 1.

After a `gap`, Rukoo asks `meeting.get` for each meeting it shows, also those that are `done`: Voice may have deleted one, and the `meeting.removed` that said so may be among the events Rukoo missed. A `not_found` answer counts as that `meeting.removed`.

A wrong token, an unsupported version or a late or missing `hello` gets an `error` message and the connection closes:

```json
{ "type": "error", "code": "bad_token", "message": "The token doesn't match." }
```

| Close code | `code` | Meaning |
|---|---|---|
| 4001 | `bad_token` | The token is wrong |
| 4002 | `unsupported_protocol` | Voice doesn't speak the requested version |
| 4003 | `hello_timeout` | No `hello` within 5 seconds, or another message first |
| 4004 | `too_many_clients` | Four clients are already connected |
| 1001 | | Voice is quitting |

Voice sends a WebSocket ping every 20 seconds and closes a connection that doesn't answer within 20 seconds. Rukoo reconnects with backoff, starting at 1 second and doubling up to 30 seconds, and starts again at once when the discovery file changes.

A message from Rukoo is at most 1 MiB. A message from Voice is at most 8 MiB.

## Events

Voice sends events. Every event has a sequence number, and Voice stores events, so Rukoo can ask for everything after the last number it saw.

```json
{ "type": "event", "seq": 1042, "event": "passage", "meeting": "m-20261030-171203-4f2a", "createdAt": 1793000000123, "data": { } }
```

- `seq` starts at 1 and goes up with every event, across restarts of Voice. Only a new data folder, which brings a new `instance`, starts it again. A number is never given out twice.
- `meeting` is there for meeting events and missing for the others.
- Applying an event twice gives the same result. Things with an id replace the earlier version with the same id.
- Rukoo ignores event types it doesn't know.
- Voice keeps a meeting's events as long as it keeps the meeting. Of the `status` events it keeps only the newest: a new one replaces the stored one. So the numbers in a replay can skip, and Rukoo doesn't expect them to follow each other. `oldest` in `welcome` is the lowest stored number.

### `status`

What Voice is doing right now. `welcome` carries the same object. Every dictation sends two, so Voice stores only the newest.

```json
{ "recording": "m-20261030-171203-4f2a", "dictating": false, "screenDefault": "frames" }
```

`recording` is the meeting being recorded or paused, or `null`. `dictating` is true while the user holds a dictation key. `screenDefault` is optional: the `screen` a meeting gets when `meeting.start` leaves it out, one of the values of `meeting.started`. Voice sends a new `status` when it changes.

### `meeting.started`

```json
{ "title": "Weekly with Anna", "startedAt": 1793000000000, "tracks": ["mic", "call"], "screen": "off", "startedBy": "rukoo", "context": { "calendarEvent": "abc123" } }
```

- `tracks` are the audio tracks Voice records: `mic` is the user's microphone, `call` the computer's audio output, which holds the other people in a call.
- `screen` is `off`, `frames` or `video`: what Voice records of the screen in this meeting, from `meeting.start` or else Voice's Settings. A Voice that can't record the screen on this computer starts the meeting with `off`. A `video_dropped` mark says when Voice stopped the video, and a `dropped` mark when it stopped recording the screen.
- `startedBy` is `voice` or `rukoo`.
- `context` is the object `meeting.start` passed, unchanged, or `null`. Voice stores it and doesn't read it.

### `meeting.starting`

Voice is about to record the screen in a meeting it started itself, from its pill, its tray or a key.

```json
{ "screen": "frames" }
```

Voice records the audio already. Before it captures the first frame, it waits up to 500 ms for every connected Rukoo to send `meeting.protected`, so Rukoo can hide its windows first. A Rukoo that hides its windows during meetings does so, waits for the next desktop composition and then sends `meeting.protected`; a Rukoo that doesn't hide them sends it at once. With no Rukoo connected, Voice doesn't wait, unless a Rukoo was connected earlier while Voice ran: then Voice waits up to 5 seconds for one to connect, sends it a new `meeting.starting` after its `welcome`, and waits up to 500 ms for its answer.

Voice sends no `meeting.starting` for a meeting Rukoo started, because Rukoo hides its windows before it sends `meeting.start`, nor for a meeting that records no screen. A `meeting.starting` that comes in a replay, up to the `head` of `welcome`, is history, and Rukoo doesn't answer it. `meeting.get` leaves it out.

### `meeting.state`

```json
{ "state": "processing", "at": 3605120, "endedAt": 1793003605120, "reason": "user" }
```

| `state` | Meaning |
|---|---|
| `recording` | Voice records |
| `paused` | Voice records nothing until it resumes; the clock keeps running |
| `processing` | Recording ended; Voice is still transcribing what it recorded |
| `done` | Every recorded second is transcribed |
| `failed` | Voice can't finish the transcript; `reason` says why. The audio stays |

- `at` is the meeting time of the change.
- `endedAt` comes with `processing` and with every state after it.
- `reason` comes with `processing` and `failed`: `user` when someone pressed Stop, `rukoo` when Rukoo sent `meeting.stop`, `interrupted` when Voice found the meeting cut off at its next start, `limit` when the meeting reached Voice's length limit, or an error text for `failed`.

### `speaker`

A voice in the meeting, as Voice labels it.

```json
{ "id": "call-1", "track": "call", "label": "Speaker 1" }
```

The user's microphone is always the speaker `mic` with the label `You`. Voice doesn't tell apart people who share the microphone.

The call track's speakers depend on speaker labels in Voice's Settings:

- Without labels, the whole call is one speaker, `call-1`, labelled `Speaker 1`. Voice sends it when the meeting starts.
- With labels, each voice Voice tells apart gets its own id, `call-1`, `call-2` and so on, labelled `Speaker 1`, `Speaker 2` with the same number. Voice sends a speaker right before the first passage that names it, so a meeting can show only `You` until someone on the call speaks. Numbers follow the order in which Voice first heard each voice; a number can be missing when a voice never got a passage.
- An id belongs to one voice for the whole meeting, also when Voice's labelling restarts during the meeting, and Voice never gives it to another voice. Voice can still hear one person as two voices, and Rukoo then shows two speakers.
- A `speaker` event for an id Rukoo already has carries the same `track` and `label`.

Rukoo owns the names people give speakers; Voice never learns them. Rukoo keeps a name on the speaker id, so it follows every passage that has that id.

### `passage`

A stretch of transcript.

```json
{ "id": "call-000065320", "track": "call", "speaker": "call-1", "start": 65320, "end": 71880, "text": "Let's move the launch to Thursday.", "revision": 1, "dictation": false, "language": "en" }
```

- `id` is the track, a dash and the start in milliseconds as nine digits. It stays the same when Voice transcribes the passage again.
- A passage with a higher `revision` replaces the one Rukoo has. Voice sends a new revision when a better transcript or a settled speaker arrives.
- `speaker` is a speaker id, or `null` while Voice doesn't know. With speaker labels, a call passage can arrive with `null` when Voice couldn't tell whose words they were, and a later revision can name the speaker. Voice never moves a passage from one speaker id to another.
- `dictation` is true when the user was dictating into another app while the meeting recorded. Those words reached the call, but weren't part of the conversation.
- `language` is a BCP 47 code when the recognizer reports one, otherwise `null`.

### `passage.removed`

```json
{ "id": "call-000065320" }
```

Voice removed a passage, for example when it transcribed a stretch again with other boundaries.

### `mark`

Something that happened at a moment in the meeting.

```json
{ "id": "k-3", "kind": "gap", "at": 120400, "until": 126900, "track": "mic", "detail": "device", "label": null }
```

| `kind` | Meaning | `until` | `detail` |
|---|---|---|---|
| `user` | Someone marked this moment | `null` | `null` |
| `dictation` | The user dictated into another app | end of the dictation | `null` |
| `gap` | The track has no audio | end of the gap | `crash`, `device` or `suspend` |
| `device` | The track moved to another device; `label` is its name | `null` | `default`, `reconnected` or `lost` |
| `dropped` | Voice stopped recording the screen, to keep recording audio | `null` | `disk`, `encoder`, `device` or `crash` |
| `video_dropped` | Voice stopped the screen video, to keep recording audio; the frames go on | `null` | `disk`, `encoder` or `device` |
| `screen_gap` | The screen has no frames | end of the gap | `locked`, `crash`, `device` or `monitor` |
| `monitor` | Voice moved the screen recording to another monitor; `label` is its name | `null` | `user` or `lost` |

- `track` is `mic`, `call`, `screen` or `null` when the mark is about the whole meeting. `dropped`, `video_dropped`, `screen_gap` and `monitor` come with `screen`; `gap` never does.
- `label` is the text the user gave a `user` mark, the device name of a `device` mark or the monitor name of a `monitor` mark.
- On a `screen_gap`, `locked` means the computer was locked or its session disconnected, `crash` that Voice's screen recorder restarted, `device` that the graphics device was reset, and `monitor` that no monitor was left to record. On `dropped`, `device` means the graphics device couldn't be started again, and `crash` that Voice's screen recorder stopped five times in a row, so Voice gave up on it for this meeting. On `video_dropped`, `device` means the graphics device was reset and didn't come back with the video support the encoder needs; the frames go on, unless no device came back at all, when a `dropped` with `device` follows at once. `lost` on a `monitor` mark means the monitor Voice recorded went away. When the screen recording stops for good, Voice sends `dropped`; when the video had stopped before, a `video_dropped` came first.
- Voice sends a mark once it is complete. A gap is sent when the audio or the screen comes back, or the meeting ends.
- Rukoo shows a mark of a `kind` it doesn't know as a plain mark.

### `frame`

A moment the screen changed noticeably, in a meeting that records the screen.

```json
{ "id": "f-000412000", "at": 412000, "width": 1920, "height": 1080, "text": "Q3 roadmap" }
```

- `id` is `f-` and `at` as nine digits. A frame Voice sends again has the same `id`.
- `width` and `height` are the size of the image in pixels.
- `text` is the text Voice read on the screen, on this computer: at most 8,000 characters, lines separated by `\n`, or `""` when it read nothing.
- Voice sends a frame once its text is read, which can be a few seconds after `at`. Rukoo fetches the image with `frame.get`.

### `meeting.removed`

Voice removed the meeting and its events. `data` is `{}` for retention or an ordinary removal. An explicit user deletion sends `{ "reason": "user" }`; every connected Rukoo then purges its notes, names, corrections and cached transcript, and remembers the deleted id so stale replay cannot restore it. Chats remain, with a deleted-meeting label.

## Commands

Rukoo sends commands; Voice answers each with one reply.

```json
{ "type": "command", "id": "c-17", "command": "meeting.start", "args": { "title": "Weekly with Anna" } }
```

```json
{ "type": "reply", "id": "c-17", "ok": true, "result": { "meeting": "m-20261030-171203-4f2a" } }
```

```json
{ "type": "reply", "id": "c-18", "ok": false, "error": { "code": "busy", "message": "A meeting is already recording.", "meeting": "m-20261030-171203-4f2a" } }
```

`id` is Rukoo's own, at most 64 characters, and comes back unchanged. Voice answers commands in order, except that `meeting.protected` bypasses the queue so it can acknowledge a `screen.set` waiting on that same connection. Events may arrive before a command's reply.

| Command | `args` | `result` | Errors |
|---|---|---|---|
| `meeting.start` | `title`?, `tracks`?, `screen`?, `context`? | `{ "meeting" }` | `busy` |
| `meeting.stop` | `meeting` | `{}` | `not_found`, `not_recording` |
| `meeting.pause` | `meeting` | `{}` | `not_found`, `not_recording` |
| `meeting.resume` | `meeting` | `{}` | `not_found`, `not_recording` |
| `meeting.mark` | `meeting`, `label`? | `{ "mark", "at" }` | `not_found`, `not_recording` |
| `meeting.list` | `limit`?, default 50 | `{ "meetings": [ … ] }` | |
| `meeting.get` | `meeting` | a snapshot, see below | `not_found` |
| `meeting.delete` | `meeting` | `{}` | `not_found`, `busy`, `failed` |
| `frame.get` | `meeting`, `at` | `{ "frame", "image" }` | `not_found`, `not_available` |
| `clip.get` | `meeting`, `start`, `end` | `{ "clips": [ … ] }` | `not_found`, `not_available`, `failed` |
| `meeting.protected` | `meeting`, `request`? | `{}` | `not_found` |
| `screen.get` | `meeting` | the screen recording, see below | `not_found`, `not_recording`, `not_available` |
| `screen.select` | `meeting`, `monitor` | `{ "monitor" }` | `not_found`, `not_recording`, `not_available` |
| `screen.set` | `meeting`, `screen`, `monitor`? | `{ "screen" }` | `not_found`, `not_recording`, `not_available`, `failed` |
| `media.open` | `meeting`, `kind`, `start`, `end`? | `{ "media", "ready", … }` | `not_found`, `not_available`, `bad_request`, `failed` |
| `media.read` | `media`, `offset`, `length` | `{ "offset", "data" }` | `not_found`, `bad_request`, `failed` |
| `status.get` | | the `status` object | |

- `meeting.start` uses the Settings default for every argument left out. `title` is at most 200 characters, `tracks` a non-empty list of `mic` and `call`, `screen` one of the values of `meeting.started`, and `context` an object of at most 16 KiB.
- `meeting.list` returns the newest meetings first: `{ "id", "title", "startedAt", "endedAt", "state", "duration" }`, with `endedAt` `null` while recording and `duration` in milliseconds.
- `meeting.get` returns `{ "meeting", "speakers", "passages", "marks", "frames", "head" }`. `meeting` holds the fields of `meeting.started` plus `id`, `state` and `endedAt`. The lists hold the current version of everything, and `head` is the sequence number the snapshot is complete up to. `frames` holds the data of the meeting's `frame` events, oldest first; an older Voice leaves it out. To keep the reply small, older frames can come with `"text": ""` and `"textOmitted": true`; `frame.get` returns their text.
- `frame.get` returns the frame at or before `at` as `{ "frame", "image" }`: `frame` is the data of its `frame` event, `image` is `{ "type": "image/jpeg", "data": "<base64>" }` in standard base64 with padding. Once Voice removed the meeting's screen media with its audio, `image` is `null` and `frame` still carries the text read from the screen, so a frame a snapshot sent with `textOmitted` keeps its text. It answers `not_found` when no frame is at or before `at`, and `not_available` when the meeting didn't record the screen.
- `clip.get` cuts the meeting's video between `start` and `end` without encoding it again; `end` ≥ `start`. `clips` lists one file for each stretch of continuous video, oldest first: `{ "path", "start", "end", "width", "height", "bytes" }`. `path` is the full path of an MP4 file on this computer, which Voice deletes after an hour; copy it to keep it. A clip starts at a keyframe at or before `start` and ends at or after `end`, so its own `start` and `end` say what it holds. Voice cuts at most 5 minutes from `start` and answers within 4 seconds of receiving the command with the clips it finished by then. A longer cut goes on: `clips` is empty when nothing was finished yet, and the same command a little later returns the files. It answers `not_found` when the meeting has no video between `start` and `end`, `not_available` when the meeting didn't record video or its video was removed, and `failed` when the disk is too full to cut a clip. Rukoo sends one `clip.get` at a time on a connection.
- `meeting.delete` accepts only finished or failed meetings. Recording, paused or processing meetings and active media preparation return `busy`. Voice removes the meeting folder and prepared media, writes a durable encrypted-bucket index tombstone even when offline, and emits `meeting.removed` with reason `user`. Encrypted bucket objects remain subject to the bucket's lifecycle rules. Interrupted local deletions finish on startup. Rukoo owns renamed titles locally; no rename or delete MCP tool is added.
- `meeting.protected` answers `meeting.starting`: Rukoo applied its window protection. A mid-meeting announcement carries a unique `request` (1 to 64 characters); the acknowledgement must echo it. Each connected client must acknowledge that round. Duplicate or stale acknowledgements cannot stand in for another client.
- `screen.set` chooses `off`, `frames` or `video` during recording or pause; an optional monitor is selected before capture starts. Off releases capture and closes the current video without ending the process or permanently dropping either part. Enabling from an audio-only meeting starts the screen process on the existing recorder clock. Before enabling capture, Voice sends `meeting.starting` with a request id and waits up to 2 seconds for every connected Rukoo to protect its windows. Failure leaves the previous mode unchanged. The command deadline is 15 seconds. Every actual mode change adds a `screen_mode` mark with `track: "screen"`, `detail: "user"` and the mode as `label`, at the journal's exact meeting time. `meeting.started.screen` stays historical; `meeting.get.meeting.initialScreen` holds that value and `meeting.get.meeting.screen` reports the current mode. Frame lookup never crosses an Off interval, and video clips contain only the recorded stretches.
- `screen.get` returns `{ "screen", "state", "video", "frames", "monitor", "monitors" }`. `screen` is the current mode. `state` is `off`, `starting`, `recording`, `paused`, `locked` or `stopped`. `video` and `frames` say whether each part still records. `monitor` is the id of the monitor Voice records, or `null`. `monitors` lists `{ "id", "name", "width", "height", "primary" }` for every monitor, left to right. It answers `not_recording` for a meeting that doesn't record now and `not_available` when screen recording is unavailable on this computer. Audio-only live meetings return Off and the available monitors. Like `clip.get`, it answers within 4 seconds of receiving the command; when the screen recorder doesn't answer by then, the state is what Voice knew last.
- `screen.select` moves the screen recording to the monitor with that id from `screen.get`, and returns the id. It answers `not_found` for a monitor that isn't there, and `not_available` when the screen recording stopped. It answers within 4 seconds of receiving the command, `failed` when the screen recorder is still starting or didn't switch by then; Rukoo can ask again.
- `media.open` prepares a file of a meeting's media that Rukoo can hand to an agent, and names it with an id. `kind` is `audio`, `frame` or `video`:
  - `audio`: a WAV file of all the meeting's tracks mixed into one, 16 kHz mono 16-bit PCM with the 44-byte header, from `start` to `end`. Gaps are silence.
  - `frame`: the JPEG of the frame at or before `start`. `end` is ignored.
  - `video`: an MP4 of the screen from `start` to `end`, cut at fragment boundaries without re-encoding, so it can begin a few seconds before `start`. It holds one stretch of continuous video: when the video breaks before `end`, at a pause or a monitor switch, the file ends there. Voice serves at most 300,000 ms of video, the limit of `clip.get`, and answers `bad_request` for more.

  `end` is required for `audio` and `video`. Voice cuts the range to what was recorded. Preparing can take a while, for instance when Voice fetches the media back from the user's bucket, so Voice answers at once. While the file isn't ready, the result is `{ "media": "md-…", "ready": false }`, and Rukoo sends the same `media.open` again, no more than every 250 ms. A ready file gives `{ "media", "ready": true, "type", "size", "start", "end" }`: `type` is `audio/wav`, `image/jpeg` or `video/mp4`, `size` the file's length in bytes, and `start` and `end` the meeting times the file covers (both the frame's `at` for a frame). The same arguments give the same `media` while Voice keeps the file.
- `media.read` returns up to `length` bytes of a prepared file from `offset` as `{ "offset", "data" }`, with `data` in standard base64. It returns fewer bytes at the end of the file and an empty `data` past it. `length` is at most 1,048,576, so a reply stays far below Voice's 8 MiB limit and one command never holds the connection for long. Voice keeps a file for 10 minutes after it was last read and answers `not_found` afterwards; Rukoo then opens it again.
- `media.open` answers `not_found` when the meeting or the frame doesn't exist or nothing was recorded at `start`, and `not_available` when the meeting didn't record that kind of media, or its media is gone from this computer and isn't in a synced bucket. `media.read` answers `bad_request` for a file that isn't ready yet.

Errors carry a `code` and an English `message` for logs. Rukoo shows its own text for a code.

| `code` | Meaning |
|---|---|
| `bad_request` | The command or its arguments don't match this document |
| `unknown_command` | Voice doesn't know the command; a newer Rukoo may send commands an older Voice lacks |
| `not_found` | No meeting, frame, monitor or prepared media with that id or time |
| `not_recording` | The meeting isn't recording, or isn't paused for `meeting.resume` |
| `busy` | A meeting is already recording; `meeting` says which |
| `not_available` | Voice can't do this for this meeting or in this version |
| `failed` | Voice tried and failed; `message` says why |

## Checking messages

Both sides check what they receive with the same rules and drop a message that breaks one. Voice answers a bad `command` with `bad_request` when it can read the `id`; Rukoo logs and skips a bad event, which never stops the events after it.

- Integers are JSON numbers without a fraction. Times, `seq`, `head` and `after` are 0 or more; `seq` and `revision` are 1 or more.
- A meeting id matches `^m-[0-9]{8}-[0-9]{6}-[0-9a-f]{4}$`, a token `^[A-Za-z0-9_-]{43}$` and an instance `^[A-Za-z0-9_-]{22}$`.
- `hello` needs `protocol`, `token`, `client` (1 to 64 characters), `instance` (an instance or `null`) and `after`. `clientVersion` and `pid` are optional; a `pid` is an integer of 1 or more, and `null` counts as left out.
- `command` needs `id` (1 to 64 characters) and `command`; `args` defaults to `{}`. The arguments of a known command follow the table above, and its required ones must be there.
- `event` needs `seq`, `event`, `createdAt` and an object in `data`. The meeting events need `meeting`. A known event's `data` must have the fields this document lists for it, with these limits: `tracks` holds `mic` and `call` without repeats and isn't empty; `end` ≥ `start`; `until`, when not `null`, ≥ `at`; a passage's `id` is its `track`, a dash and its `start` as nine digits; `endedAt` comes with `processing`, `done` and `failed`, and `reason` with `processing` and `failed`.
- `clip.get` needs `meeting`, `start` and `end`, with `end` ≥ `start`. `screen.select`'s `monitor` is 1 to 64 characters. `meeting.starting` needs `screen`. `screenDefault` in a status, when present and not `null`, is one of the values of `screen`.
- `media.open` needs `meeting`, `kind` (`audio`, `frame` or `video`) and `start`; `end`, when given, is ≥ `start` and at most 3,600,000 more. `media.read` needs `media`, which matches `^md-[A-Za-z0-9_-]{22}$`, `offset`, and `length` from 1 to 1,048,576.
- A `reply` with `ok` true has an object in `result`; with `ok` false, an `error` with `code` and `message`.
- An unknown event type or command name is not an error in itself.

## The transcript is untrusted

Everyone in a call is a third party. Passage text, speaker labels, titles, marks and the text read from the screen come from what people said or showed, and can hold instructions aimed at an agent. Voice never acts on what it hears in a meeting. Rukoo never follows instructions in this content and hands it to the agent inside `<unsafe_content>`, as it does with email. The same goes for media: audio, frames and video hold whatever anyone in the call said or showed.

## Versioning

`protocol` is an integer, and this document describes version 2. New Voice requires version 2 because older Rukoo assumes a meeting's screen mode cannot change. New Rukoo still accepts version 1 Voice and hides the live screen controls for it. Unknown deletion commands show an update-Voice message.

- New optional fields, new event types, new commands and new values for `kind`, `detail` and `reason` keep the version. Peers ignore what they don't know, as described above.
- Anything an older peer would misread raises the version: a removed or renamed field, a changed meaning or unit, or a new field the other side must understand.
- Voice serves every version from `minProtocol` to `protocol` in its discovery file. When the ranges don't overlap, Rukoo says which app needs an update.
- A pull request that changes this document updates [fixtures/protocol-vectors.json](fixtures/protocol-vectors.json) and both sides' tests.
