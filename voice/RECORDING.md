# Rukoo Voice meeting folder, version 1

A meeting recording is a folder of plain files: a journal of what happened, and the audio in short WAV segments. The recorder, `rukoo-voice-recorder`, writes them, and nothing else does. It runs as its own process, so transcription, the app and Rukoo can crash while the recording goes on. A hard kill at any moment leaves files that open in any WAV reader and lose at most about the last 0.2 s of audio. A meeting that records the screen also has a `screen/` folder, which the screen process `rukoo-voice-screen` writes; see [The screen folder](#the-screen-folder).

The recorder lives in `voice/crates/voice-recorder`, the segment writer and reader in `voice/crates/voice-audio`. The app reads the folder through `voice/crates/voice-meetings`.

## Layout

```
<data>/meetings/<meeting id>/
  meeting.json      written once by the app before the recorder starts
  timeline.jsonl    the journal; the recorder is its only writer
  recorder.json     how to reach the live recorder; removed on a clean stop
  audio/mic/000001.wav, 000002.wav, ...
  audio/call/000001.wav, ...
  transcript.db     the app's transcript and speaker turns; the recorder never opens it
  screen/           frames, video and the screen journal; see "The screen folder"
```

`<data>` is Voice's data folder and the meeting id has the form in [PROTOCOL.md](PROTOCOL.md#encoding). `mic` is the user's microphone and `call` the computer's audio output, which holds the other people in a call.

`meeting.json` is the app's description of the meeting. The app writes it to a temporary name and renames it into place. The recorder doesn't read it.

```json
{ "id": "m-20261030-171203-4f2a", "title": "Weekly with Anna", "createdAt": 1793000000000, "tracks": ["mic", "call"], "sources": { "mic": "default-input", "call": "default-output" }, "screen": "off", "startedBy": "voice", "context": null, "retention": "days7", "version": 1 }
```

`retention` is how long the meeting's audio stays after it is transcribed, as Settings said when the meeting started: `days7`, `keep` or `after_transcription`. A later change in Settings applies to new meetings only, as in Ownkey. A `meeting.json` without it follows the current setting.

## Audio segments

Every segment is 16 kHz, mono, signed 16-bit little-endian PCM with the canonical 44-byte WAV header, the format of Ownkey's meeting chunks and of the speech models. The recorder captures each device in its own format and converts it.

- A segment holds contiguous audio that starts at the meeting time in its `segment` line. It ends at that time plus its samples divided by 16, in whole milliseconds.
- Each track numbers its segments from `000001`. Numbering continues across sessions and a number is never used twice. A resumed recorder continues after the highest number in the timeline or in the folder, whichever is higher, and it never overwrites a file.
- A new segment starts after every 30 s of audio, and after a device change or loss, a pause, a resume, a suspend, the start of a session, and when the recorder fell so far behind that it dropped audio.
- After 30 s, the next segment starts exactly where the previous one ended. When the device's clock has drifted more than 100 ms from the meeting clock, it starts at the clock's time instead, so two neighbouring segments of a track can overlap or leave a gap of about 0.1 s. After any other break, a segment never starts before the end of the track's previous one.

### Writing, in crash-safe order

1. The recorder creates the file, refusing an existing one, with a header that counts 0 samples: RIFF size 36, data size 0.
2. It writes the `segment` line.
3. Every 100 to 200 ms it appends the new samples, then rewrites the RIFF size (offset 4) and the data size (offset 40) in a single write. The samples come first, so the header never counts samples that aren't on disk.
4. It syncs the file to disk every 5 s, and when the segment closes.
5. It closes the segment and writes the `segment_end` line.

A hard kill can leave three things: a file the timeline doesn't name, holding only its header; samples at the end of a file that the header doesn't count yet, at most one append; and half a sample. A power cut can lose up to the last 5 s.

While it records, the recorder keeps a second writer out of the timeline and of every segment it has open: on Windows it shares them for reading and deleting only, and on macOS and Linux it holds an exclusive lock on each open file. Other processes can read them, and a second writer can't open them. The lock goes with the process, so a recorder that was hard-killed leaves its meeting writable.

### Reading

- When the header's data size is smaller than the file, it wins: the rest is an append the writer hadn't counted.
- A data size of 0 or `0xFFFFFFFF` means unknown. Use the file length, rounded down to whole samples.
- `finalize` and `--resume` repair the header from the file length. That recovers an uncounted last append and drops a half sample.

### Silence

The recorder never writes silence to cover a gap: a lost device, a crash, a pause, a suspend or dropped audio stays a gap, and the timeline says why.

The one exception is real silence on a loopback track (`default-output` or `output:<id>`). The loopback endpoint delivers nothing while nothing plays, so while its device is healthy, the recorder pads the track with zeros from the clock once it trails the clock by more than 0.5 s, and again up to the moment of a pause or stop. This is what Ownkey does (`meetings/capture.py:414-418`). Padding stops the moment the device reports an error, and it never trims a track that runs ahead.

## The timeline

`timeline.jsonl` holds one JSON object per line. The recorder writes each line with a single write and a trailing newline, and starts every object with `kind`.

- A reader ignores a last line without a newline, any line that doesn't parse, and kinds and fields it doesn't know. Before a recorder or `finalize` writes to a timeline, it cuts off a last line without a newline.
- `t` is meeting time in milliseconds. Session 1 starts at `t = 0`; its `session` line holds the wall time of that moment. A later session starts at `t` = the wall time now minus that of session 1, but never earlier than anything already recorded, even when the wall clock went back. Within a session `t` follows the monotonic clock, and after a suspend it jumps ahead by the time the computer slept, never by a step of the wall clock. Pauses don't stop the clock.
- On Windows and macOS, `session` and `heartbeat` lines also carry `boot`: a clock that counts sleep, read right after `t`, in 100 ns units. It is the Windows interrupt time on Windows and `mach_continuous_time` on macOS. It counts sleep and the meeting clock follows the same counter, so within a session `t + (boot_now - boot) / 10,000` is the meeting time of any later moment, read in any process. `wall` can't do that: in a `session` line it is read 11 to 27 ms before the clock starts, and it follows time syncs. Linux has no shared counter that counts sleep, so the field is missing there.
- Lines appear in the order they were written, which is close to the order of `t` but not exactly: `segment_end` carries the end computed from the samples, a padded loopback track writes its `segment` line up to 0.5 s after the segment's `t`, and `mark` and `dictation` carry the moment their request arrived, or the moment the recorder ended a dictation itself.

| `kind` | Fields | When |
|---|---|---|
| `session` | `t`, `wall`, `boot`, `session`, `pid`, `resumed`, `version`, `recorder` | A recorder starts recording into the meeting. `session` counts from 1, `version` is the format version 1, `recorder` the recorder's version. `boot` is the clock that counts sleep when `t` was read, in 100 ns units: the Windows interrupt time, or `mach_continuous_time` on macOS. It is missing on Linux |
| `track` | `t`, `track`, `source`, `device`, `deviceId`, `nativeRate`, `nativeChannels` | A track opened a device: at the start, after a loss, after a move and after a suspend. `device` is its name |
| `segment` | `t`, `track`, `seq`, `file` | A segment starts at `t`. `file` is relative to the meeting folder, with forward slashes |
| `segment_end` | `t`, `track`, `seq`, `samples` | A segment closed; `t` is its end |
| `heartbeat` | `t`, `wall`, `boot`, `samples` | Every second. `samples` maps each track with an open segment to the samples on disk in it; a track without one is missing. `boot` as in `session` |
| `device_lost` | `t`, `track`, `error` | A track has no device: it failed to open, the device failed, a microphone sent nothing for 5 s, or the disk refused the audio. Its segment closed |
| `device` | `t`, `track`, `device`, `reason` | A track got a device back, `reconnected`, or moved to another one, `default`. A `track` line follows |
| `pause`, `resume` | `t` | The user paused or resumed |
| `mark` | `t`, `id`, `label` | A user mark. `id` is `k-<n>` and `n` keeps counting across sessions; `label` is `null` without one |
| `dictation` | `t`, `state` | The app reports dictation on the microphone: `start` or `end`. Only a change is written. The app reads the microphone through a tap and closes it when the user stops dictating, quits or crashes, so the recorder writes `end` itself when the microphone's last tap goes and a dictation is open. A dictation the dead session left open ends at `lastSeen` |
| `suspend` | `t`, `until` | The computer slept for more than 3 s: since the last heartbeat, more time passed by a clock that counts sleep than the computer was awake. A stalled recorder or a step of the wall clock doesn't count. `t` is the last heartbeat before it, `until` the meeting time after it. The segments closed before the line; `track` lines follow as the devices reopen |
| `crash` | `t`, `lastSeen` | Written by a resumed session: the previous one died, and audio is missing from `lastSeen` to `t` |
| `stop` | `t`, `wall`, `reason` | Recording ended: `user`, `rukoo`, `limit` or `interrupted`. Nothing follows |

A microphone that was unplugged for two seconds, with the heartbeats after the first left out:

```json
{"kind":"session","t":0,"wall":1793384408939,"boot":123456789012,"session":1,"pid":14152,"resumed":false,"version":1,"recorder":"0.1.0"}
{"kind":"track","t":33,"track":"mic","source":"default-input","device":"Microphone (USB Audio CODEC)","deviceId":"wasapi:{0.0.1.00000000}.{00000000-0000-0000-0000-000000000001}","nativeRate":48000,"nativeChannels":2}
{"kind":"segment","t":41,"track":"mic","seq":1,"file":"audio/mic/000001.wav"}
{"kind":"heartbeat","t":1050,"wall":1793384409989,"boot":123467289012,"samples":{"mic":16144}}
{"kind":"segment_end","t":1985,"track":"mic","seq":1,"samples":31104}
{"kind":"device_lost","t":2002,"track":"mic","error":"The requested audio device is not available. It may have been disconnected."}
{"kind":"device","t":4076,"track":"mic","device":"Microphone (USB Audio CODEC)","reason":"reconnected"}
{"kind":"track","t":4076,"track":"mic","source":"default-input","device":"Microphone (USB Audio CODEC)","deviceId":"wasapi:{0.0.1.00000000}.{00000000-0000-0000-0000-000000000001}","nativeRate":48000,"nativeChannels":2}
{"kind":"segment","t":4089,"track":"mic","seq":2,"file":"audio/mic/000002.wav"}
{"kind":"segment_end","t":8153,"track":"mic","seq":2,"samples":65024}
{"kind":"stop","t":8160,"wall":1793384417099,"reason":"user"}
```

`reconnected` means the track opened the same device id as before, or its first device. `default` means the id changed: the default device moved, or the track came back on a different default.

`lastSeen` is the highest of: the dead session's start, its last heartbeat `t`, its last `segment_end` `t`, and for each track the last segment's `t` plus its samples on disk divided by 16.

A resumed session writes, in this order: `segment_end` for every segment the dead session left open, after repairing its header; `dictation` `end` at `lastSeen` when the dead session left a dictation open, because the app's tap went with it; `session` with `resumed` true; `crash`; `pause` when the meeting was paused, because a restart must not record a user who paused; then `track` for each device.

## The recorder

```
rukoo-voice-recorder record --meeting <dir> [--resume] --track mic=<source> [--track call=<source>]
rukoo-voice-recorder finalize --meeting <dir> [--reason interrupted]
rukoo-voice-recorder devices
```

| Source | Records |
|---|---|
| `default-input` | The default microphone, moving with the default |
| `default-output` | Loopback of the default output device, moving with the default; padded |
| `input:<id>` | One microphone, by an `id` from `devices` |
| `output:<id>` | Loopback of one output device, by an `id` from `devices`; padded |
| `synthetic:tone:<hz>` | For tests: a sine at half scale, 48 kHz stereo, in real-time 10 ms blocks |
| `synthetic:idle` | For tests: a healthy loopback with nothing playing, which delivers nothing; padded |
| `file:<path>[,loop]` | A WAV file in real time, then silence, or looped. For end-to-end tests with recorded speech |

The synthetic sources take `,fail-at=<ms>` (the device disappears, reported through the same path as a real loss), `,back-at=<ms>` (it can be opened again), `,switch-at=<ms>` (another device becomes the default) and `,open-ms=<ms>` (every open takes this long on the recorder's thread, as a Bluetooth headset switching profile can). The times count from the start of the session. Device sources work on Windows only in this version; elsewhere `record` refuses them with exit code 2.

Two options exist for tests: `--limit-ms <ms>` replaces the 8-hour limit and `--segment-ms <ms>` the 30 s segment length.

| Exit code | Meaning |
|---|---|
| 0 | Done |
| 1 | Failed, for example a disk error at start |
| 2 | Wrong arguments, or a source this platform can't record |
| 3 | The meeting doesn't allow it: `record` without `--resume` into a meeting that has a recording, or any `record` after `stop` |
| 4 | Another recorder is recording into the meeting |

### `record`

- It refuses to start while `recorder.json` names a recorder that answers on its port.
- `--resume` continues a meeting whose recorder died. With an empty timeline it starts session 1.
- It captures each device in the device's own format and converts it to 16 kHz mono.
- A `default-*` track checks the default device every 2 s and moves to a new one. It also rebuilds its stream when Windows reports the default changed.
- A failed device closes the segment and writes `device_lost`, timed when the device reported the failure, or where the track's audio ends if that is later. The track retries every second: a `default-*` track opens whatever is the default, an `input:` or `output:` track waits for its own device. Unplugging and Bluetooth profile switches show up this way.
- It stops on the `stop` command, when the meeting reaches 8 hours (reason `limit`), or when it's killed. It doesn't stop when its parent exits. It never reads stdin, writes nothing to stdout, and reports problems on stderr.
- On stop it pads healthy loopback tracks up to that moment, closes every segment, writes `stop`, removes `recorder.json` and only then replies.
- On Windows the app starts it detached: no console, its own process group, and outside any kill-on-close job, so it outlives the app.

### `recorder.json`

```json
{ "pid": 14152, "port": 51544, "token": "q7hR0lJ3x2b8Yc5vN1mK4sT9wE6uA0dF3gH2jL5pZ8o", "session": 1, "startedAt": 1793384408939 }
```

The recorder writes it after the `session` line, to a temporary name that it renames into place. `token` is 32 random bytes as base64url, made fresh for every session. A recorder is live only when it answers `status` on that port with that token; the file alone, or a running `pid`, proves nothing.

### `finalize`

`finalize` finishes a meeting whose recorder died: it repairs every segment header from the file length, writes `segment_end` for segments that have none, `dictation` `end` when a dictation is open, and `stop` at `lastSeen` with the given reason, `interrupted` by default, when the timeline has no `stop`. It removes a stale `recorder.json`. Running it again changes nothing. It refuses a live meeting with exit code 4, and prints:

```json
{ "ok": true, "ended": 2, "stopped": true, "t": 1528, "problems": [] }
```

`stopped` says whether it wrote `stop` now, and `t` is that line's time. `problems` lists segments it couldn't repair, such as a missing file.

### `devices`

```json
{ "inputs": [{ "id": "wasapi:{0.0.1.00000000}.{00000000-0000-0000-0000-000000000001}", "name": "Microphone (USB Audio CODEC)", "default": true }], "outputs": [{ "id": "wasapi:{0.0.0.00000000}.{00000000-0000-0000-0000-000000000002}", "name": "Speakers", "default": true }] }
```

## The control port

The recorder listens on `127.0.0.1` at the port in `recorder.json`. A client sends one JSON object per line and gets one line back for each. Every request carries the token, and a connection can carry several requests. The first request must arrive within 5 s of connecting, or the recorder closes the connection; after it, a connection may stay idle.

```json
{ "token": "q7hR0lJ3x2b8Yc5vN1mK4sT9wE6uA0dF3gH2jL5pZ8o", "cmd": "mark", "label": "Decision" }
```

| `cmd` | Arguments | Reply |
|---|---|---|
| `status` | | `{"ok":true,"state":"recording","t":5120,"tracks":{"mic":{"device":"…","ok":true}}}`; `state` is `recording` or `paused`; a track without a device has `"device":null,"ok":false` and an `error` |
| `stop` | `reason`: `user` (default) or `rukoo` | `{"ok":true,"t":8160}` once `stop` is written; then the recorder exits |
| `pause` | | `{"ok":true,"t":…}`, or `not_recording` when already paused |
| `resume` | | `{"ok":true,"t":…}`, or `not_recording` when not paused |
| `mark` | `label`, optional, at most 1000 characters | `{"ok":true,"id":"k-3","t":…}`; also while paused |
| `dictation` | `state`: `start` or `end` | `{"ok":true,"t":…}`, also when it changes nothing |
| `tap` | `track` | `{"ok":true,"rate":16000}`, then the raw PCM of that track; `unknown_track` when the meeting doesn't record it, `no_device` while the track has no device |

Errors are `{"ok":false,"error":"<code>"}`:

| `error` | Meaning | Connection |
|---|---|---|
| `bad_token` | The token is missing or wrong | closed |
| `bad_request` | The line isn't a JSON object, or the arguments are wrong | closed for a line that isn't JSON, open otherwise |
| `unknown_command` | The recorder doesn't know `cmd` | open |
| `not_recording` | `pause` while paused, `resume` while recording | open |
| `unknown_track` | `tap` for a track the meeting doesn't record | open |
| `no_device` | `tap` for a track that has no device now; ask again once `status` says it has one | open |

A request line is at most 64 KiB, and the recorder serves at most 16 connections at a time.

After a successful `tap` the connection carries raw signed 16-bit little-endian PCM, 16 kHz, mono, as the track captures it, until the client closes it, the track loses its device, the computer sleeps or the recording stops. The recorder notices within about 0.25 s that a client closed its end, also while the track sends nothing, so a closed tap never keeps one of the 16 connections. The tap carries audio also while the meeting is paused, so dictation during a pause still works, and it never carries padding. A client more than about 5 s behind is disconnected rather than handed audio with holes in it. During a meeting the app reads the microphone for dictation from this tap, so the microphone is captured once.

## The screen folder

A meeting that records the screen, with `screen` set to `frames` or `video` in `meeting.json`, gets a `screen/` folder. The screen process, `rukoo-voice-screen`, writes it. It runs detached like the recorder, so a graphics driver, encoder or OCR crash never costs audio. It reads `timeline.jsonl` and never writes it, and the recorder knows nothing about it. The app reads `screen/`. It writes there only the clips in `screen/clips/`, and it runs the screen executable's `finalize` when a screen process died.

The screen process lives in `voice/crates/voice-screen`, the format and its readers in `voice/crates/voice-screen-files`.

```
<data>/meetings/<meeting id>/screen/
  journal.jsonl     the screen journal; the screen process is its only writer
  screen.json       how to reach the live screen process; removed on a clean stop
  video/000001.mp4, 000002.mp4, ...       fragmented MP4, H.264, one per continuous stretch
  frames/f-000412000.jpg, ...             one JPEG per saved frame; the name holds its t
  clips/c-000060000-000075000-000001.mp4  cut by clip.get; deleted an hour after their last use
```

Relative paths in the journal use forward slashes and start with `screen/`.

### Meeting time on the screen

`t` in the screen journal is meeting time in milliseconds on the recorder's clock, so a frame and a passage with the same time show the same moment. The screen process takes its anchor from the latest `session` or `heartbeat` line of the recorder's current session and maps a moment to meeting time with `t = anchor.t + (boot - anchor.boot) / 10,000`, where `boot` is the moment's clock that counts sleep: the interrupt time on Windows, `mach_continuous_time` on macOS. A captured frame's own moment comes from its capture timestamp, read back to back with the current value of that clock, so only the frame's age of a few milliseconds goes through another counter. Without `boot` in the timeline, it uses `t = anchor.t + (wall - anchor.wall)` and writes `anchor` `wall` in its `session` line.

- Time never steps back: a frame captured before the last time handed out gets that time.
- A new recorder `session` line moves the anchor. When the new mapping differs from the old one by more than 50 ms at the same moment, the open video file ends with `video_end` reason `clock` and a new one starts, so each file maps to meeting time with one offset.
- A video file's media time 0 is the `t` of its `video` line; a sample's meeting time is that `t` plus its media time. Each sample keeps its frame's capture time, rounded to the file's timescale of 1/15,000 s, whichever encoder wrote it, so the frame rate follows the screen up to the `video` line's `fps`.

### `screen.json`

```json
{ "pid": 18220, "port": 51603, "token": "Zx3kQ9m2X7pL1sR4tY8wZQb3kQ9m2X7pL1sR4tY8wZQ", "session": 1, "startedAt": 1793384409366 }
```

The screen process writes it after its `session` line, to a temporary name that it renames into place, and removes it on a clean stop. As with `recorder.json`, the token is 32 random bytes as base64url, new for every session, and the process is live only when it answers `status` on that port with that token. The file holds the token, so it never syncs and never goes into a log.

### The screen journal

`journal.jsonl` follows the timeline's rules: one JSON object per line, written with a single write and a trailing newline, `kind` first. A reader ignores a last line without a newline, lines that don't parse, and kinds and fields it doesn't know; a writer cuts off a last line without a newline before it appends. While it records, the screen process keeps a second screen process out of the journal: on Windows with read and delete sharing only, on macOS and Linux with an exclusive lock on the open file.

| `kind` | Fields | When |
|---|---|---|
| `session` | `t`, `wall`, `boot`, `session`, `pid`, `resumed`, `version`, `mode`, `anchor`, `screen` | The process starts recording into the meeting. `mode` is `off`, `frames` or `video`, `anchor` is `boot` or `wall`, the clock it maps with, `version` is the format version 1 and `screen` the screen process's version. `boot` is missing on Linux |
| `monitor` | `t`, `id`, `name`, `width`, `height`, `primary`, `reason` | Capture moves to a monitor: `start`, `resume`, `user` or `lost`. `id` is the platform's monitor id: the Windows device name such as `\\.\DISPLAY1`, or the display id on macOS |
| `capture` | `t`, `wall`, `monitor`, `width`, `height` | The first frame of a capture session arrived: at the start, after a switch, an unlock, a resume, a new graphics device or a monitor that came back |
| `video` | `t`, `seq`, `file`, `width`, `height`, `fps`, `encoder`, `hardware` | A video file starts; its media time 0 is meeting time `t`. `encoder` is the encoder's name and `hardware` whether it is a hardware encoder |
| `video_end` | `t`, `seq`, `reason`, `bytes` | A video file ended; `t` is the end of its last sample. `reason` is `pause`, `lock`, `monitor`, `resize`, `clock`, `stop`, `error`, `disk` or `crash` |
| `frame` | `t`, `id`, `file`, `width`, `height`, `text`, `ocr`, `language` | A frame was saved and read. `text` is what the platform's text reader read on it, lines separated by `\n`, at most 8,000 characters; `ocr` is `ok`, `none`, `skipped` or `failed`, and `language` the OCR language or `null` |
| `dropped` | `t`, `what`, `reason` | A part stopped for the rest of the meeting: `what` is `video` or `frames`, `reason` is `disk`, `encoder` or `device` |
| `lock`, `unlock` | `t` | The session locked or disconnected, and came back. Nothing is captured in between |
| `device_lost` | `t` | The graphics device was removed or reset. The next `capture` line ends the gap |
| `no_monitor` | `t` | No monitor is left to record, or the one `--monitor` pinned went away. Written once per wait; the next `capture` line ends it |
| `heartbeat` | `t`, `wall`, `boot`, `frames`, `videoBytes` | Every 2 s: the frames saved and the bytes of the open video file |
| `crash` | `t`, `lastSeen` | Written by a resumed session: the previous one died, and the screen is missing from `lastSeen` to `t` |
| `stop` | `t`, `reason` | Recording the screen ended: `meeting` with the recorder's `stop`, `dropped` when both parts were dropped, `failed` when the app gave up restarting it, `interrupted` when the recording ended without it, such as when the recorder was gone. Nothing follows |

`lastSeen` is the highest `t` of any line of the dead session, or the end of its last complete video fragment when that is later.

A frames meeting with one slide change, locked for a minute, the heartbeats left out:

```json
{"kind":"session","t":412,"wall":1793384409366,"boot":123460909012,"session":1,"pid":18220,"resumed":false,"version":1,"mode":"frames","anchor":"boot","screen":"0.1.0"}
{"kind":"monitor","t":413,"id":"\\\\.\\DISPLAY1","name":"DELL S3220DGF","width":2560,"height":1440,"primary":true,"reason":"start"}
{"kind":"capture","t":470,"wall":1793384409424,"monitor":"\\\\.\\DISPLAY1","width":2560,"height":1440}
{"kind":"frame","t":470,"id":"f-000000470","file":"screen/frames/f-000000470.jpg","width":2560,"height":1440,"text":"Weekly with Anna\nAgenda","ocr":"ok","language":"nl-NL"}
{"kind":"frame","t":12880,"id":"f-000012880","file":"screen/frames/f-000012880.jpg","width":2560,"height":1440,"text":"Q3 roadmap","ocr":"ok","language":"nl-NL"}
{"kind":"lock","t":30200}
{"kind":"unlock","t":91000}
{"kind":"capture","t":91090,"wall":1793384500044,"monitor":"\\\\.\\DISPLAY1","width":2560,"height":1440}
{"kind":"frame","t":91090,"id":"f-000091090","file":"screen/frames/f-000091090.jpg","width":2560,"height":1440,"text":"Q3 roadmap","ocr":"ok","language":"nl-NL"}
{"kind":"stop","t":120000,"reason":"meeting"}
```

### Writing, in crash-safe order

- A frame's JPEG is written to `<name>.jpg.tmp`, synced and renamed; only then is it read and its `frame` line written. A JPEG that no `frame` line names, left by a crash, is read and journaled by `--resume` or `finalize`, with `t` from its name.
- A video file is created with fail-if-exists, so a number is never used twice; numbering continues across sessions. It is fragmented MP4: `ftyp` and `moov`, then a `moof` and `mdat` pair per fragment of 1 to 2 s, and `mfra` once the file is finalized. A hard kill leaves a file that plays to its last complete fragment.
- `--resume` and `finalize` truncate the newest video file after its last complete `moof` and `mdat` pair, and write `video_end` with reason `crash` when it has none.
- A clip is written to `<name>.mp4.tmp`, synced and renamed, under a lock per clip path in the app; a cut that runs out of time leaves no file. Its name holds the requested start and end and the video file it was cut from, so the same request finds the same file. The app deletes clips an hour after their last use.

### Video and frames

- Video is H.264 at up to 15 frames per second, from a hardware encoder. A fragment ends at the first keyframe once it holds a second of video, and the sample after that second is always a keyframe. A frame goes to the encoder while the screen changes. One that changed next to nothing since the last frame sent waits, as Windows can hand over unchanged frames at 15 a second: after a quarter second without a sample the newest waiting frame goes in at its capture time, or else the last picture again. So a still screen records about four frames a second, a small change such as typed text shows within half a second, and fragments are 1 to 2 s long. The cursor isn't recorded.
- A video file has the monitor's own size when each side is at most 4096 and the picture at most 36,864 macroblocks of 16 x 16, else it is scaled by one factor to fit both. Both sides are then rounded down to even, so a portrait 1440 x 2560 monitor keeps its size and 5120 x 1440 becomes 4096 x 1152.
- A frame is saved when at least 1% of a 1/8-scale picture changed noticeably since the last saved frame, once the screen is still for 0.7 s or 3 s after the change began, at most one every 2 s, and one every 10 s after 10 saves in a row at that pace. One is always saved at the start of a capture, after a monitor switch, an unlock and a resume.
- Frames are JPEG at quality 0.75 with the longest side at most 2560 pixels. Their text is read on this computer by the platform's text reader, Windows' OCR on Windows and Vision on macOS, in the user's language when it reads that language.

### Readers

- A frame is complete once its `frame` line exists, a video file once its `video_end` line exists, and the journal once its last line is `stop`. Until then, read the journal as a growing file with the torn-line rule above. An open video file can still be truncated by a repair.
- Nothing outside the screen process writes under `screen/`, except the app's clips, `finalize`, and a restore from the user's bucket, which puts a file back at its own path byte for byte (see "Other readers").

### When parts stop

The audio never stops for the screen. The screen process drops the video first and then the frames, each for the rest of the meeting, and writes a `dropped` line for each:

| Cause | Video | Frames |
|---|---|---|
| Less than 4 GiB free on the meeting folder's volume, checked every 5 s and at the start | dropped, `disk` | |
| Less than 2 GiB free | dropped, `disk` | dropped, `disk` |
| The disk is full writing video | dropped, `disk` | |
| The disk is full writing a frame | dropped, `disk` | dropped, `disk` |
| The encoder fails: one retry with a new device and file after 2 s, then | dropped, `encoder` | |
| No hardware encoder, or Media Foundation is missing | dropped, `encoder` | |
| The graphics device is lost and two new ones fail | dropped, `device` | dropped, `device` |
| The graphics device is lost and two new ones come without video support | dropped, `device` | |

When both parts are dropped, the process writes `stop` with reason `dropped` and exits. A lost graphics device is otherwise recreated: the video file ends with reason `error`, `device_lost` is written, and the next `capture` line ends the gap. While the video records on a device with video support, a new device without it, such as the basic display driver Windows runs while the real one restarts, counts as a failed one: the process tries again, and after two such devices it drops the video and records the frames on one. Other writers in the app, such as clip cuts, write only while at least the screen recording's floor plus 2 GiB stays free.

### `rukoo-voice-screen`

```
rukoo-voice-screen record   --meeting <dir> --mode off|frames|video [--resume] [--monitor <id>] [--test <spec>]
rukoo-voice-screen finalize --meeting <dir> [--reason interrupted|failed]
rukoo-voice-screen probe
rukoo-voice-screen inspect <file.mp4>
```

`record` records one monitor of the screen into the meeting.

- It refuses to start while `screen.json` names a process that answers on its port (exit code 4), when the journal has `stop`, and when `screen/` has content and `--resume` isn't given (exit code 3).
- It waits up to 5 s for a `session` line in the timeline, else it exits with code 1.
- It records the monitor of the topmost ordinary window from the foreground window down, skipping hidden, minimized, cloaked, tool, shell and capture-excluded windows such as Rukoo's; else the cursor's monitor; else the primary. With `--monitor`, it records that monitor or exits with code 1; it never records another one. When a pinned monitor goes away mid-meeting, or no monitor is left, capture stops until one is back, with a `no_monitor` line.
- It follows the timeline every 250 ms: it stops capturing at `pause` and starts again at `resume`, and at `stop` it ends at the `stop` line's `t`, finishes the video file, reads the frames still queued, writes `video_end` and `stop` with reason `meeting`, removes `screen.json` and exits with code 0.
- It stops by itself, with reason `interrupted`, when the timeline has had no new line for 60 s of awake time and nobody asked its `status` in that time: the screen is never recorded without the audio.
- It never reads stdin and writes nothing to stdout. Its stderr log never holds frame text, window titles or monitor names. Frames are logged by id, size, bytes and JPEG and OCR time only. A development copy also logs the graphics memory it uses once a minute.

`finalize` finishes a screen journal whose process died: the repair and orphan frames above, then `stop` at `lastSeen` with the given reason, `interrupted` by default, when the journal has none. It removes a stale `screen.json`, refuses a live process with exit code 4, and changes nothing when run again.

`probe` prints what this computer can record:

```json
{ "version": 1, "capture": true, "encoders": [{ "name": "NVIDIA H.264 Encoder MFT", "hardware": true, "vendor": "VEN_10DE" }], "hardwareEncoder": { "name": "NVIDIA H.264 Encoder MFT", "hardware": true, "vendor": "VEN_10DE" }, "ocr": ["nl-NL"], "monitors": [{ "id": "\\\\.\\DISPLAY1", "name": "DELL S3220DGF", "x": 0, "y": 0, "width": 2560, "height": 1440, "primary": true }] }
```

`inspect` prints the fragments, their durations and the keyframes of a video file. It works on every platform; on Windows it also counts the samples Media Foundation reads, and on macOS the shared box walker reads them.

`record`, `finalize` and `probe` work on Windows and macOS; on other platforms they exit with code 2. Until ScreenCaptureKit lands, macOS records the synthetic screen of `--test` and refuses a real one. The exit codes are the recorder's: 0 done, 1 failed, also for a pinned monitor that isn't there, 2 wrong arguments or platform, 3 not allowed in this meeting's state, 4 another screen process records into it.

`--test` is honoured only in a development copy, with `RUKOO_VOICE_DATA_DIR` set. It replaces the screen with a synthetic source and adds faults, as `synthetic:<w>x<h>` followed by options separated by commas:

| Option | Does |
|---|---|
| `change-every=<ms>` | The picture changes this often |
| `text=<word>` | It shows `<word> <n>`, with `n` counting up from 1 at each change |
| `monitors=<n>` | It pretends there are `n` monitors, `\\.\SYNTHETIC1`, `\\.\SYNTHETIC2` and so on, side by side, the first primary |
| `software-encoder` | Microsoft's software encoder counts as a working encoder |
| `no-hardware` | No hardware encoder is found |
| `free-bytes-file=<path>` | The free bytes are read from that file |
| `encoder-fail-at=<ms>` | The encoder fails at that moment |
| `write-fail=video@<ms>`, `write-fail=frames@<ms>` | Writing video or frames fails with a full disk from that moment |
| `device-lost-at=<ms>[x<n>]` | The graphics device is lost then, and the next `n` new devices fail too |
| `no-video-device=<n>` | With `device-lost-at`, the next `n` new devices after the loss have no video support |
| `monitor-gone-at=<ms>` | The first monitor leaves the monitor list then, without the capture's `Closed` notice, as when that notice is lost |
| `warp` | The graphics device is WARP, Microsoft's software rasterizer, as on a CI runner |

### The screen control port

`mode` changes capture with `mode: "off" | "frames" | "video"` and an optional `monitor` id. Monitor selection happens before capture starts. Off releases capture and closes the current video with reason `mode`, without ending the process or permanently dropping a part. `status.state` is `off` while disabled. Capture can be enabled again on the same recorder clock, including in a meeting that began with audio only.

Each change writes a journal line `{ "kind": "mode", "t", "id", "mode", "reason": "user" }`. Its unique id and exact meeting time become a `screen_mode` transcript mark. Readers follow a newly appearing `screen/` folder and never reuse a frame across an Off interval. The command returns `{ "ok": true, "mode": "…" }` after journaling, or `unknown_monitor`, `not_available` or `journal` on failure.

The screen process listens on `127.0.0.1` at the port in `screen.json` and speaks the recorder's line protocol: one JSON object per line, each with the token, several requests per connection, the first within 5 s. It serves at most 8 connections at a time, and a request line is at most 64 KiB.

| `cmd` | Arguments | Reply |
|---|---|---|
| `status` | | `{"ok":true,"state":"recording","t":…,"mode":"video","video":true,"frames":true,"monitor":"\\\\.\\DISPLAY1","monitors":[…],"dropped":{"video":null,"frames":null},"encoder":"NVIDIA H.264 Encoder MFT","hardware":true,"saved":12}`; `state` is `starting`, `recording`, `paused`, `locked` or `stopped`, `video` and `frames` whether each part still records, `dropped` the reason for each part that stopped, `hardware` whether the encoder is a hardware one (`null` without an encoder), and `saved` the frames saved |
| `monitors` | | `{"ok":true,"monitors":[…]}`, left to right, as in `probe`; with `--monitor`, only that monitor |
| `monitor` | `id`, 1 to 64 characters | `{"ok":true,"monitor":"\\\\.\\DISPLAY2"}` once capture moved there; `unknown_monitor` for a monitor that isn't there, and with `--monitor` for every other monitor; `stopped` once recording ended |
| `pause`, `resume` | | `{"ok":true}`. Each first reads the recorder's timeline and acts only when its latest `pause` or `resume` line agrees: `pause` then stops capturing at once, and `resume` starts again. The two can arrive in either order, so a `resume` that arrives after a newer pause records nothing, and a `pause` that arrives after a newer resume stops nothing. The timeline's own lines end or start a pause within 250 ms without them |
| `stop` | | `{"ok":true,"t":…}` once `stop` is written; then the process exits |

Errors are `{"ok":false,"error":"<code>"}`. `bad_token`, and a line that isn't a JSON object (`bad_request`), close the connection; a missing `cmd` or bad arguments (`bad_request`), `unknown_command`, `unknown_monitor` and `stopped` leave it open.

## Other readers

Bucket sync and media links read the meeting folder too, and never write to a file the recorder or the screen recorder writes. Bucket sync reads a segment once its `segment_end` line exists, `timeline.jsonl` once it has `stop`, and `transcript.db` through a snapshot. A media link may also read the open segment of a meeting that still records, up to the samples its header counts, as any other process may (see "Writing, in crash-safe order"). In `screen/`, both read a frame once a `frame` line names it, a video file once its `video_end` line exists, and `journal.jsonl` once it ends with `stop`; `screen/clips/` and `screen/screen.json` never leave this computer. A file restored from the bucket comes back byte for byte at its own path.

## Versioning

This document describes version 1, the `version` in `session` lines, in the screen journal's `session` lines and in `meeting.json`. New line kinds, new fields and new values for `reason` keep the version, because readers ignore what they don't know. A change an older reader would misread, such as a renamed field or a new unit, raises it.
