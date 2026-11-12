# Rukoo Voice UI: commands and events

The pill (`pill.html`) and Settings (`settings.html`) are plain HTML, CSS and JavaScript modules in this folder, with no bundler. They talk to the Rust app (`voice/crates/voice-app`) only through the Tauri commands and events below. The app implements exactly this list. A change to a name or a payload changes this file, the page that uses it and `mock/`, in the same pull request.

## How the pages call the app

- `tauri.conf.json` sets `app.withGlobalTauri: true`. The pages use `window.__TAURI__.core.invoke(command, args)` and `window.__TAURI__.event.listen(name, handler)`; `lib/bridge.js` wraps both.
- `frontendDist` is this folder. The pages load `i18n/en.json` and `i18n/nl.json` with `fetch`, so the asset protocol must serve `.json`. Nothing is fetched from the network; Inter is bundled in `fonts/`. `dev/` and `mock/` are development tools and may be left out of the bundle.
- Arguments are camelCase objects. Tauri maps a top-level argument name to the Rust parameter of the same name in snake_case, so `invoke('pill_ack', { seq })` calls `fn pill_ack(seq: u64)`. Where a command takes one struct, the page passes it under one name (`layout`, `save`, `request`, `preview`), and the Rust parameter has that name. Structs use `#[serde(rename_all = "camelCase")]`.
- A command that fails returns `Err(CommandError)`, which rejects the page's promise with `{ "code": "<code>", "message": "<English text>" }`. The pages show their own text for a known `code` and put `message` behind Error details. Codes: `invalid_value`, `same_key`, `dev_mode`, `need_endpoint_model`, `not_found`, `not_removable`, `busy`, `config`, `auth`, `network`, `timeout`, `provider`, `failed`, and for bucket sync `needs_test`, `endpoint`, `bucket`, `clock`, `permission`, `no_secret`, `key`, `key_in_use`, `recording_screen` and `off`.
- Both windows need `core:event:default` in their capability. App commands registered with `invoke_handler` are allowed for every window unless the app defines command permissions; if it does, allow the `pill_*` commands for the `pill` window and the `settings_*` commands for the `settings` window only.
- Suggested CSP for both windows: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost`. The pages set custom properties in `style` attributes, hence `'unsafe-inline'` for styles only.

Outside Tauri, or with `?mock` in the URL, `mock/pill.js` and `mock/settings.js` answer instead. [Mock pages](#mock-pages) lists the URLs.

---

## The pill

Window `pill`, `pill.html`, 384 × 100 logical pixels, transparent and click-through (pill-settings-design.md 3.2). The pill draws itself 24 px from the window's top, centred, at most 320 px wide and 36 px tall. The page renders snapshots and does nothing else: the Rust state machine decides the phase, the holds and when to hide.

### Event `pill://snapshot`

One complete state per change, never a patch. The page ignores a snapshot whose `seq` isn't newer than the last one it applied, and one that fails its shape check.

```json
{
  "seq": 418,
  "visible": true,
  "phase": "meeting",
  "activity": "dictate",
  "notice": null,
  "words": null,
  "meeting": {
    "state": "recording",
    "atMs": 724000,
    "screen": "frames",
    "screenSwitch": true,
    "compact": false,
    "confirmStop": false,
    "stopEnabled": true,
    "rukooConnected": true
  },
  "edge": "bottom",
  "theme": "dark",
  "locale": "en",
  "dev": false
}
```

| Field | Type | Meaning |
|---|---|---|
| `seq` | integer | +1 per snapshot. The page acks it with `pill_ack` after painting it. |
| `visible` | bool | `false` plays the exit (140 ms) and renders nothing. The app hides the window 180 ms after a hidden snapshot, unless a newer visible one came. |
| `phase` | string | `hidden`, `arming`, `listening`, `transcribing`, `rewriting`, `inserted`, `rewrote`, `nothingHeard`, `warning`, `error`, `meeting` |
| `activity` | string | `dictate` or `rewrite`. While listening, `rewrite` shows "Say the edit". |
| `notice` | object or null | `{ "key": "<catalog key>", "args": {} }`, from the table below. An unknown key is ignored; in the `error` phase it shows "Try again". |
| `words` | integer or null | Words inserted, with `phase: "inserted"`. |
| `meeting` | object or null | Present while a meeting records, is paused or has just ended. |
| `meeting.state` | string | `recording`, `paused`, `processing` (shows "Saving"), `done` (not shown) or `failed` |
| `meeting.atMs` | integer | Meeting time when the app sent the snapshot. The page adds the time since it received the snapshot and ticks every second, also while paused, because the meeting clock keeps running (PROTOCOL.md). Resend every 60 s to correct drift. |
| `meeting.screen` | string | `off`, `frames` or `video`: what the meeting records of the screen. Optional: an app from before screen recording leaves it out. |
| `meeting.screenSwitch` | bool, optional | Draw the screen button left of Stop while live screen controls are available, including when the current mode is Off or there is only one monitor. Missing means `false`. |
| `meeting.compact` | bool | Hide the label and show the time only. The app sets it 4000 ms after the last meeting change, never while paused or while a notice shows. |
| `meeting.confirmStop` | bool | The stop button shows "Stop?" after the first click. The app clears it after 3000 ms. |
| `meeting.stopEnabled` | bool | Draw the stop button. `false` during a dictation in a meeting and in `processing` and `failed`. |
| `meeting.rukooConnected` | bool | Not drawn in 0.1. |
| `edge` | string | `bottom` or `top`: the direction of the entry animation. |
| `theme` | string | `light` or `dark`, resolved from Settings. System follows Windows or macOS, and a state with the new theme arrives when the system changes while Voice runs. |
| `locale` | string | `en` or `nl`, resolved from Settings. |
| `dev` | bool, optional | Shows a small "Dev" tag, so a development pill can't be mistaken for the installed one. Send `true` when `RUKOO_VOICE_DATA_DIR` is set. |

What the page shows:

| `phase` | `notice.key` | Shows |
|---|---|---|
| `arming` | | grey rippling bars, "Starting" |
| `listening` | none | orange bars following `pill://level`, "Listening" or "Say the edit" |
| `listening` | `pill.noAudio` | breathing bars, "No audio detected" |
| `warning` | `pill.selectTextBox` | info icon, "Select a text box" |
| `transcribing` | none or `pill.loadingModel` | bouncing bars, shimmering "Transcribing" or "Loading model" |
| `rewriting` | | bouncing bars, shimmering "Rewriting" |
| `inserted` | | green check, "Inserted {words} words" |
| `rewrote` | | green check, "Rewrote the selection" |
| `nothingHeard` | | no icon, "Nothing heard" |
| `error` | `pill.error.offline`, `.noKey`, `.auth`, `.timeout`, `.mic`, `.model`, `.modelMissing`, `.blocked`, `.selectText`, `.selectionTooLong`, `.generic` | red x on a red tint, the short error. The full text goes to a tray notification. |
| `meeting`, `recording` | none | pulsing dot, "Recording" (or nothing when `compact`), time, the monitor button when `screenSwitch`, stop button |
| `meeting`, `recording` or `paused` | `pill.micLost`, `pill.micBack` | microphone icon in orange or green, the notice, time, stop button |
| `meeting`, `recording` or `paused` | `pill.videoDropped.disk`, `pill.videoDropped.encoder`, `pill.videoDropped.device`, `pill.framesDropped.disk`, `pill.screenFailed` | screen icon in orange, the notice: the video, or the whole screen recording, stopped; the audio goes on. The app holds each for 4000 ms. |
| `meeting`, `recording` or `paused` | `pill.screenMoved` with `args.name` | screen icon in grey, "Screen: {name}": Voice records another monitor now. The app holds it for 2200 ms. |
| `meeting`, `paused` | | pause glyph, "Paused", grey time, stop button |
| `meeting`, `processing` | | spinner, "Saving" |
| `meeting`, `failed` | `pill.meetingFailed` | red x on a red tint, "Couldn't save the meeting" |

`ProviderError` kinds map to error notices like this: `auth` → `pill.error.auth`, `network` → `pill.error.offline`, `timeout` → `pill.error.timeout`, `config` for a missing key → `pill.error.noKey`, `local_missing_model` → `pill.error.modelMissing`, anything else → `pill.error.generic`. Decision 7's `SendInput` shortfall → `pill.error.blocked`.

### Event `pill://level`

`{ "level": 0.42 }`, a number from 0 to 1 after Ownkey's level pipeline (pill-settings-design.md 1.9). At most every 20 ms, only while `phase` is `listening`. It stays out of the snapshot so a level tick never re-renders the pill.

### Event `pill://pointer`

`{ "hover": true, "pressed": false, "target": "stop" }`, from a native button window: hover on `WM_MOUSEMOVE`, pressed between `WM_LBUTTONDOWN` and `WM_LBUTTONUP`, both false on `WM_MOUSELEAVE`. `target` is `stop` for the stop window or `screen` for the monitor window; without it the page takes `stop`. The page only colours the button `target` names; Rust handles the click, and a click on the screen window opens the native mode and monitor menu (`EngineApi::pill_screen_clicked`).

### Command `pill_snapshot`

`invoke('pill_snapshot')` → the current snapshot. The page calls it on load and then every 1000 ms as a safety net, and applies it only when `seq` is newer.

### Command `pill_ack`

`invoke('pill_ack', { seq })` → nothing. Called two animation frames after the page applied snapshot `seq`, so the frame with it was painted. The app's watchdog nudges the window when no ack arrives within 500 ms of a show and rebuilds it after 2 s (pill-settings-design.md 3.2, step 5).

### Command `pill_layout`

`invoke('pill_layout', { layout })` → nothing. Where the pill, the stop button and the monitor button are, so the app can put its native windows exactly over the buttons.

```json
{ "seq": 418, "pill": { "x": 80, "y": 24, "width": 225, "height": 36 }, "button": { "x": 273, "y": 28, "width": 28, "height": 28 }, "screenButton": { "x": 241, "y": 28, "width": 28, "height": 28 } }
```

- Rectangles are in CSS pixels from the window's top left, rounded outwards to whole pixels. At the page's 100% zoom a CSS pixel is a logical pixel; multiply by the monitor's scale for physical pixels.
- `button` is `null` when no stop button is drawn: no meeting, `stopEnabled` false, the pill hidden or leaving. Hide the stop window then.
- `screenButton` is `null` when no monitor button is drawn: `screenSwitch` false or missing, no meeting that records or is paused, the pill hidden or leaving. Hide the monitor window then. An app from before the monitor button ignores it.
- `pill` is `null` while nothing is drawn.
- `seq` is the snapshot the layout was measured for. Ignore a layout whose `seq` is older than the newest snapshot that has a stop button.
- The page reports after every change: a new snapshot, the end of the entry animation, every frame of a width transition (the label sliding to 0 when the pill compacts, the button widening to "Stop?") and when Inter finishes loading. It never sends the same layout twice in a row.

### Strings the app reads from the catalogs

The app has text of its own and reads it from `i18n/en.json` (the `message` field) and `i18n/nl.json`, in the language Settings resolves:

- `pill.stopButton`: the title of the native stop window, which screen readers announce.
- `app.pill.screenOff`, `app.pill.screenFrames`, `app.pill.screenVideo`, `app.pill.screenMonitor`: labels in the pill's native screen menu. It offers Off, Frames, Video and the available monitors. Opening it calls `EngineApi::pill_screen_clicked`; selecting calls `pill_screen_set` with the meeting id so a stale menu cannot change the next meeting.
- `pill.screenButton`: the title of the native monitor window, which screen readers announce.
- `settings.title`: the Settings window's title.
- `settings.model.folder`, `settings.storage.folder`: the title of the folder picker that `settings_choose_folder` opens for `models` and `recordings`. Settings 0.1 asks only for `models` (see the command below).
- `app.tray.tooltip`, `app.tray.tooltipRecording`, `app.tray.settings`, `app.tray.startMeeting`, `app.tray.pauseMeeting`, `app.tray.resumeMeeting`, `app.tray.stopMeeting`, `app.tray.quit`: the tray.
- `app.notify.dictationFailed`, `app.notify.rewriteFailed`, `app.notify.noTextBox`, `app.notify.meeting`, `app.notify.settings`, `app.notify.welcome`, `app.notify.sync`: the titles of the engine's tray notifications (`Notification.title_key`). The body comes from the engine as it is.

`app.notify.sync`, "Bucket sync", goes out once a day while uploads have failed for more than a day, with the body "Voice couldn't upload to your bucket for a day. Your audio stays on this computer until it can, as long as 10 GB stay free." When held media had to go because less than 10 GiB were free, the body is "The disk was almost full, so Voice removed meeting media that hadn't reached your bucket yet. Settings > Storage lists how much.", or, with an unreadable upload queue, "The disk was almost full, so Voice removed meeting media it couldn't check against your bucket." The engine writes these bodies as it writes the other meeting notifications.

`test/voice-ui-i18n.test.js` checks that every key in this list exists in both catalogs.

---

## Settings

Window `settings`, `settings.html`, 760 × 680, minimum 480 × 480, native decorations. The app sets the window's background to the theme's `--bg` (`#141519` dark, `#f3f4f6` light) before the page loads, so it doesn't flash white.

The page can open at a page or a group: `settings.html?page=<page>`, where `<page>` is `transcription`, `dictionary`, `fillers`, `rewriting`, `meetingTranscription`, `speakers`, `storage`, `rukoo` or `import`, or one of the top page's groups `status`, `dictation`, `meetings`, `general`, `about`. The app can use it to open Settings at the import after the first start.

### Command `settings_state`

`invoke('settings_state')` → `SettingsState`, the whole state. The page calls it once on load.

### Event `settings://state`

A whole `SettingsState` with a higher `seq`, sent whenever anything in it changes: a saved setting, a model's download progress (at most 10 per second), the bridge status, the update check, the Ownkey import, a resolved theme or language. The page redraws only the cards whose content changed, so text the user is typing is never reset.

### `SettingsState`

```json
{
  "seq": 57,
  "version": "0.1.0",
  "devMode": false,
  "platform": "windows",
  "features": { "screen": true, "speakers": true, "sync": true },
  "theme": "dark",
  "locale": "en",
  "config": {
    "dictation": { "hotkey": "right alt", "rewriteHotkey": "right ctrl", "pasteMode": true, "readyChime": false, "startAtLogin": true },
    "transcription": { "provider": "orukeet", "endpoint": "", "model": "orukeet-onnx-int8", "language": "auto", "keySaved": false },
    "local": { "modelFolder": "", "idleMinutes": 20 },
    "dictionary": { "vocabulary": ["Rukoo"], "corrections": [{ "from": "rocco", "to": "Rukoo" }] },
    "fillers": { "enabled": true, "languages": ["en", "nl"], "custom": "" },
    "rewriting": { "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/chat/completions", "model": "mistral-small-latest", "keySaved": true, "auto": false, "tone": "auto", "formatting": true, "instructions": "" },
    "meetings": { "provider": "orukeet", "endpoint": "", "model": "orukeet-onnx-int8", "keySaved": false, "consent": null, "retention": "days7", "stopKey": "off", "screen": "frames", "liveSpeakers": true },
    "ui": { "language": "system", "theme": "system", "pillEdge": "bottom", "hidePillFromCapture": true },
    "updates": { "check": false },
    "storage": { "sync": { "enabled": false, "service": "upcloud", "endpoint": "", "region": "", "bucket": "", "prefix": "", "accessKeyId": "", "retention": "keep", "secretSaved": false, "tested": false, "secret": "" } }
  },
  "providers": {
    "audio": [
      { "id": "openai", "endpoints": ["https://api.openai.com/v1/audio/transcriptions"], "streamingPrefix": null },
      { "id": "mistral", "endpoints": ["https://api.mistral.ai/v1/audio/transcriptions"], "streamingPrefix": "voxtral-mini-transcribe-realtime" }
    ],
    "rewrite": [{ "id": "openai", "endpoints": ["https://api.openai.com/v1/responses", "https://api.openai.com/v1/chat/completions"] }]
  },
  "dictation": { "status": "ready" },
  "rukoo": { "state": "connected", "clientVersion": "2.2.0", "protocol": 1, "port": 49821, "discoveryPath": "C:\\Users\\…\\Rukoo Voice\\bridge.json", "behind": 0 },
  "models": { "orukeet": "ModelState", "nemotron": "ModelState" },
  "speakers": "SpeakersState",
  "storage": { "folder": "C:\\Users\\…\\Rukoo Voice\\meetings", "usedBytes": 2400000000, "freeBytes": 182000000000 },
  "screen": { "available": true, "hardwareEncoder": "NVIDIA H.264 Encoder MFT", "ocr": ["nl-NL"], "monitors": 2 },
  "updates": { "state": "idle", "version": null },
  "ownkey": "OwnkeyState",
  "sync": {
    "available": true,
    "state": "off",
    "waiting": 0,
    "waitingBytes": 0,
    "oldestWaiting": null,
    "lastUpload": null,
    "lastError": null,
    "failingSince": null,
    "lost": 0,
    "screenRecording": false,
    "key": { "exists": false, "id": null, "exported": false },
    "restore": { "state": "idle", "available": 0, "done": 0, "total": 0, "error": null },
    "otherKeys": 0
  }
}
```

| Field | Meaning |
|---|---|
| `seq` | +1 per state, and the app sends states in `seq` order, each built after its number was given. The page drops a state that isn't newer than the one it has. |
| `version` | Voice's version, shown under About. |
| `devMode` | `RUKOO_VOICE_DATA_DIR` is set. The page disables Start at login and the import button, with a note, and key lists say development copies use F13 and F14. |
| `platform` | `windows`, `macos` or `linux`. |
| `features` | Which features' Settings this Voice shows: `screen`, `speakers` and `sync`. A feature not in this Voice is `false`. |
| `theme`, `locale` | `ui.theme` and `ui.language` resolved: `light` or `dark`, `en` or `nl`. The page follows these, never the raw setting. |
| `config` | Every setting, see below. API keys appear only as `keySaved` flags and the bucket secret only as `storage.sync.secretSaved`; `storage.sync.secret` is always `""`. |
| `providers` | Voice-core's provider registry in Ownkey's order (ownkey-providers.md 2): `audio` is OpenAI, Google, Mistral, Custom, Orukeet; `rewrite` is OpenAI, Anthropic, Google, Mistral, Ollama, OpenRouter, Custom. `endpoints` are the presets, the first is the default. The page translates the labels of `custom` and `orukeet` and shows the brand names for the rest. |
| `providers.audio[].streamingPrefix` | For Mistral, `voxtral-mini-transcribe-realtime`: a meeting model whose name starts with it streams the meeting instead of sending recorded windows, so its consent is a `stream` agreement. `null` for the other providers, which never stream. The name still marks a realtime model for every provider: `settings_save_provider` refuses it for dictation and rewriting whichever provider lists it, as an OpenAI-compatible proxy in front of Mistral can, and those two forms don't offer it. |
| `dictation.status` | `ready`, `loadingModel`, `modelMissing`, `noKey` or `noMic`: the status on the top page. |
| `rukoo.state` | `connected`, `notRunning`, `rukooOutdated` or `voiceOutdated` (from `unsupported_protocol`, PROTOCOL.md Versioning), `refused` (a client sent a wrong token). A refusal shows until a client connects again. |
| `rukoo.clientVersion`, `rukoo.protocol` | From the connected client's `hello`, or `null`. |
| `rukoo.port` | The bridge's port on 127.0.0.1, or `null` before it listens. |
| `rukoo.discoveryPath` | The discovery file's full path (PROTOCOL.md, Where the file is). |
| `rukoo.behind` | The event log's head minus the last event the connected client has (its hello's `after`, then what the bridge sent it), or `null`. |
| `models` | A `ModelState` for `orukeet` and one for `nemotron`, the speaker label model. |
| `speakers` | A `SpeakersState`, below: whether new meetings get speaker labels, and why not. |
| `storage` | The recordings folder, the bytes meetings use and the free bytes on its drive. In 0.1 the folder is `meetings` in Voice's data folder and can't be moved. |
| `screen` | What this computer can record of the screen, from the screen process's probe, or `null` until it answered. `available`: Windows can capture the screen (Windows 10 2004 or later); `false` disables Screen recording. `hardwareEncoder`: the hardware H.264 encoder's name, or `null` when there is none and Video records frames only. `ocr`: the text recognition languages Windows has, such as `nl-NL`; an empty list means frames are saved without their text. `monitors`: how many monitors there are. The page shows a note under Screen recording for each limit. |
| `updates.state` | `idle`, `checking`, `available` (with `version`), `upToDate` or `failed`. |
| `ownkey` | An `OwnkeyState`, below. |
| `sync.available` | Bucket sync works here: Windows, with the sync store open. The Storage page shows the sync card only then. When the store didn't open (locked, damaged or from a newer Voice) and `storage.sync.enabled` is on, `state` is `paused`, `lastError` says why, and the page shows only the switch, with a line that nothing uploads and that retention keeps the audio while the switch is on, so the user can turn it off. |
| `sync.state` | `off`; `idle`, everything is in the bucket; `uploading`; `live`, a meeting records and its audio and frames go up as they close; `retrying`, the bucket can't be reached and Voice tries again; `failing`, the bucket refused (see `lastError`) and nothing uploads until the settings change, Test connection passes or 15 minutes pass; `key`, Voice can't open its encryption key; `paused`, the switch is on but a connection field or the secret changed since the last passed Test connection, so nothing uploads or comes back from the bucket until a test passes, and retention keeps holding what hasn't uploaded. |
| `sync.waiting`, `waitingBytes`, `oldestWaiting` | Files waiting to upload, their size, and when the oldest was queued (epoch ms or `null`). |
| `sync.lastUpload` | When the last file reached the bucket, or `null`. |
| `sync.lastError` | `{ "at", "code", "message" }` of the last failure, `null` after a success. `code` is one of the Test connection codes; `message` is English, for Error details. |
| `sync.failingSince` | When uploads started failing without a success since, or `null`. |
| `sync.lost` | Files Voice removed from this computer before they reached the bucket, because the disk was almost full (under 10 GiB free). |
| `sync.screenRecording` | A meeting records the screen now. The page closes the key dialogs when it turns true. |
| `sync.key` | `exists`: Voice has an encryption key; `id`: its first 8 hex characters, to tell keys apart; `exported`: the user said they saved it. Never the key. |
| `sync.restore` | `state`: `idle`, `listing`, `available`, `restoring`, `done` or `failed`; `available`: meetings in the bucket that aren't on this computer; `done` and `total` while restoring; `error`: English text after a failure. |
| `sync.otherKeys` | How many other encryption keys have meetings in this bucket, from the last Test connection. |

`config`, by the key `settings_set` takes for each field:

| Key | Values | Default | From Ownkey's |
|---|---|---|---|
| `dictation.hotkey` | `right alt`, `right ctrl`, `right shift`, `f13`, `f14`, `f15`, `pause`, `scroll lock` | `right alt`; `f13` in dev mode | `hotkey` |
| `dictation.rewriteHotkey` | `off` or a key above | `right ctrl`; `f14` in dev mode | `rewrite_hotkey` |
| `dictation.pasteMode` | bool | `true` | `paste_mode` |
| `dictation.readyChime` | bool | `false` | `ready_chime` |
| `dictation.startAtLogin` | bool; `dev_mode` error in dev mode | `true` for an installed copy | the Run key |
| `ui.pillEdge` | `bottom`, `top`: where the pill appears | `bottom` | new |
| `ui.hidePillFromCapture` | bool: excludes the pill and its stop window from screen capture | `true` | new |
| `transcription.*` | read-only here; saved with `settings_save_provider` | Orukeet | `audio_*`, `language` |
| `local.modelFolder` | a path, or `""` for automatic: Ownkey's folder first, then Voice's own | `""` | `local_model_directory` |
| `local.idleMinutes` | integer 0 to 1440; 0 never unloads | `20` | `local_model_idle_timeout_minutes` |
| `dictionary.vocabulary` | the whole list of words | `[]` | `vocabulary` |
| `dictionary.corrections` | the whole list of `{ "from", "to" }` | `[]` | `corrections` |
| `fillers.enabled` | bool | `true` | `remove_fillers` |
| `fillers.languages` | subset of `en`, `nl`, `de`, `fr`, `es`; an empty list stays empty | `["en", "nl"]` | `filler_languages` |
| `fillers.custom` | comma-separated words | `""` | `custom_fillers` |
| `rewriting.provider`, `endpoint`, `model`, `keySaved` | read-only here; saved with `settings_save_provider` | Mistral | `rewrite_*` |
| `rewriting.auto` | bool | `false` | `auto_rewrite` |
| `rewriting.tone` | `auto`, `professional`, `casual`, `friendly`, `concise` | `auto` | `rewrite_tone` |
| `rewriting.formatting` | bool | `true` | `rewrite_formatting` |
| `rewriting.instructions` | string | `""` | `rewrite_custom_instructions` |
| `meetings.provider`, `endpoint`, `model`, `keySaved`, `consent` | read-only here; saved with `settings_save_provider` | Orukeet | `meetings_audio_*` |
| `meetings.retention` | `days7`, `keep`, `after_transcription` | `days7` | `meetings_retention` |
| `meetings.stopKey` | `off` or a key | `off` | new |
| `meetings.screen` | `off`, `frames`, `video`: what a new meeting records of the screen when Rukoo doesn't choose for it. Voice tells Rukoo in `status.screenDefault` (PROTOCOL.md). | `off` | new |
| `meetings.liveSpeakers` | bool, default `true`, stored as `meetings_live_speakers`: speaker labels for meetings started from then on | `true` | new; Ownkey's `meetings_auto_speakers` ("label after Stop") is kept but not read |
| `ui.language` | `system`, `en`, `nl` | `system` | new |
| `ui.theme` | `system`, `light`, `dark` | `system` | new |
| `updates.check` | bool | `false` | new |

Bucket sync, saved in `config.json` under `storage_sync`:

| Key | Values | Default | `config.json` |
|---|---|---|---|
| `storage.sync.enabled` | bool; `true` is refused with `needs_test` unless the current fields and secret passed Test connection. Only the user turns it off | `false` | `storage_sync.enabled` |
| `storage.sync.service` | `upcloud`, `r2`, `other` | `upcloud` | `storage_sync.service` |
| `storage.sync.endpoint` | `https://…`, or `http://` to this computer for tests; `""` | `""` | `storage_sync.endpoint` |
| `storage.sync.region`, `bucket`, `prefix`, `accessKeyId` | string | `""` | `storage_sync.<field>` |
| `storage.sync.retention` | `keep`, `days365`, `days90`: how long the bucket keeps a meeting's media | `keep` | `storage_sync.retention` |
| `storage.sync.secret` | refused with `invalid_value`; use `settings_sync_save_secret` | | never stored here |

Changing `service`, `endpoint`, `region`, `bucket`, `prefix` or `accessKeyId`, or saving or removing the secret, clears the passed test. The switch stays where the user put it: while it is on, `sync.state` is `paused` and nothing uploads until Test connection passes again, then uploads go on by themselves. Retention keeps holding media that hasn't uploaded meanwhile, so repairing a failing bucket never costs that media; only turning the switch off lets it go. Once a test passes for another endpoint, bucket or folder, everything still on this computer uploads again, `waiting` counts it, and retention holds its media until the new bucket has it. `secretSaved` and `tested` are read-only.

`meetings.retention` applies to meetings started from then on, as in Ownkey: each meeting keeps the choice it started with in its `meeting.json`, so changing it, or importing Ownkey's, never removes older meetings' audio. Meetings from before Voice stored it follow the current choice.

The provider objects carry `provider`, `endpoint`, `model` and `keySaved`. `transcription` adds `language` (`auto`, `en`, `nl`, `de`, `fr`, `es`, `it`, `pt`, `pl`, `ja`, `zh`). `meetings` adds `consent`: `{ "provider": "mistral", "host": "api.mistral.ai", "mode": "windows" }` or `null`. `mode` is `windows` for a model that gets recorded windows and `stream` for a realtime model (`providers.audio[].streamingPrefix`); one doesn't cover the other, so changing between them asks again. An agreement without `mode`, saved before streaming existed, is `windows`.

Meeting transcription has no "Same as dictation" choice. Decision 10 makes it Orukeet (the default) or a cloud provider with consent per provider and host, and a "same" choice would move meeting audio to the cloud whenever someone changed the dictation provider. The import resolves Ownkey's `same` to the provider it pointed at.

`ModelState`:

```json
{
  "id": "orukeet", "name": "Orukeet", "sizeBytes": 487000000,
  "stage": "installed", "percent": null,
  "location": "ownkey", "path": "C:\\Users\\…\\Ownkey\\models\\orukeet-onnx-int8",
  "removable": false, "active": "loaded", "error": null
}
```

| Field | Meaning |
|---|---|
| `stage` | `missing`, `downloading`, `verifying`, `extracting`, `installed`, `failed` (the download or its check failed), `unavailable` (installed, but the runtime can't use it) |
| `percent` | 0 to 100 during a stage that knows it, else `null` |
| `location` | `ownkey` (Ownkey's shared folder), `voice` (Voice's own `models/`), `folder` (the folder chosen in Settings), or `null` while the files aren't there |
| `removable` | `true` only for files in Voice's own `models/`. Never for Ownkey's folder (decision 13); the page says "Shared with Ownkey. Remove it in Ownkey." instead. |
| `active` | When this model is the chosen provider: `unloaded`, `loading`, `loaded` or `error`; otherwise `null` |
| `error` | The last download or load error in English, or `null`. The page cuts it at 180 characters and shows all of it behind Error details. |

`OwnkeyState`:

```json
{
  "running": false, "keys": ["right alt", "right ctrl"],
  "configFound": true, "configPath": "%APPDATA%\\Ownkey\\config.json",
  "lastImport": 1793000000000,
  "models": [{ "id": "orukeet", "name": "Orukeet", "sizeBytes": 487000000 }],
  "report": {
    "hotkeys": { "dictation": "right alt", "rewrite": "right ctrl" },
    "transcription": { "provider": "orukeet", "model": "orukeet-onnx-int8", "keyImported": false },
    "rewriting": { "provider": "mistral", "model": "mistral-small-latest", "keyImported": true },
    "dictionary": { "words": 12, "corrections": 3 },
    "fillers": { "enabled": true, "languages": ["en", "nl"] },
    "meetings": { "provider": "orukeet", "keyImported": false, "retention": "days7", "consent": false },
    "skipped": ["summaries"]
  }
}
```

- `running`: an Ownkey process runs. Reading the process list is fine; Voice never stops it. `keys`: the dictation and rewrite keys in Ownkey's config, read-only. The top page warns when they overlap Voice's keys.
- `configPath` is shown as it is, so use the `%APPDATA%` form rather than the user's real path.
- `lastImport` is epoch ms, or `null`.
- `models` are the models found in Ownkey's model folder.
- `report` is the last import's result, or `null` before any import. Each area is `null` when Ownkey's config didn't have it. `keyImported` says a key came over; the key never reaches the page. `meetings.consent` is `true` only when Ownkey had `meetings_live_transcription_policy == "allow"` for a cloud provider (decision 10). `skipped` lists settings Voice doesn't take over: `summaries` covers `meetings_auto_summary` and `meetings_remote_policy`.

`SpeakersState`:

```json
{
  "engine": "nemotron",
  "ready": true,
  "reason": null,
  "detail": null,
  "missing": [],
  "cpu": { "avx2": true, "fma": true, "f16c": true, "bmi2": true },
  "runtime": { "location": "voice", "commit": "97a15af" },
  "live": [{ "meeting": "m-20261102-101500-1a2b", "mode": "nemotron", "state": "listening", "speakers": 3, "reason": null, "workerPid": 4242 }]
}
```

| Field | Meaning |
|---|---|
| `engine` | Who labels new meetings: `mistral` when the meeting model is a realtime model (it labels in the same stream it transcribes), else `nemotron` on this computer. Nemotron never stands in for Mistral or the other way round. |
| `ready` | New meetings get speaker labels. Otherwise the whole call is one speaker, `call-1`. |
| `reason` | Why not, or `null`: `off` (the switch), `platform` (not Windows on x64), `cpu` (the processor lacks AVX2, FMA, F16C or BMI2), `runtime` (no NeMo runtime verifies next to Voice), `runtimeFailed` (a speaker worker failed to load since Voice started; it tries again after a restart), `model` (the Nemotron model isn't there), `modelDamaged` (the model failed its check since Voice started), `key` (a realtime model without a meeting API key: the stream can't start, and the page points to Meeting transcription; it comes before `consent`), `consent` (a realtime model without a `stream` agreement for its provider and host). |
| `detail` | The English error behind `runtimeFailed` and `modelDamaged`, for Error details; else `null`. |
| `missing` | With `cpu`, the missing features in lower case, such as `["bmi2"]`; else `[]`. |
| `cpu` | This processor's features. |
| `runtime` | The NeMo runtime Nemotron would use, while it can run: `location` `voice` (next to Voice), `ownkey` (Ownkey's copy, read only) or `dev` (`RUKOO_VOICE_NEMO_DIR` in a development copy), and `commit`, the NeMo-Speech.cpp commit's first seven characters. `null` otherwise. |
| `live` | One entry per meeting that is transcribing: `mode` `nemotron`, `mistral` or `off`; `state` `off`, `starting`, `matching`, `listening`, `catchingUp`, `restarting`, `finished` or `failed`; `speakers`, the voices labelled so far; `reason`, the code a meeting in mode `off` labels without (the codes above, `stream`, `noCall` for a meeting that records no call track, or `upgrade` for a meeting that a Voice without speaker labels started); `workerPid`, the worker's process id in a development copy, else `null`. `failed` means labels stopped for that meeting while its recording and transcription go on. |

A meeting keeps the labels it started with: a change of the switch, the model or the meeting provider applies to the next meeting.

### Command `settings_set`

`invoke('settings_set', { key, value })` → nothing. Saves one value from the table above, applies it at once and emits `settings://state`. A key change restarts the hotkey poller; `ui.language` and `ui.theme` re-resolve the language and theme for Settings and the pill at once; `ui.pillEdge` and `ui.hidePillFromCapture` take effect at the pill's next show; `startAtLogin` writes or removes the Run value.

Errors: `same_key` when the key is already the dictation, rewrite or stop key (the page greys those out, the app checks again), `dev_mode` for `startAtLogin` in dev mode, `invalid_value` for a value outside the table.

### Command `settings_save_provider`

`invoke('settings_save_provider', { save })` → nothing, once saved. One provider form, checked and saved as a whole, like Ownkey's Save.

```json
{ "target": "meetings", "provider": "mistral", "endpoint": "https://api.mistral.ai/v1/audio/transcriptions", "model": "voxtral-mini-latest", "apiKey": null, "consent": true }
```

| Field | Meaning |
|---|---|
| `target` | `transcription`, `rewriting` or `meetings` |
| `provider` | An id from `providers.audio` (transcription, meetings) or `providers.rewrite` (rewriting) |
| `endpoint`, `model` | Trimmed. For `orukeet`: `""` and `orukeet-onnx-int8`. |
| `apiKey` | `null` keeps the saved key, `""` saves none, a string replaces it. A key is never carried to another provider: when `provider` differs from the saved one, `null` means no key. |
| `language` | `transcription` only; `auto` for Orukeet. |
| `consent` | `meetings` only. `true` saves `consent: { provider, host, mode }` with the host of `endpoint` and the mode of `model`; `false` clears it. |

The app repeats Ownkey's checks (ownkey-dictation.md 13.8) and rejects with `need_endpoint_model` when a cloud provider lacks an endpoint or a model. A realtime model is for meetings only: `transcription` and `rewriting` reject a model whose name starts with a `streamingPrefix`, from any provider, with `invalid_value`, "Realtime models work for meeting transcription only." For `transcription` with Orukeet it loads and checks the model before it commits, so a broken model never becomes active; the page shows "Loading Orukeet" until the promise settles. A failure keeps the old settings and rejects with the reason in `message`.

Consent: the page shows the disclosure and its checkbox when the provider isn't Orukeet and the endpoint's host isn't `localhost`, `127.0.0.1`, `::1` or `0.0.0.0`, which is Ownkey's rule for when to ask. For a realtime model the disclosure says the meeting streams and that clips of up to 15 s per voice go out after a reconnect, and the checkbox reads "Stream meeting audio to …". The app uses the same rule to decide whether a meeting's transcription may start: valid consent names the saved provider, the host of the saved endpoint and the model's mode. Without it, the meeting records and its transcription, and with a realtime model its labels, wait.

### Command `settings_remove_key`

`invoke('settings_remove_key', { target })` → nothing. Deletes the saved key of `transcription`, `rewriting` or `meetings`, or the bucket's secret for `sync`, at once. Removing the bucket's secret pauses uploads until Test connection passes again; the switch stays on.

### Command `settings_list_models`

`invoke('settings_list_models', { request })` → `["mistral-small-latest", …]`, sorted, without duplicates (ownkey-providers.md 6).

`request` is `{ "target", "provider", "endpoint", "apiKey" }`. `apiKey: null` uses the saved key, but only when `provider` is the saved provider of `target`. Rejects with `auth`, `network`, `timeout`, `provider` or `config`. The page drops a result that arrives after the user changed the provider, endpoint or key.

### Command `settings_model`

`invoke('settings_model', { model, action })` → nothing; progress arrives through `settings://state`.

- `model`: `orukeet` or `nemotron`.
- `action`: `download` (Orukeet into the chosen model folder or Voice's own `models/`, Nemotron into Voice's own `models/`), `cancel`, `remove`. `remove` rejects with `not_removable` unless the model's `removable` is true, and for `nemotron` with `busy`, "A meeting is labelling speakers with this model.", while a `speakers.live` entry in mode `nemotron` isn't `finished`, `failed` or `off`.

### Command `settings_choose_folder`

`invoke('settings_choose_folder', { purpose })` → the chosen path, or `null` when the user cancelled. Opens the system's folder picker and saves the choice itself: `models` sets `local.modelFolder` (Ownkey's dialog title was "Choose model download or existing model folder"). `recordings` comes later than 0.1: Voice 0.1 keeps meetings in its data folder (`storage.folder`) and rejects it with `invalid_value`, so Settings > Storage offers Open folder there, not Browse.

### Command `settings_open_folder`

`invoke('settings_open_folder', { folder })` → nothing. Opens Explorer at `logs`, `bridge` (the discovery file's folder), `recordings` or `models`.

### Command `settings_open_link`

`invoke('settings_open_link', { link })` → nothing. Opens a fixed address in the browser. The page never sends a URL; the app keeps the table: `orukeet` → `https://huggingface.co/oruk/orukeet`, `nemotron` → `https://huggingface.co/nvidia/Nemotron-3-Diarization`.

### Command `settings_clean_preview`

`invoke('settings_clean_preview', { preview })` → the cleaned text. `preview` is `{ "text", "enabled", "languages", "custom" }`. The app runs `voice_core::cleanup` with those values without saving them, so the Filler words preview shows the real cleanup.

### Command `settings_check_updates`

`invoke('settings_check_updates')` → the new `updates` object, `{ "state", "version" }`. Checks now, also when `updates.check` is off, because the user asked. Reads only `voice-v` tags.

### Command `settings_new_token`

`invoke('settings_new_token')` → nothing. Makes a new bridge token, rewrites the discovery file and closes the connected clients, which reconnect with the new file (PROTOCOL.md, Connecting).

### Command `settings_import_ownkey`

`invoke('settings_import_ownkey')` → the new `report`. Reads `%APPDATA%\Ownkey\config.json` and never writes to Ownkey's folders. Rejects with `dev_mode` in dev mode, where the import runs only with `--import-ownkey <copy>`, and with `not_found` when Ownkey's config is missing. The page asks before it imports over an earlier import.

### Command `settings_sync_test`

`invoke('settings_sync_test')` → `{ "otherKeys": 0 }` when Voice can write, read, list and delete in the bucket of `config.storage.sync` with the saved secret, and start and abort a multipart upload. It marks the fields as tested. Rejects with `endpoint`, `network`, `auth`, `bucket`, `clock`, `permission`, `no_secret` or `failed`, with an English `message` for Error details.

### Command `settings_sync_save_secret`

`invoke('settings_sync_save_secret', { secret })` → nothing. Keeps the bucket's secret access key with Windows DPAPI for this Windows user, never in `config.json`, and pauses uploads until Test connection passes again; the switch stays on. Rejects with `invalid_value` for an empty secret or one over 1,024 characters, `failed` when Windows can't protect it or `config.json` can't be saved. The passed test leaves `config.json` before the secret changes, so a failed save keeps the secret that was tested. `settings_remove_key({ target: "sync" })` removes it. The page calls it when the secret's field loses focus and keeps no copy.

### Command `settings_sync_key`

`invoke('settings_sync_key', { action, code })`:

- `show` → `{ "code": "RKV1-…", "id": "0f1e2d3c" }`. Makes the key when there is none. Rejects with `recording_screen` while a meeting records the screen, and with `key` when Voice can't open its encryption key on this computer, for instance after a Windows password reset.
- `saved` → nothing: the user saved the code; `sync.key.exported` becomes `true`. Rejects with `key` when there is no key, or Voice can't open it.
- `import` with `code` → `{ "id" }`: this computer uses that key from now on. Rejects with `invalid_value` for a code that isn't a key, `key_in_use` once this computer uploaded meetings with its own key, and `recording_screen` while a meeting records the screen.

The page shows the code only in its dialog, with the window hidden from capture (`settings_protect`), and closes the dialog when `sync.screenRecording` turns true.

### Command `settings_sync_key_copy`

`invoke('settings_sync_key_copy')` → nothing. Puts the recovery code on the clipboard, hidden from clipboard history and cloud clipboard, and clears it after 60 seconds if it is still there. Rejects with `key` when Voice can't open its encryption key on this computer. The page never gets the code from this command and never uses `navigator.clipboard`.

### Command `settings_protect`

`invoke('settings_protect', { on })` → nothing. `true` hides the Settings window from screen capture and screen sharing, as the pill is hidden; `false` shows it again. The Storage page uses it while a key dialog is open. Closing the window resets it.

### Command `settings_sync_restore`

`invoke('settings_sync_restore', { action })`:

- `list` → `{ "meetings": [ { "id", "title", "startedAt", "duration" } ] }`: the meetings in the bucket that aren't on this computer. Rejects like `settings_sync_test`, or with `off` when sync is off.
- `start` → nothing. Restores those meetings' text now and their audio when it's needed; progress in `sync.restore`.

### Command `settings_close`

`invoke('settings_close')` → nothing. Escape on the top page closes the window.

---

## Mock pages

`node voice/ui/dev/serve.js` serves this folder on 127.0.0.1 and prints the address. The pages need a server: they are ES modules and load their catalogs with `fetch`, and neither works from `file://`.

| URL | Shows |
|---|---|
| `pill.html?mock=<state>&theme=light&lang=nl&backdrop=1` | One pill state; `<state>` is a key of `STATES` in `mock/pill.js`, such as `meeting-screen-button` or `meeting-frames-dropped`. `backdrop=1` draws a document behind the pill, `hit=1` outlines the stop and monitor windows from the last `pill_layout`, `edge=top`, `dev=1`. A click on the monitor button moves to the next demo monitor. |
| `pill.html?mock=dictation`, `mock=meeting`, `mock=cycle` | A dictation; a meeting with a two-step stop you can click; every state in turn |
| `dev/gallery.html?theme=dark&lang=en` | Every pill state side by side |
| `settings.html?mock=1&page=meetings&theme=dark&lang=nl` | Settings at a page or group |
| `&scenario=fresh`, `busy`, `cloud`, `failed`, `dev` | First start; a download with Ownkey on the same keys; cloud meeting transcription without consent; failures; a development copy |
| `&features=screen` | The Settings of the features listed (`screen`, `speakers`, `sync`, comma-separated), as the engine lists them in `state.features` |
| `&noEncoder=1`, `&noOcr=1`, `&screenUnavailable=1` | Screen recording on a computer without a hardware video encoder, without an OCR language, or on a Windows that can't capture; each turns on `features.screen` |
| `&later=1` | Every feature's Settings (`features` all `true`): screen recording, speaker labels, bucket sync. Without it the mock's `features` are `false`, except those named in `&features=screen,speakers,sync` and those a scenario parameter of their own turns on. |
| `&speakers=ready`, `off`, `platform`, `cpu`, `runtime`, `runtimeFailed`, `model`, `modelDamaged`, `consent`, `mistral`, `stopped` | Speaker labels in that state, with `features.speakers` on: ready with Nemotron; the switch off; each reason labels can't run; Mistral's realtime model without and with a stream agreement; a meeting whose labels stopped |
| `&sync=fresh`, `tested`, `on`, `live`, `retrying`, `failing`, `paused`, `unavailable`, `key`, `restore`, `restore-failed` | Bucket sync on Storage: nothing filled in; tested and still off; up to date; a meeting uploading; the bucket out of reach; the bucket refusing, with files lost; a new secret waiting for Test connection, with files waiting; the switch on while the sync store didn't open; an unreadable key; meetings to restore; a stopped restore. `&sync=1` is `on`. Test connection fails for the bucket `fail` and finds another key in the bucket `shared`. |

`node voice/ui/dev/screenshots.js <outDir>` saves every pill state and every Settings page in both themes and languages, a narrow window and the dialogs, through the repository's Electron, without taking focus. Keep the images out of Git; they go on the GitHub issue.

`node voice/ui/dev/check.js` runs behavior checks against the mocks the same way: typed text surviving redraws, consent asked again for another provider, host or way of sending, keys never carried to another provider, speaker labels' reasons and switch, the screen row and its notes, the bucket secret never going through `settings_set`, the sync switch waiting for Test connection, the key dialogs' screen protection and clean-up, the stop and monitor buttons' layout report, the screen notices, the snapshot shape check and reduced motion. `test/voice-ui-tokens.test.js` and `test/voice-ui-i18n.test.js` run with `npm test`.
