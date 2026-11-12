# Rukoo Voice

Rukoo Voice is a desktop app for push-to-talk dictation that also records meetings for Rukoo. It will replace [Ownkey for Windows](https://github.com/MajesteitBart/ownkey-windows). Version 0.1 runs on Windows; [What 0.1 leaves out](#what-01-leaves-out) lists the parts of this design that come later.

## What it does

**Dictation.** Hold a key, speak and let go. Voice types the text into whichever app has focus. Hold the rewrite key over selected text to rewrite it with a spoken instruction. Audio goes to the speech provider you configured with your own key, or stays on your computer with a local model.

**Meetings.** Voice records your microphone, the call audio and, if you want, your screen. It transcribes while you talk and streams the transcript to Rukoo. Rukoo shows the meeting next to your agent, so you can ask the agent questions during the call. Afterwards the agent writes the notes with its own meeting-notes skill and stores them where you keep notes.

Voice has no meeting screens of its own. During a meeting the pill shows that it's recording and has a stop button. Meetings need Rukoo. Dictation works without it.

## How Voice and Rukoo work together

```
              commands: start, stop, pause, mark
   Rukoo  ─────────────────────────────────────────►  Rukoo Voice
          ◄─────────────────────────────────────────  mic, call audio, screen
              events: passages, speakers, frames, status
     │
     │  MCP tools: transcript since the last turn, search, notes, frame at time t
     ▼
   Your agent: Hermes, Claude Code, Codex, OpenClaw
```

- Voice sends events to Rukoo over a WebSocket on `127.0.0.1`, protected by a token. Every event has a sequence number. When Rukoo starts late, restarts or loses the connection, it asks for everything after the last number it saw.
- Voice publishes its address and token in a file that only your user account can read. That's how Rukoo finds it.
- You can start and stop a meeting from either app. Voice keeps recording when Rukoo isn't running.
- Voice owns the media: audio tracks, screen frames, video and the transcript as it was recognized. Rukoo owns what you work with: notes, corrections, speaker names, the agent chat and the agent's output. Rukoo uses the calendar event's attendees to turn "Speaker 2" into a name.
- Rename or delete meetings in Rukoo. Renamed titles stay in Rukoo. Deleting a finished meeting permanently removes its local media and owned data and queues a bucket index tombstone, including while offline. Encrypted bucket objects follow the bucket's lifecycle rules. Chats stay with a deleted-meeting label.
- Your agent never talks to Voice. It reaches the meeting through Rukoo's MCP tools. Everyone in the call is a third party, so Rukoo hands the transcript to the agent as untrusted content.
- Voice captures the microphone once, for dictation and the meeting together. If you dictate during a meeting, your words also reach the call. Voice marks those stretches, so the agent knows they weren't part of the conversation.

## Screen recording

Each meeting can record the screen in one of three ways: off, frames only, or the full video. Settings has the default, and Rukoo can pick another one for a meeting it starts.

While recording or paused, the pill's screen button and Rukoo's meeting header offer Off, Frames, Video and a monitor. Off releases capture while audio continues. Capture can start again on the same clock, including after an audio-only start. The transcript marks each mode change. Before enabling capture, Voice waits for every connected Rukoo to confirm its window protection; a missing confirmation leaves the previous mode unchanged. These live controls use bridge protocol 2 and require updated copies of both apps. Updated Rukoo still connects to older Voice with the live controls hidden.

- A process of its own, `rukoo-voice-screen`, records the screen, so a crash in the graphics driver, the encoder or the text reader never costs audio.
- Voice records one monitor at most. It starts with the monitor of the frontmost window, skipping hidden and minimized windows and windows kept out of screen capture, such as Rukoo's. You can switch monitors with the pill's screen button or from the meeting in Rukoo. Recording a single window doesn't work when you share your own screen, because you keep switching windows.
- The agent gets frames, not the video. Each time the screen changes noticeably and then settles, Voice saves a frame at that moment in the transcript, and Windows reads the text on it, on this computer. When the agent needs more, it asks for a clip between two times. A clip is an MP4 file cut from the recording, so only an agent on this computer gets the file; other agents get a handful of frames from that stretch.
- Audio and video share one clock. A passage in the transcript jumps to the same moment in the video. Rukoo doesn't play the video yet.
- Video is encoded on the graphics card, because transcription already uses more than one CPU core during a meeting. Without a hardware encoder, Voice records frames only. The frame rate follows the screen: up to 15 frames a second while it changes, and about 4 a second while it is still, also when Windows keeps handing over unchanged frames. Small changes, such as typed text, reach the video at that slower pace, within half a second.
- Windows draws a border around the monitor being recorded. Voice leaves it on, so you can see what it records.
- While the screen records, Rukoo holds back its mail and booking notifications, so they don't end up in a frame. Calendar reminders still show, and other apps' notifications are recorded.

| | Screen | Call audio | Hardware encoder |
|---|---|---|---|
| Windows | Windows.Graphics.Capture | WASAPI loopback, or one app through process loopback | Media Foundation: NVENC, AMF or Quick Sync |
| macOS | ScreenCaptureKit, after the Screen Recording permission | ScreenCaptureKit audio, or Core Audio process taps | VideoToolbox |
| Linux | xdg-desktop-portal ScreenCast over PipeWire | The PipeWire monitor of the output device | VA-API or NVENC |

Rukoo can hide its window from screen capture while a meeting records, so your agent chat stays out of the recording. On Windows 10 2004 and later, that also keeps it out of Teams and Zoom screen shares. On macOS 15 and later, other apps' screen shares still show Rukoo, although Voice can leave it out of its own recording. Linux has no way to hide a window, so keep Rukoo on a monitor you don't record.

## Speaker labels

During a call, Voice tells the voices on the call apart: Speaker 1, Speaker 2 and so on. Rukoo turns them into names from the calendar event's guests. Voice never learns the names.

- By default the labels come from NVIDIA Nemotron 3 Diarization, which runs on this computer in its own process, `rukoo-voice-speakers`, so the call audio stays here. It needs a processor with AVX2, FMA, F16C and BMI2, the speaker model (107 MB, shared with Ownkey when Ownkey has it) and the runtime that comes with Voice.
- When you transcribe meetings with Mistral's realtime model, Voxtral Mini Transcribe Realtime 3, Mistral labels the voices in the same stream. That happens only after you agreed to stream meeting audio to Mistral at that server. After a dropped connection Voice sends Mistral a short clip of each voice it already knows, so the numbers stay; Mistral bills those clips as audio. Nemotron never hands its work to Mistral: when Nemotron can't run, the call stays one speaker.
- Your microphone is always you. Voice doesn't tell apart people who share your microphone.
- A voice keeps its number for the whole meeting, also when labelling restarts after a crash: Voice plays the new process a few seconds of every voice it already knows and matches them.
- Without labels, or on a computer that can't run them, meetings record and transcribe with the whole call as one speaker. Settings › Meetings › Speaker labels says why.

## Crash safety

A meeting can't be repeated, so losing a recording is the worst thing Voice can do.

- A small recorder process captures and writes to disk, and nothing else. Transcription, speaker labels, dictation, the tray and Rukoo can all crash while the recording goes on. Transcription reads the recorded files and can always start over from them.
- Every file survives a hard kill. Audio goes to disk in short segments, video as fragmented MP4 with a fragment every second or two, and the transcript into SQLite. A crash loses at most the last fragment.
- A watchdog restarts a crashed recorder into the same meeting and marks the gap. On the next start, Voice finishes processing any meeting that was cut off.
- The screen process runs detached like the recorder, with its own watchdog. A crashed screen process restarts into the same meeting, and the transcript marks the gap in the screen.
- When the disk fills up or the video encoder fails, Voice drops the video first, then the frames, and never the audio. Video stops when less than 4 GiB is free and frames below 2 GiB, so the audio of a long meeting still fits. Voice tells you what it dropped.
- Voice follows device changes: an unplugged headset, a Bluetooth headset switching profile, a new default device, sleep and resume. It marks the switch instead of recording silence.

## Storage

Everything records to your computer first. A dropped connection never costs audio.

Sync is optional and off until you turn it on. It goes to an S3-compatible bucket you own, such as UpCloud Managed Object Storage or Cloudflare R2, once Test connection passed for that bucket. Voice encrypts every file on this computer before upload: XChaCha20-Poly1305 in 64 KiB pieces, with a random key per file and object names that show neither meeting ids nor titles. The storage provider can't listen. It still sees how many files there are, how big they are and when they arrive.

The bucket gets each meeting's audio, screen frames and video, `meeting.json`, its timeline and a copy of its transcript. Audio and frames upload as they close during the meeting, at a modest rate; video and the text follow after it. A meeting's audio, frames and video stay on this computer until they are in the bucket, whatever Keep audio says, as long as 10 GB stay free. Below that, Voice removes the oldest media that is waiting to upload, video first, so the meeting being recorded never runs out of room, and Settings says what never reached the bucket. A new secret or a corrected bucket detail pauses uploads until Test connection passes again, and the held media stays meanwhile; only turning sync off lets it go. Moving to another bucket, endpoint or folder uploads everything still on this computer again, and holds its audio until the new bucket has it. Voice doesn't copy between buckets, so what only the old bucket has, such as audio Keep audio already removed here, stays there. The bucket keeps media as long as you choose and the text for good.

Save the encryption key when Settings offers it. With it, a new computer restores your meetings from the bucket; without it, nobody can open them, you included. Voice keeps the key and the bucket's secret with Windows DPAPI, readable only by your Windows account.

An agent never reads the bucket. It asks Rukoo for a link, and the link points at Rukoo on this computer. When the agent fetches it within five minutes, Rukoo asks Voice for the bytes, and Voice reads its own copy or fetches the file back from the bucket and decrypts it. A link opens one meeting's audio, one frame or a stretch of video, only from the address that asked for it, and only while "Agents can read meetings" is on in Rukoo. So this computer must be on to serve a link, and long video leaves over its upload.

## Coming from Ownkey for Windows

On its first start, Voice imports Ownkey's settings and uses the speech models Ownkey already downloaded, so you don't download them again. Ownkey for Windows has no update check, so Voice has an opt-in update check from its first release. Ownkey Keyboard for Android keeps its name and works on its own.

## How it's built

Voice is a Rust workspace with five processes. The recorder, `rukoo-voice-recorder`, captures audio and writes it to disk and does nothing else; [RECORDING.md](RECORDING.md) describes what it writes. The screen process, `rukoo-voice-screen`, records the screen next to the recorder, follows the recorder's clock and never touches the audio. The local speech worker, `rukoo-voice-local`, runs Orukeet, so a crashing model can't take dictation down. The speaker worker, `rukoo-voice-speakers`, runs NeMo-Speech.cpp for speaker labels, so a crash there can't stop recording, transcription or dictation. The app, `rukoo-voice`, is a Tauri shell around everything else: dictation, the pill, Settings, live transcription and the bridge to Rukoo. Its pages are plain HTML, CSS and JavaScript in `ui/`, built from Rukoo's design tokens. Bucket sync runs inside the app and only reads finished files. The local speech worker and the speaker worker end with the app, so a crashed Voice never leaves a model loaded in a process nobody talks to; the recorder does not, because a meeting can't be repeated.

The bridge between Voice and Rukoo is specified in [PROTOCOL.md](PROTOCOL.md), with examples both apps test against in `fixtures/`.

Meeting transcription runs on this computer with Orukeet unless you pick a cloud provider in Settings. A cloud provider gets meeting audio only after you agree to it for that provider and server. Without that agreement, a meeting still records and its transcription waits.

## What 0.1 leaves out

- Recording a single app's audio through process loopback; Voice records everything the computer plays.
- macOS and Linux capture. The recorder, the bridge and the transcript pipeline are tested on Linux, but the app runs on Windows only.
- A signed installer and automatic updates. The opt-in update check tells you a new version exists.

## Open decisions

- In the first version the agent answers only when you ask. Letting it speak up on its own comes later.
- A ready-made line that tells the other participants their words are recorded and processed.
