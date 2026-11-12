// The live paired run of Rukoo Voice and Rukoo, their acceptance test: a development Voice on the real engine records
// SAPI speech through its recorder's file sources, transcribes it with Orukeet, records the screen, and streams the
// meeting to a development Rukoo over the bridge.
//
//   node voice/scripts/live-meeting.mjs [--skip-build] [--release] [--screen off|frames|video] [--real-screen]
//                                       [--choose-monitor=<\\.\DISPLAYn>] [--visible-rukoo] [--speakers[=cpu|runtime|model]]
//                                       [--models <dir>] [--nemo-dir <dir>]
//                                       [--sync] [--remote-fetch=<ssh target>]
//                                       [--measure=screen [--seconds=<n>] | --measure=speakers [--minutes=<n>] | --measure=sync]
//
// Windows and macOS. Everything it starts lives under tmp/live/<time>/ in this checkout: Voice's data folder
// (RUKOO_VOICE_DATA_DIR), Rukoo's profile (SEM_DATA_DIR), the discovery file both get as RUKOO_VOICE_BRIDGE, and the
// speech. Captures of the apps' own windows go to tmp/live-shots/. --models seeds the run's data folder with a models
// root in Voice's layout (<root>/orukeet/<revision>) without another copy: an APFS clone on macOS, a hard link
// elsewhere, a copy as the last resort. Without --models it uses tmp/voice-assets/models in this checkout when that
// holds Orukeet, so a Mac run transcribes; Windows still reads Ownkey's model folder when it isn't given one. Nothing
// touches the installed apps, the clipboard or the keyboard, nothing plays out loud, and every process it started is
// stopped by its process id. Cargo builds into CARGO_TARGET_DIR when it is set, else voice/target, and the programs
// run from there: debug builds unless --release is given, release builds for --measure=screen. Rukoo's MCP server
// listens on port 47810, not the 47800 an installed Rukoo uses.
//
// On macOS the default run is --screen off, because the macOS screen process doesn't record yet: --screen frames or
// video, --real-screen, --choose-monitor, --visible-rukoo, --measure and --sync refuse with a message that says what
// they would need. The speech comes from `say` with two installed English voices (the SAPI path stays on Windows). The
// screen process, window-protection, WebView2 capture and video checks are skipped, each with a line in the report that
// says why. --speakers is decided by Voice: when its Settings > Speaker labels reports reason "platform" the run
// refuses with that message, and on a build where the macOS speaker worker exists it runs with --nemo-dir (or
// tmp/voice-assets/nemo-arm64/nemo_speech) and the AMI excerpt and Nemotron from tmp/voice-assets/speaker-assets.
// A run without a model or a provider key still reports that as its one failed prerequisite and skips the passage
// checks: the script seeds a model but never downloads one and never uses a key.
//
// The checks: passages with the expected words reach Rukoo on both tracks before the stop; the pill shows the
// recording; Rukoo's windows are protected from capture; frames with their text reach Rukoo while it records and
// Rukoo gets their images; a killed screen process comes back with a screen gap and no gap in the audio; Voice
// restarted mid-meeting reattaches to the recorder and the screen process and sends no passage twice; a killed
// recorder comes back through the watchdog with a gap mark; the video files hold 1 to 2 s fragments; the stop goes to
// processing, then done, after the screen journal's stop; Rukoo's meeting.get has every passage Voice has; and a
// meeting started from Voice shows up in Rukoo with Voice's default screen mode. The report is
// tmp\live\<time>\report.json: counts, timings and which demo strings matched, never the text read from a frame.
//
// The screen. --screen sets Voice's default (meetings_screen in the dev config.json, default frames); the meeting
// started from Rukoo uses it, and one more short meeting from Rukoo asks for video explicitly. By default the screen
// process draws numbered slides ("Slide 1", "Slide 2" ...) itself (RUKOO_VOICE_TEST_SCREEN) and captures nothing of
// the desktop. --real-screen captures the real displays: a second Electron app puts a demo slide window on one display
// and an always-on-top demo cover on every other, RUKOO_VOICE_TEST_MONITOR pins the slide display, and a run whose
// first monitor isn't that display stops, deletes the meeting's screen folder and fails. Toasts and other apps' pop-ups
// can still draw above the demo windows, so a real-screen run keeps its captures in tmp\, takes no captures of the
// meeting view and deletes Voice's data folder and Rukoo's profile when it ends. Run it only when nobody uses the
// desktop.
//
//   --choose-monitor=<id>  (needs --real-screen) the slide window on <id> is a normal window that gets the focus, no
//                          monitor is pinned, and the run checks that Voice chose <id>, then switches from the pill and
//                          from Rukoo's header.
//   --visible-rukoo        (needs --real-screen) Rukoo's window sits over half the slide; two meetings from Voice,
//                          with Rukoo's hide-from-capture switch off (its demo strings must show in the first frame)
//                          and on (they must not, and the frame must show the slide where Rukoo's window is).
//   --measure=screen       (Windows) CPU and GPU of Voice's processes in four 10-minute meetings with live
//                          transcription, sampled by measure.ps1; release build. --seconds=<n> shortens each meeting for
//                          a trial. On the synthetic screen by default, at the sizes of this computer's displays as
//                          Windows reports them (off, frames and video at the main display's size, then video at the
//                          other display's size), with a new slide every 10 s; nothing shows on the desktop. With
//                          --real-screen, on the demo slide display instead: slides every 10 s, then a scrolling page,
//                          then still. Live speaker labels are off, so the screen's cost stands alone; with --speakers
//                          they are on, the call is the AMI excerpt and the speaker worker is measured too, for the
//                          combined load.
//
// --speakers plays the AMI meeting excerpt (fetched by scripts/speaker-assets.ps1 into tmp/speaker-assets, never
// committed) as the call, looped; the microphone keeps its SAPI speech. On top of the checks above: at least three
// call speakers reach Rukoo before the stop, and the three main AMI speakers get ids of their own; the id of MEE071
// gets a name; the speaker worker is killed and its voice keeps that id afterwards, and again after the Voice
// restart; after done at least 90% of the call's passage time went to the right speaker
// (scripts/lib/speaker-score.mjs) and no passage moved from one id to another between revisions. It doesn't combine
// with --real-screen, whose slides follow the SAPI call. `--nemo-dir` names the NeMo-Speech.cpp runtime (else
// RUKOO_VOICE_NEMO_DIR, else the shared folder, else Voice's lookup), and the AMI excerpt and Nemotron come from the
// shared speaker-assets folder. When Voice reports speaker labels unavailable for the platform (Settings' reason
// "platform"), the run refuses instead of recording a meeting without labels.
//
// --speakers=cpu|runtime|model runs a short meeting with RUKOO_VOICE_TEST_NO_SPEAKERS set to that reason: the call
// is one speaker, call-1, Settings > Speaker labels says why, and Voice captures that page.
//
// --models <dir> seeds each fresh Voice data folder from a models root in Voice's layout (<root>/orukeet/<revision>):
// an APFS clone with cp -c on macOS, a hard link or a copy elsewhere, so the model costs no second 487 MB and Voice
// finds it, hashes it once and keeps its stamp, as after an install. It defaults to tmp/voice-assets/models in this
// checkout when that holds Orukeet, on every platform. The run never downloads a model; --models points at one
// fetched once, for example with Voice's own Settings > Transcription code.
//
// --nemo-dir <dir> is the NeMo-Speech.cpp runtime folder for --speakers (a local macOS arm64 build, for instance); it
// becomes RUKOO_VOICE_NEMO_DIR for Voice. Without it tmp/voice-assets/nemo-arm64/nemo_speech is used when it holds a
// runtime.
//
// --measure=speakers measures labels on and off (spec RUK-109 section 7): the worker's real-time factor through its
// --self-test, then two meetings started from Voice in fresh data folders, labels on and labels off, each --minutes
// long (default 5) with CPU per process read by pid at minute 1 and 5 (and at the end of a longer one), the speaker
// worker's peak working set, and the call passages' latency. With --minutes of 21 or more the labels-on meeting is
// also the compaction run: the settled position keeps moving past minute 20, the worker never restarts, and the three
// main speakers keep their ids from the first five minutes to the last five.
//
// The runtime: RUKOO_VOICE_NEMO_DIR when set; otherwise tmp\speaker-assets\nemo_speech when it holds one; otherwise
// Voice's own order (next to the exe, then Ownkey's copy, read only).
//
// --sync adds bucket sync against voice-s3-fake (rukoo-voice-fake-s3) in tmp\live\<time>\s3\: Voice connects
// through its Settings commands before the meeting; audio objects arrive while it records, and frames too when the
// meeting records the screen; the fake stops for 20 s mid-meeting and the recorder's heartbeats stay within 1,300 ms;
// the Voice restart leaves no object uploaded twice; video goes up only after the stop; after done every object passes
// check-encrypted.mjs, the clips and screen.json never went up, and neither the secret nor the recovery code is in
// plain text; an agent's audio link (get_meeting_media_link) gives the meeting's speech and is refused after its 5 s,
// and its frame and video links give the screen; and a second Voice on a fresh data folder imports the code, restores
// every meeting into Rukoo with its screen journal and speaker ids, and serves the same audio, frame and video from
// the bucket. It combines with --screen and --speakers, never with --real-screen. --measure=sync adds spec-RUK-110's
// measurements to report.json: a 10-minute meeting with sync on, a 3-minute one with sync off and six short ones.
// --remote-fetch=<ssh target> lets that machine, on the tailnet, call the tool over Rukoo's Tailscale listener and
// download the link with curl, read only; it then checks that the link is refused from this computer's own address
// and after it expired. The recovery code and the secret stay in memory.

import { createRequire } from 'node:module';
import { spawn, spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { accuracy, mapping } from './lib/speaker-score.mjs';

const require = createRequire(import.meta.url);
const { _electron: electron } = require('@playwright/test');

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const VOICE = path.join(ROOT, 'voice');
const MEASURES = ['screen', 'speakers', 'sync'];
const REASONS = ['cpu', 'runtime', 'model'];
const SCREENS = ['off', 'frames', 'video'];
const WIN = process.platform === 'win32';
const MAC = process.platform === 'darwin';
// macOS has no .exe suffix, and the screen recorder doesn't record there yet: the default run keeps it off.
const EXE_SUFFIX = WIN ? '.exe' : '';
const DEFAULT_SCREEN = MAC ? 'off' : 'frames';

// Why a flag that needs the screen refuses on macOS, and why the checks that need a screen (or a capture to protect
// Rukoo from) are skipped there.
const SCREEN_NEEDS = 'the screen process does not record on macOS yet (voice/crates/voice-screen exits with "needs Windows"; the macOS screen recorder is a later step), so the live run needs --screen off';
const SCREEN_SKIP = 'the macOS screen process does not record yet (the screen recorder is a later step), so this run has no screen to record, switch or inspect';
const PROTECTION_SKIP = 'window protection belongs to the Windows screen-capture checks; the macOS screen recorder is not in this build, so there is no capture to hide Rukoo from';
const CAPTURE_SKIP = 'WebView2 captures are Windows-only; the macOS WKWebView capture is a later step, so only Rukoo\'s own window is captured';

// The shared folder under this checkout's ignored tmp/: the downloaded models, the speaker runtime and the speaker
// assets. Each run's data folder is seeded from it, so a fresh run doesn't download 487 MB or 100 MB again. --models
// and --nemo-dir point elsewhere.
const SHARED = path.join(ROOT, 'tmp', 'voice-assets');
const SHARED_MODELS = path.join(SHARED, 'models');
const SHARED_ASSETS = path.join(SHARED, 'speaker-assets');
const SHARED_NEMO = path.join(SHARED, 'nemo-arm64', 'nemo_speech');
// ModelSpec::orukeet() in voice-core: the folder and revision under a models root, the same path under Voice's data
// folder, and the files it checks.
const ORUKEET = {
  id: 'orukeet-onnx-int8',
  folder: 'orukeet',
  revision: '55a984d46f68323301837194ce647c702f55facc',
  files: ['encoder.int8.onnx', 'decoder.int8.onnx', 'joiner.int8.onnx', 'tokens.txt', 'bpe.vocab', 'LICENSE-WEIGHTS', 'NOTICE.md']
};
const ORUKEET_REL = path.join(ORUKEET.folder, ORUKEET.revision);
const ORUKEET_DIR = path.join('models', ORUKEET_REL);
// What --speakers needs when Voice itself says the platform can't label speakers.
const SPEAKER_NEEDS =
  "Speaker labels are unavailable on this Mac: Voice's Settings > Speaker labels reports reason \"platform\" through the status hook. " +
  'Speaker labels on macOS need the arm64 NeMo-Speech.cpp runtime (voice/nemo-runtime/build-macos.sh) pointed at with --nemo-dir or ' +
  `RUKOO_VOICE_NEMO_DIR, the Nemotron model and the AMI excerpt in ${SHARED_ASSETS}, and a Voice build with the macOS speaker worker`;

class Refusal extends Error {}

function refuseOnMac(out) {
  if (!MAC) return;
  const no = [];
  if (out.screenGiven && out.screen !== 'off') no.push(`--screen ${out.screen}: ${SCREEN_NEEDS}`);
  if (out.realScreen) no.push(`--real-screen: ${SCREEN_NEEDS}`);
  if (out.chooseMonitor) no.push(`--choose-monitor: ${SCREEN_NEEDS}`);
  if (out.visibleRukoo) no.push(`--visible-rukoo: ${SCREEN_NEEDS}`);
  if (out.measure) no.push(`--measure=${out.measure}: the measurements use voice/scripts/measure.ps1, the screen's own timings and the speaker worker, which are Windows-only`);
  if (out.sync) no.push("--sync: bucket sync's keys need the macOS Keychain protector (voice-sync/src/protect/keychain.rs) in this build, and Voice reports sync as unavailable without it; run without --sync");
  if (no.length) throw new Refusal(`The live run refuses these flags on macOS for now:\n  ${no.join('\n  ')}`);
}

function parseArgs(argv) {
  const out = { skipBuild: false, release: false, screen: DEFAULT_SCREEN, screenGiven: false, realScreen: false, chooseMonitor: null, visibleRukoo: false, speakers: null, sync: false, remoteFetch: null, measure: null, seconds: 600, minutes: 5, models: null, nemoDir: null };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const eq = arg.indexOf('=');
    const key = eq > 0 ? arg.slice(0, eq) : arg;
    const value = eq > 0 ? arg.slice(eq + 1) : null;
    const next = () => value ?? argv[++i];
    switch (key) {
      case '--skip-build':
        out.skipBuild = true;
        break;
      case '--release':
        out.release = true;
        break;
      case '--models':
        out.models = next();
        if (!out.models) throw new Error('--models takes a models root: <root>/orukeet/<revision>/ holds the model');
        break;
      case '--nemo-dir':
        out.nemoDir = next();
        if (!out.nemoDir) throw new Error('--nemo-dir takes the folder holding the NeMo-Speech.cpp runtime (runtime.json and the libraries)');
        break;
      case '--speakers':
        if (value !== null && !REASONS.includes(value)) throw new Error(`--speakers takes no value or one of ${REASONS.join(', ')}`);
        out.speakers = value ?? true;
        break;
      case '--minutes':
        out.minutes = Number(next());
        if (!Number.isInteger(out.minutes) || out.minutes < 1) throw new Error('--minutes takes a whole number of minutes, at least 1');
        break;
      case '--screen':
        out.screen = next();
        out.screenGiven = true;
        if (!SCREENS.includes(out.screen)) throw new Error(`--screen takes ${SCREENS.join(', ')}`);
        break;
      case '--real-screen':
        out.realScreen = true;
        break;
      case '--choose-monitor': {
        // \\.\DISPLAY2, or DISPLAY2: Git Bash turns the leading \\ into \ on its way to node.
        const n = /^(?:\\{1,2}\.\\)?DISPLAY(\d+)$/i.exec(next() || '');
        if (!n) throw new Error('--choose-monitor takes a display id such as \\\\.\\DISPLAY2 or DISPLAY2');
        out.chooseMonitor = `\\\\.\\DISPLAY${n[1]}`;
        break;
      }
      case '--visible-rukoo':
        out.visibleRukoo = true;
        break;
      case '--sync':
        out.sync = true;
        break;
      case '--remote-fetch':
        out.remoteFetch = next();
        if (!out.remoteFetch) throw new Error('--remote-fetch takes an ssh target');
        break;
      case '--measure':
        if (!MEASURES.includes(value)) throw new Error(`--measure takes a value: ${MEASURES.map((m) => `--measure=${m}`).join(', ')}`);
        out.measure = value;
        break;
      case '--seconds':
        out.seconds = Number(next());
        if (!Number.isInteger(out.seconds) || out.seconds < 30) throw new Error('--seconds takes a whole number of seconds, at least 30');
        break;
      default:
        throw new Error(`unknown option ${arg}`);
    }
  }
  if ((out.chooseMonitor || out.visibleRukoo) && !out.realScreen) throw new Error('--choose-monitor and --visible-rukoo need --real-screen');
  if ([out.chooseMonitor, out.visibleRukoo, out.measure].filter(Boolean).length > 1) throw new Error('--choose-monitor, --visible-rukoo and --measure are separate runs');
  if (out.speakers !== null && out.realScreen) throw new Error('--speakers and --real-screen are separate runs: the slides follow the SAPI call');
  if (REASONS.includes(out.speakers) && out.measure) throw new Error('--speakers=<reason> and --measure are separate runs');
  if (out.measure === 'speakers' && out.speakers !== null) throw new Error('--measure=speakers plays the AMI excerpt by itself; leave out --speakers');
  if (out.measure === 'sync') out.sync = true;
  // A real-screen run deletes what it captured; it never uploads the real screen to a bucket (contract 1.13).
  if (out.sync && out.realScreen) throw new Error('--sync and --real-screen are separate runs: the real screen never goes to a bucket');
  if (out.sync && (out.measure === 'screen' || out.measure === 'speakers' || REASONS.includes(out.speakers))) throw new Error('--sync runs the standard meeting; --measure=screen, --measure=speakers and --speakers=<reason> are separate runs');
  if (out.remoteFetch && !out.sync) throw new Error('--remote-fetch needs --sync');
  refuseOnMac(out);
  return out;
}

// A refused flag prints its message without a stack trace, before anything is started or written.
function parsedArgs(argv) {
  try {
    return parseArgs(argv);
  } catch (err) {
    if (!(err instanceof Refusal)) throw err;
    console.error(err.message);
    process.exit(2);
  }
}

const ARGS = parsedArgs(process.argv.slice(2));
// --release builds and runs the release programs, as the measurements should; --measure=screen always does.
const RELEASE = ARGS.release || ARGS.measure === 'screen';
const PROFILE = RELEASE ? 'release' : 'debug';
const SPEAKERS = ARGS.speakers;
const MEASURE = ARGS.measure;
const MINUTES = ARGS.minutes;
const SYNC = ARGS.sync;
const REMOTE_FETCH = ARGS.remoteFetch;
// The AMI excerpt is the call in every speaker run.
const AMI = SPEAKERS !== null || MEASURE === 'speakers';
const STAMP = new Date().toISOString().replace(/[:.]/g, '-');
const RUN = path.join(ROOT, 'tmp', 'live', STAMP);
const SHOTS = path.join(ROOT, 'tmp', 'live-shots');
const TARGET = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(VOICE, 'target');
const BIN = path.join(TARGET, PROFILE);
const EXE = path.join(BIN, `rukoo-voice${EXE_SUFFIX}`);
const SPEAKERS_EXE = path.join(BIN, `rukoo-voice-speakers${EXE_SUFFIX}`);
const SCREEN_EXE = path.join(BIN, `rukoo-voice-screen${EXE_SUFFIX}`);
const FAKE_S3 = path.join(BIN, `rukoo-voice-fake-s3${EXE_SUFFIX}`);
const SHERPA = process.env.SHERPA_ONNX_ARCHIVE_DIR || path.join(ROOT, 'tmp', 'sherpa-archive');
// The models root this run seeds from: --models, else the shared folder when it holds the pinned Orukeet.
const MODELS = ARGS.models
  ? path.resolve(ARGS.models)
  : fs.existsSync(path.join(SHARED_MODELS, ORUKEET_REL, 'encoder.int8.onnx'))
    ? SHARED_MODELS
    : null;
const NEMO_DIR = ARGS.nemoDir ? path.resolve(ARGS.nemoDir) : null;
// The speaker assets: tmp/voice-assets/speaker-assets when it holds them, else tmp/speaker-assets, which the asset
// scripts fill.
const ASSETS =
  MAC &&
  fs.existsSync(path.join(SHARED_ASSETS, 'ami', 'ami_en2002d_2132.wav')) &&
  fs.existsSync(path.join(SHARED_ASSETS, 'nemotron', 'Nemotron-3-Diarization.q8_0.gguf'))
    ? SHARED_ASSETS
    : path.join(ROOT, 'tmp', 'speaker-assets');
const AMI_WAV = path.join(ASSETS, 'ami', 'ami_en2002d_2132.wav');
const AMI_RTTM = path.join(ASSETS, 'ami', 'ami_en2002d_2132.rttm');
const NEMOTRON = path.join(ASSETS, 'nemotron', 'Nemotron-3-Diarization.q8_0.gguf');
// ModelSpec::nemotron() in voice-core: Voice's own models folder keeps it at <data>\models\<folder>\<revision>.
const NEMOTRON_DIR = path.join('models', 'nemotron-3-diarization', 'f667ed73aee57d40cc39428eb768b4fd87a0a29e');
// The three AMI speakers who talk for more than a few words; FEO072 says three.
const MAIN = ['MEE071', 'FEO070', 'MEE073'];
// Bucket sync: the fake bucket's folder, the second Voice of the restore on a fresh data folder, Rukoo's MCP port
// (an installed Rukoo listens on 47800) and the lifetime of the run's links, so it sees one refused.
const S3_DIR = path.join(RUN, 's3');
const VOICE_DATA_2 = path.join(RUN, 'voice-data-2');
const MCP_PORT = 47810;
const LINK_TTL_MS = 5000;
const TITLE = 'Live run: launch review';
// The recorder's segments are 30 s (voice/RECORDING.md).
const SEGMENT_MS = 30000;

// Where the current Voice and Rukoo keep their files. --measure gives each of its meetings a fresh set.
let VOICE_DATA;
let RUKOO_DATA;
let BRIDGE;
let HOOKS;
function folders(dir) {
  VOICE_DATA = path.join(dir, 'voice-data');
  RUKOO_DATA = path.join(dir, 'rukoo-data');
  BRIDGE = path.join(dir, 'bridge', 'voice-bridge.json');
  HOOKS = path.join(dir, 'hooks');
  fs.mkdirSync(path.dirname(BRIDGE), { recursive: true });
}

// The synthetic screen of every run without --real-screen (contract 1.13). --measure=screen sets its size to a display's.
const synthetic = (size) => `synthetic:${size},change-every=10000,text=Slide`;
let syntheticSize = '1920x1080';

// What each track says. The words checked in Rukoo are distinct per track. The call's parts start the demo slides
// of a real-screen run: slide k shows from the moment part k starts in the call's WAV.
const YOU = {
  parts: [
    'Good morning everyone, thanks for joining the launch review.',
    'The budget for the Thursday launch is approved.',
    'Please send the slides to Maria before Friday.',
    "Let's move on to the marketing timeline."
  ],
  words: ['launch', 'budget', 'slides', 'marketing', 'thursday', 'friday']
};
const THEM = {
  lead: 1500,
  parts: [
    'Thanks, the invoice from Amsterdam arrived yesterday.',
    "Let's look at slide two, the warehouse contract.",
    'The deadline for the warehouse contract is next Tuesday.',
    'I will call the supplier about the delivery.'
  ],
  // With the AMI excerpt as the call: words its passages say.
  words: AMI ? ['speaker', 'organis', 'organiz', 'segment', 'steve', 'evaluation', 'time'] : ['invoice', 'amsterdam', 'deadline', 'warehouse', 'supplier', 'delivery', 'tuesday']
};

// The bucket secret the run chooses and the recovery code Voice shows: in memory only, never in the report.
const SECRETS = { secret: `test-secret-${crypto.randomBytes(12).toString('hex')}`, code: null };

// The ssh target stays out of the report.
const report = {
  started: new Date().toISOString(),
  run: RUN,
  platform: process.platform,
  options: { ...ARGS, remoteFetch: Boolean(ARGS.remoteFetch) },
  checks: [],
  notes: [],
  screen: {},
  measurements: {},
  models: { root: MODELS, seeded: [] }
};
const started = { voice: [], rukoo: null, screens: null, measure: [], dataDirs: [], fake: [] };
let failed = 0;
let skipped = 0;
let nemoDir = null;
// Whether this run's data folder holds the seeded Orukeet, so its stamp is worth checking.
let seededOrukeet = false;
// Whether meetings transcribe in this run (macOS decides it from Voice's Settings; Windows keeps its Orukeet in
// Ownkey's folder and doesn't ask).
let TRANSCRIPT = { ready: true };

function check(name, ok, detail = null) {
  report.checks.push({ name, ok: Boolean(ok), detail });
  if (!ok) failed += 1;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? `: ${typeof detail === 'string' ? detail : JSON.stringify(detail)}` : ''}`);
  return ok;
}

// A check that can't hold on this platform yet. It says why, in the report and on the console, and isn't a pass.
function skip(name, reason) {
  report.checks.push({ name, skipped: true, reason });
  skipped += 1;
  console.log(`skip ${name}: ${reason}`);
  return false;
}

function note(text) {
  report.notes.push(text);
  console.log(`     ${text}`);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function until(what, fn, ms = 60000, every = 250) {
  const end = Date.now() + ms;
  for (;;) {
    const value = await fn();
    if (value) return value;
    if (Date.now() > end) throw new Error(`timed out after ${ms / 1000} s waiting for ${what}`);
    await sleep(every);
  }
}

function alive(pid) {
  if (!pid) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === 'EPERM';
  }
}

// TerminateProcess on Windows and SIGKILL here: a hard kill of this one process id, as a crash would be.
function killPid(pid) {
  try {
    process.kill(pid, 'SIGKILL');
    return true;
  } catch (_) {
    return false;
  }
}

// A polite stop of one process id: SIGTERM first, SIGKILL when it is still there after ms. Windows stops it the way it
// always did, with TerminateProcess.
async function stopPid(pid, ms = 10000) {
  if (!pid || !alive(pid)) return;
  if (WIN) return void killPid(pid);
  try {
    process.kill(pid, 'SIGTERM');
  } catch (_) {
    return;
  }
  const end = Date.now() + ms;
  while (Date.now() < end && alive(pid)) await sleep(100);
  if (alive(pid)) killPid(pid);
}

// A process and its descendants, by pid: on Windows one taskkill /T, else the children first (ps ppid), so Electron's
// helpers go with it.
function killTree(pid) {
  if (!pid) return;
  if (WIN) {
    spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
    return;
  }
  const out = spawnSync('ps', ['-axo', 'pid=,ppid='], { encoding: 'utf8' });
  const children = new Map();
  for (const line of String(out.stdout || '').split('\n')) {
    const [child, parent] = line.trim().split(/\s+/).map(Number);
    if (!child) continue;
    if (!children.has(parent)) children.set(parent, []);
    children.get(parent).push(child);
  }
  const tree = [];
  const walk = (p) => {
    tree.push(p);
    for (const c of children.get(p) || []) walk(c);
  };
  walk(pid);
  for (const p of tree.reverse()) killPid(p);
}

function powershell(script, args = []) {
  const out = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', script, ...args], { encoding: 'utf8', windowsHide: true });
  if (out.status !== 0) throw new Error(`PowerShell failed: ${out.stderr}`);
  return out.stdout;
}

const median = (xs) => percentile(xs, 0.5);
function percentile(xs, p) {
  if (!xs.length) return null;
  const sorted = [...xs].sort((a, b) => a - b);
  const at = (sorted.length - 1) * p;
  const lo = Math.floor(at);
  const hi = Math.ceil(at);
  return Math.round(sorted[lo] + (sorted[hi] - sorted[lo]) * (at - lo));
}

// ---------- build, assets and speech ----------

function build() {
  const crates = ['voice-app', 'voice-recorder', 'voice-local', 'voice-speakers', 'voice-screen', ...(SYNC ? ['voice-s3-fake'] : [])];
  console.log(`building ${crates.join(', ')} (${PROFILE}) into ${TARGET}`);
  const out = spawnSync('cargo', ['build', ...(RELEASE ? ['--release'] : []), ...crates.flatMap((c) => ['-p', c])], {
    cwd: VOICE,
    env: { ...process.env, SHERPA_ONNX_ARCHIVE_DIR: SHERPA },
    stdio: 'inherit',
    windowsHide: true
  });
  if (out.status !== 0) throw new Error('cargo build failed');
}

// The AMI excerpt and the model, checked against their pins; the runtime too once voice/nemo-runtime/manifest.json
// names a published archive (the script skips it before that).
// On Windows the PowerShell script fetches the AMI excerpt, Nemotron and the published runtime. On macOS the same
// folder comes from tmp/voice-assets/speaker-assets, or from voice/scripts/speaker-assets.mjs once that script exists
// (the runtime is a local arm64 build, so --nemo-dir points at it instead of downloading one).
function assets() {
  if (WIN) {
    const out = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join(VOICE, 'scripts', 'speaker-assets.ps1'), '-Out', ASSETS, '-Ami', '-Model', '-Runtime'], {
      encoding: 'utf8',
      windowsHide: true
    });
    if (out.status !== 0 || !fs.existsSync(AMI_WAV)) throw new Error(`speaker-assets.ps1 failed: ${out.stderr || out.stdout}`);
  } else {
    const script = path.join(VOICE, 'scripts', 'speaker-assets.mjs');
    if (fs.existsSync(script)) {
      const args = [script, '--out', ASSETS, '--model', '--ami'];
      if (NEMO_DIR) args.push('--nemo-dir', NEMO_DIR);
      const out = spawnSync(process.execPath, args, { encoding: 'utf8' });
      if (out.status !== 0 || !fs.existsSync(AMI_WAV)) throw new Error(`speaker-assets.mjs failed: ${out.stderr || out.stdout}`);
    } else if (!fs.existsSync(AMI_WAV) || !fs.existsSync(NEMOTRON)) {
      throw new Error(`speaker assets are missing: ${AMI_WAV} and ${NEMOTRON}. Fetch them with voice/scripts/speaker-assets.mjs, or put them in ${ASSETS}`);
    }
  }
  if (NEMO_DIR && fs.existsSync(path.join(NEMO_DIR, 'runtime.json'))) nemoDir = NEMO_DIR;
  else if (process.env.RUKOO_VOICE_NEMO_DIR) nemoDir = process.env.RUKOO_VOICE_NEMO_DIR;
  else if (fs.existsSync(path.join(ASSETS, 'nemo_speech', 'runtime.json'))) nemoDir = path.join(ASSETS, 'nemo_speech');
  else if (fs.existsSync(path.join(SHARED_NEMO, 'runtime.json'))) nemoDir = SHARED_NEMO;
  if (nemoDir) {
    const runtime = JSON.parse(fs.readFileSync(path.join(nemoDir, 'runtime.json'), 'utf8').replace(/^﻿/, ''));
    const toolset = runtime.toolset?.msvc ? `MSVC ${runtime.toolset.msvc}` : runtime.build ? 'this platform' : '?';
    note(`NeMo runtime: ${nemoDir} (${runtime.name || 'unnamed'}, commit ${String(runtime.commit || '').slice(0, 7)}, ${toolset})`);
  } else {
    note("NeMo runtime: Voice's own lookup (next to the exe, then Ownkey's copy, read only)");
  }
}

// The AMI reference: its RTTM turns in ms, and the length of one loop of the WAV.
let AMI_TURNS = [];
let AMI_MS = 0;
function loadAmi() {
  AMI_TURNS = fs
    .readFileSync(AMI_RTTM, 'utf8')
    .split('\n')
    .filter((line) => line.startsWith('SPEAKER '))
    .map((line) => {
      const f = line.trim().split(/\s+/);
      const start = Math.round(Number(f[3]) * 1000);
      return { start, end: start + Math.round(Number(f[4]) * 1000), speaker: f[7] };
    });
  AMI_MS = Math.round((fs.statSync(AMI_WAV).size - 44) / 32);
}

// SAPI speech as 16 kHz mono 16-bit WAV, with a pause after every sentence so the pipeline can cut windows there.
function sapiSpeech(file, { parts, lead = 0 }) {
  const script = path.join(RUN, 'speak.ps1');
  fs.writeFileSync(
    script,
    [
      'param([string]$Out, [string]$Parts, [int]$Lead)',
      'Add-Type -AssemblyName System.Speech',
      '$s = New-Object System.Speech.Synthesis.SpeechSynthesizer',
      "$v = $s.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -like 'en-*' } | Select-Object -First 1",
      'if ($v) { $s.SelectVoice($v.VoiceInfo.Name) }',
      '$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)',
      '$s.SetOutputToWaveFile($Out, $fmt)',
      '$p = New-Object System.Speech.Synthesis.PromptBuilder',
      'if ($Lead -gt 0) { $p.AppendBreak([TimeSpan]::FromMilliseconds($Lead)) }',
      "foreach ($part in $Parts.Split('|')) { $p.AppendText($part); $p.AppendBreak([TimeSpan]::FromMilliseconds(2500)) }",
      '$s.Speak($p)',
      '$s.Dispose()'
    ].join('\r\n')
  );
  const out = spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script, '-Out', file, '-Parts', parts.join('|'), '-Lead', String(lead)], { encoding: 'utf8', windowsHide: true });
  if (out.status !== 0 || !fs.existsSync(file)) throw new Error(`SAPI failed: ${out.stderr}`);
  return fs.statSync(file).size;
}

// The pause SAPI's prompt builder puts after every sentence, and the silence before a lead.
const SPEECH_GAP_MS = 2500;
const PEAK = { voices: null, offsets: new Map() };
const SAMPLES_PER_MS = 16;

// `say -v '?'` lists the installed voices; two ordinary English ones are picked, never the novelty voices (Bells and
// the like sing), so the two tracks are different voices as well as different words. macOS installs at least one
// English voice, and Samantha and Daniel are the usual pair.
function sayVoices() {
  if (PEAK.voices) return PEAK.voices;
  const out = spawnSync('say', ['-v', '?'], { encoding: 'utf8' });
  if (out.status !== 0) throw new Error(`say -v '?' failed: ${out.stderr || `exit ${out.status}`}`);
  const names = [];
  const locales = new Map();
  // Each line is "<voice> <locale>    # <sample>"; a localized voice has spaces and parentheses in its name.
  for (const line of String(out.stdout || '').split('\n')) {
    const sample = /^(.*?)\s+#/.exec(line);
    const head = sample ? sample[1].trim() : line.trim();
    if (!head) continue;
    const locale = /^(.*)\s+([a-z]{2}_[A-Z]{2})$/.exec(head);
    const name = locale ? locale[1].trim() : head;
    if (!name || names.includes(name)) continue;
    names.push(name);
    if (locale) locales.set(name, locale[2]);
  }
  const novelty = new Set(['Albert', 'Bad News', 'Bahh', 'Bells', 'Boing', 'Bubbles', 'Cellos', 'Deranged', 'Good News', 'Hysterical', 'Jester', 'Organ', 'Superstar', 'Trinoids', 'Whisper', 'Wobble', 'Zarvox']);
  const preferred = ['Samantha', 'Daniel', 'Karen', 'Moira', 'Tessa', 'Fiona', 'Alex', 'Fred', 'Ralph', 'Kathy', 'Junior', 'Agnes', 'Vicki', 'Victoria', 'Eddy', 'Flo', 'Reed'];
  // `say` names a localized voice "Samantha (English (US))" and a plain one "Karen"; the preference is by that base name.
  const base = (n) => n.replace(/\s*\(.*\)$/, '').trim();
  const ordinary = names.filter((n) => !novelty.has(base(n)));
  const english = ordinary.filter((n) => (locales.get(n) || '').startsWith('en'));
  const pool = english.length >= 2 ? english : ordinary;
  const rank = (n) => {
    const i = preferred.indexOf(base(n));
    return i < 0 ? preferred.length : i;
  };
  const order = [...pool].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b)).concat(names.filter((n) => !pool.includes(n)));
  if (order.length < 2) throw new Error("say lists fewer than two voices, so the two tracks can't differ");
  PEAK.voices = { you: order[0], them: order[1], list: order };
  return PEAK.voices;
}

// The samples of a 16-bit mono 16 kHz PCM WAV, checked, whatever chunks the file carries around the data.
function wavSamples(buf, what) {
  let at = 12;
  let fmt = null;
  while (at + 8 <= buf.length) {
    const id = buf.toString('latin1', at, at + 4);
    const size = buf.readUInt32LE(at + 4);
    if (id === 'fmt ') {
      fmt = { format: buf.readUInt16LE(at + 8), channels: buf.readUInt16LE(at + 10), rate: buf.readUInt32LE(at + 12), bits: buf.readUInt16LE(at + 22) };
    }
    if (id === 'data') {
      if (!fmt || fmt.format !== 1 || fmt.channels !== 1 || fmt.rate !== 16000 || fmt.bits !== 16) throw new Error(`${what}: expected 16 kHz mono 16-bit PCM, got ${JSON.stringify(fmt)}`);
      const bytes = Buffer.from(buf.subarray(at + 8, at + 8 + size - (size % 2)));
      return new Int16Array(bytes.buffer, bytes.byteOffset, bytes.length / 2);
    }
    at += 8 + size + (size % 2);
  }
  throw new Error(`${what}: no data chunk`);
}

// One canonical 44-byte-header WAV, so the sizes and the offsets are exact.
function writeWav(file, samples) {
  const data = Buffer.alloc(samples.length * 2);
  for (let i = 0; i < samples.length; i++) data.writeInt16LE(samples[i], i * 2);
  const head = Buffer.alloc(44);
  head.write('RIFF', 0, 'latin1');
  head.writeUInt32LE(36 + data.length, 4);
  head.write('WAVE', 8, 'latin1');
  head.write('fmt ', 12, 'latin1');
  head.writeUInt32LE(16, 16);
  head.writeUInt16LE(1, 20);
  head.writeUInt16LE(1, 22);
  head.writeUInt32LE(16000, 24);
  head.writeUInt32LE(32000, 28);
  head.writeUInt16LE(2, 32);
  head.writeUInt16LE(16, 34);
  head.write('data', 36, 'latin1');
  head.writeUInt32LE(data.length, 40);
  fs.writeFileSync(file, Buffer.concat([head, data]));
}

// One sentence through `say`, as samples: straight to a 16 kHz WAV, else to AIFF and afconvert when this `say` writes
// another format.
function sayPart(text, voice, file) {
  const direct = spawnSync('say', ['-v', voice, '-o', file, '--data-format=LEI16@16000', text], { encoding: 'utf8' });
  if (direct.status === 0 && fs.existsSync(file)) {
    try {
      return wavSamples(fs.readFileSync(file), file);
    } catch (_) {
      // Another format: afconvert below.
    }
  }
  const aiff = file.replace(/\.wav$/, '.aiff');
  const said = spawnSync('say', ['-v', voice, '-o', aiff, text], { encoding: 'utf8' });
  if (said.status !== 0 || !fs.existsSync(aiff)) throw new Error(`say failed for ${voice}: ${said.stderr || said.error?.message || `exit ${said.status}`}`);
  const converted = spawnSync('afconvert', ['-f', 'WAVE', '-d', 'LEI16@16000', '-c', '1', aiff, file], { encoding: 'utf8' });
  fs.rmSync(aiff, { force: true });
  if (converted.status !== 0 || !fs.existsSync(file)) throw new Error(`afconvert failed: ${converted.stderr}`);
  return wavSamples(fs.readFileSync(file), file);
}

// `say` speech as 16 kHz mono 16-bit WAV, one sentence at a time with the same pause SAPI leaves, so the words and the
// part offsets match the Windows run.
function saySpeech(file, { parts, lead = 0, voice }) {
  const chosen = voice || sayVoices().you;
  const silence = (ms) => new Int16Array(Math.round((ms / 1000) * 16000));
  const samples = [];
  const offsets = [];
  let at = lead;
  if (lead > 0) samples.push(silence(lead));
  offsets[0] = at;
  for (const [i, part] of parts.entries()) {
    if (i > 0) {
      samples.push(silence(SPEECH_GAP_MS));
      at += SPEECH_GAP_MS;
      offsets[i] = at;
    }
    const partFile = path.join(RUN, `part-${path.basename(file, '.wav')}-${i}.wav`);
    const pcm = sayPart(part, chosen, partFile);
    samples.push(pcm);
    at += Math.round(pcm.length / SAMPLES_PER_MS);
    fs.rmSync(partFile, { force: true });
  }
  let total = 0;
  for (const s of samples) total += s.length;
  const all = new Int16Array(total);
  let o = 0;
  for (const s of samples) {
    all.set(s, o);
    o += s.length;
  }
  writeWav(file, all);
  PEAK.offsets.set(speechKey({ parts, lead }), offsets);
  return fs.statSync(file).size;
}

function speech(file, track) {
  return WIN ? sapiSpeech(file, track) : saySpeech(file, track);
}

const wavMs = (bytes) => Math.round(((bytes - 44) / 32000) * 1000);

const speechKey = (track) => `${(track.parts || []).join('\u0000')}|${track.lead || 0}`;

// Where each part of the call starts in its WAV: the lengths of the parts before it. SAPI is deterministic, so on
// Windows the WAV of the parts before it gives that offset; `say` records it while it writes the file.
function partOffsets(track) {
  const known = PEAK.offsets.get(speechKey(track));
  if (known) return known;
  const offsets = [track.lead || 0];
  for (let k = 1; k < track.parts.length; k++) {
    const file = path.join(RUN, `prefix-${k}.wav`);
    offsets.push(wavMs(speech(file, { parts: track.parts.slice(0, k), lead: track.lead })));
    fs.rmSync(file, { force: true });
  }
  return offsets;
}

// ---------- Voice ----------

// A fresh data folder: the model as a hard link in Voice's own models folder, so the run doesn't depend on Ownkey
// having it (Voice still prefers Ownkey's copy when there is one), and the settings the run asks for.
// A file into the new data folder without a second copy on disk: an APFS clone with `cp -c` on macOS (Node's
// COPYFILE_FICLONE answers ENOSYS on this build), a hard link elsewhere, and a plain copy as the last resort.
function seedFile(src, dst) {
  if (MAC) {
    const out = spawnSync('/bin/cp', ['-c', src, dst], { encoding: 'utf8' });
    if (out.status === 0) return 'clone';
  } else {
    try {
      fs.linkSync(src, dst);
      return 'link';
    } catch (_) {
      // Another filesystem, or a platform that refuses links: copy below.
    }
  }
  fs.copyFileSync(src, dst);
  return 'copy';
}

// A model in Voice's own layout under the data folder, as an install would have it. The cost is one clone per file on
// APFS and nothing on disk; a copy counts against free space and is reported.
function seedModel(name, from, to, files) {
  fs.mkdirSync(to, { recursive: true });
  const free = () => {
    const s = fs.statfsSync(VOICE_DATA);
    return s.bavail * s.bsize;
  };
  const before = free();
  const how = new Set();
  let bytes = 0;
  for (const file of files) {
    const dst = path.join(to, file);
    if (fs.existsSync(dst)) continue;
    bytes += fs.statSync(path.join(from, file)).size;
    how.add(seedFile(path.join(from, file), dst));
  }
  const freeDropBytes = Math.max(0, before - free());
  const entry = { model: name, from, to, files: files.length, bytes, how: [...how], freeDropBytes };
  report.models.seeded.push(entry);
  note(`seeded ${name} from ${from}: ${files.length} files, ${(bytes / 1e6).toFixed(0)} MB by ${entry.how.join('/')}, ${(freeDropBytes / 1e6).toFixed(1)} MB of disk used`);
  return entry;
}

function prepareVoiceData(config = null) {
  fs.mkdirSync(VOICE_DATA, { recursive: true });
  started.dataDirs.push(VOICE_DATA);
  if (MODELS) {
    const from = path.join(MODELS, ORUKEET_REL);
    if (ORUKEET.files.every((file) => fs.existsSync(path.join(from, file)))) {
      seedModel('Orukeet', from, path.join(VOICE_DATA, ORUKEET_DIR), ORUKEET.files);
      seededOrukeet = true;
    } else {
      note(`no Orukeet under ${from}: meetings can't transcribe until a model is there`);
    }
  }
  if (AMI && fs.existsSync(NEMOTRON)) {
    const dir = path.join(VOICE_DATA, NEMOTRON_DIR);
    fs.mkdirSync(dir, { recursive: true });
    const target = path.join(dir, path.basename(NEMOTRON));
    if (!fs.existsSync(target)) {
      try {
        fs.linkSync(NEMOTRON, target);
      } catch (_) {
        fs.copyFileSync(NEMOTRON, target);
      }
    }
  }
  if (config) fs.writeFileSync(path.join(VOICE_DATA, 'config.json'), JSON.stringify(config));
}

function voiceEnv({ monitor = null, noSpeakers = null, data = VOICE_DATA } = {}) {
  const call = AMI ? `${AMI_WAV},loop` : `${path.join(RUN, 'them.wav')},loop`;
  const env = {
    ...process.env,
    RUKOO_VOICE_DATA_DIR: data,
    RUKOO_VOICE_BRIDGE: BRIDGE,
    // The recorder plays the speech in real time, looping, instead of opening the microphone and the speakers.
    RUKOO_VOICE_TEST_SOURCES: `mic=file:${path.join(RUN, 'you.wav')},loop,call=file:${call}`
  };
  delete env.ELECTRON_RUN_AS_NODE;
  delete env.RUKOO_VOICE_LOCAL_EXE;
  delete env.RUKOO_VOICE_SPEAKERS_EXE;
  delete env.RUKOO_VOICE_TEST_NO_SPEAKERS;
  delete env.RUKOO_VOICE_NEMO_DIR;
  if (nemoDir) env.RUKOO_VOICE_NEMO_DIR = nemoDir;
  if (noSpeakers) env.RUKOO_VOICE_TEST_NO_SPEAKERS = noSpeakers;
  delete env.RUKOO_VOICE_TEST_SCREEN;
  delete env.RUKOO_VOICE_TEST_MONITOR;
  if (!ARGS.realScreen) env.RUKOO_VOICE_TEST_SCREEN = synthetic(syntheticSize);
  else if (monitor) env.RUKOO_VOICE_TEST_MONITOR = monitor;
  return env;
}

// Voice's default screen mode, and any other setting the run asks for, before Voice reads its settings. config.json
// keeps Ownkey's flat keys.
function writeConfig(extra = {}) {
  fs.mkdirSync(VOICE_DATA, { recursive: true });
  const file = path.join(VOICE_DATA, 'config.json');
  let cfg = {};
  try {
    cfg = JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (_) {
    // A fresh data folder.
  }
  cfg.meetings_screen = ARGS.screen;
  Object.assign(cfg, extra);
  fs.writeFileSync(file, JSON.stringify(cfg, null, 2));
}

function startVoice(n, { monitor = null, noSpeakers = null, data = VOICE_DATA } = {}) {
  const log = fs.openSync(path.join(path.dirname(VOICE_DATA), `voice-${n}.log`), 'a');
  const child = spawn(EXE, ['--dev-hooks', HOOKS], { cwd: VOICE, env: voiceEnv({ monitor, noSpeakers, data }), stdio: ['ignore', log, log], windowsHide: true });
  started.voice.push(child.pid);
  console.log(`started Voice ${n}, pid ${child.pid}${monitor ? `, screen pinned to ${monitor}` : ''}${ARGS.realScreen ? '' : `, synthetic screen ${syntheticSize}`}`);
  return child;
}

let hookId = 0;
async function hook(cmd, args = {}, ms = 60000) {
  const id = ++hookId;
  fs.mkdirSync(HOOKS, { recursive: true });
  const tmp = path.join(HOOKS, `request-${id}.tmp`);
  fs.writeFileSync(tmp, JSON.stringify({ id, cmd, ...args }));
  fs.renameSync(tmp, path.join(HOOKS, 'request.json'));
  const reply = path.join(HOOKS, `reply-${id}.json`);
  await until(`Voice's answer to ${cmd}`, () => fs.existsSync(reply), ms, 100);
  const answer = JSON.parse(fs.readFileSync(reply, 'utf8'));
  // A reply can hold the recovery code; it stays in memory only.
  fs.rmSync(reply, { force: true });
  if (!answer.ok) throw new Error(`${cmd}: ${answer.error}`);
  return answer.result;
}

const meetingDir = (id) => path.join(VOICE_DATA, 'meetings', id);

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
  } catch (_) {
    return null;
  }
}

const recorderOf = (id) => readJson(path.join(meetingDir(id), 'recorder.json'));
const screenOf = (id) => readJson(path.join(meetingDir(id), 'screen', 'screen.json'));

function lines(file) {
  try {
    return fs
      .readFileSync(file, 'utf8')
      .split('\n')
      .filter(Boolean)
      .map((line) => {
        try {
          return JSON.parse(line);
        } catch (_) {
          return null;
        }
      })
      .filter(Boolean);
  } catch (_) {
    return [];
  }
}

const timeline = (id) => lines(path.join(meetingDir(id), 'timeline.jsonl'));
const journal = (id) => lines(path.join(meetingDir(id), 'screen', 'journal.jsonl'));

// Meeting time to the wall clock, from the recorder's first session line.
function wallOf(id, t) {
  const session = timeline(id).find((l) => l.kind === 'session');
  return session ? session.wall + (t - session.t) : null;
}

// The meeting's time now, in ms, from its newest heartbeat.
function meetingNow(id) {
  const beats = timeline(id).filter((l) => l.kind === 'heartbeat');
  const last = beats.at(-1);
  return last ? last.t + (Date.now() - last.wall) : 0;
}

// A file of Voice's data folder through SQLite, read only.
async function readDb(file, query, ...params) {
  const { DatabaseSync } = await import('node:sqlite');
  const db = new DatabaseSync(file, { readOnly: true });
  try {
    return db.prepare(query).all(...params);
  } finally {
    db.close();
  }
}

// What Voice's event log holds, read only: the states a meeting went through, or how many status events it keeps.
const voiceLog = (query, ...params) => readDb(path.join(VOICE_DATA, 'bridge.db'), query, ...params);
const transcriptDb = (id, query, ...params) => readDb(path.join(VOICE_DATA, 'meetings', id, 'transcript.db'), query, ...params);


const voiceStates = async (id) =>
  (await voiceLog("SELECT data FROM events WHERE event = 'meeting.state' AND meeting = ? ORDER BY seq", id)).map((row) => JSON.parse(row.data).state);
const statusRows = async () => (await voiceLog("SELECT COUNT(*) AS n FROM events WHERE event = 'status'"))[0].n;
// Every passage event of a meeting, in order, with the time the log took it.
const passageEvents = async (id) =>
  (await voiceLog("SELECT created_at, data FROM events WHERE event = 'passage' AND meeting = ? ORDER BY seq", id)).map((row) => ({ createdAt: row.created_at, ...JSON.parse(row.data) }));

// processing comes before done, which comes last.
function stoppedThenDone(states) {
  const processing = states.indexOf('processing');
  return processing >= 0 && processing < states.lastIndexOf('done') && states.at(-1) === 'done';
}

// The meeting's entry in Settings > Speaker labels.
async function labelsOf(id) {
  const s = await hook('status');
  return (s.settings.speakers?.live || []).find((l) => l.meeting === id) || null;
}

// ---------- the speaker reference ----------

// The AMI turns on the meeting clock: the file source starts the WAV at the first call segment of every recorder
// session and loops it without a gap, until the next session.
function reference(id) {
  const lines = timeline(id);
  const starts = [];
  let open = false;
  for (const l of lines) {
    if (l.kind === 'session') open = true;
    else if (open && l.kind === 'segment' && l.track === 'call') {
      starts.push(l.t);
      open = false;
    }
  }
  const last = Math.max(0, ...lines.map((l) => (Number.isFinite(l.t) ? l.t : 0)));
  const turns = [];
  starts.forEach((start, i) => {
    const end = i + 1 < starts.length ? starts[i + 1] : last + AMI_MS;
    for (let loop = start; loop < end; loop += AMI_MS) {
      for (const r of AMI_TURNS) {
        const s = loop + r.start;
        const e = Math.min(loop + r.end, end);
        if (s < e) turns.push({ start: s, end: e, speaker: r.speaker });
      }
    }
  });
  return turns;
}

const callPassages = (passages) => passages.filter((p) => p.track === 'call').map((p) => ({ id: p.id, start: p.start, end: p.end, speaker: p.speaker ?? null }));

// The reference speaker who talks most inside a passage, as the scorer picks it.
function referenceOf(passage, turns) {
  const overlap = new Map();
  for (const r of turns) {
    const shared = Math.min(passage.end, r.end) - Math.max(passage.start, r.start);
    if (shared > 0) overlap.set(r.speaker, (overlap.get(r.speaker) || 0) + shared);
  }
  let best = null;
  for (const [name, time] of [...overlap].sort((a, b) => (a[0] < b[0] ? -1 : 1))) if (best === null || time > overlap.get(best)) best = name;
  return best;
}

// The id each main AMI speaker maps to, by the scorer's one-to-one rule.
function mainIds(passages, turns) {
  const map = mapping(passages, turns);
  return Object.fromEntries(MAIN.map((name) => [name, Object.keys(map).find((id) => map[id] === name) || null]));
}

// Passages whose speaker changed from one id to another between revisions; null to an id is allowed.
function movedPassages(events) {
  const seen = new Map();
  const moved = [];
  for (const e of events.filter((p) => p.track === 'call')) {
    if (e.speaker == null) continue;
    const before = seen.get(e.id);
    if (before && before !== e.speaker) moved.push({ id: e.id, from: before, to: e.speaker, revision: e.revision });
    seen.set(e.id, e.speaker);
  }
  return moved;
}

// The largest step between heartbeats whose wall time is in [from, to], by the meeting clock `t`, and whether any
// gap, device_lost, crash or suspend line falls in it.
function heartbeats(lines, from = 0, to = Infinity) {
  const inside = lines.filter((l) => Number.isFinite(l.wall) ? l.wall >= from && l.wall <= to : false);
  const beats = inside.filter((l) => l.kind === 'heartbeat').map((l) => l.t);
  let maxGapMs = 0;
  for (let i = 1; i < beats.length; i++) maxGapMs = Math.max(maxGapMs, beats[i] - beats[i - 1]);
  // Lines without a wall time sit between the heartbeats around them.
  const first = lines.findIndex((l) => l.kind === 'heartbeat' && l.wall >= from);
  let last = -1;
  lines.forEach((l, i) => {
    if (l.kind === 'heartbeat' && l.wall <= to) last = i;
  });
  const trouble = first >= 0 && last >= first ? lines.slice(first, last + 1).filter((l) => ['gap', 'device_lost', 'crash', 'suspend'].includes(l.kind)).map((l) => l.kind) : [];
  return { beats: beats.length, maxGapMs, trouble };
}

// CPU time and I/O transfer counts of one process, from Win32_Process.
function cpuIo(pid) {
  const out = spawnSync(
    'powershell.exe',
    ['-NoProfile', '-Command', `$p = Get-CimInstance Win32_Process -Filter "ProcessId=${Number(pid)}"; if ($p) { '{0} {1} {2} {3}' -f $p.UserModeTime, $p.KernelModeTime, $p.ReadTransferCount, $p.WriteTransferCount }`],
    { encoding: 'utf8', windowsHide: true }
  );
  const [user, kernel, read, write] = String(out.stdout || '').trim().split(/\s+/).map(Number);
  if (![user, kernel, read, write].every(Number.isFinite)) throw new Error(`no process ${pid}`);
  return { at: Date.now(), cpuMs: (user + kernel) / 1e4, readBytes: read, writeBytes: write };
}

function usage(a, b) {
  const minutes = (b.at - a.at) / 60000;
  return {
    seconds: Math.round((b.at - a.at) / 1000),
    cpuSeconds: +((b.cpuMs - a.cpuMs) / 1000).toFixed(2),
    cpuPercentOfOneCore: +(((b.cpuMs - a.cpuMs) / (b.at - a.at)) * 100).toFixed(2),
    readMB: +((b.readBytes - a.readBytes) / 1e6).toFixed(2),
    writeMB: +((b.writeBytes - a.writeBytes) / 1e6).toFixed(2),
    cpuSecondsPerMinute: +((b.cpuMs - a.cpuMs) / 1000 / minutes).toFixed(3)
  };
}

// ---------- the fake bucket (--sync) ----------

const fake = { child: null, endpoint: null, port: 0 };

// rukoo-voice-fake-s3 on port (0: any), serving S3_DIR; resolves once it prints where it listens.
async function startFakeS3(port = 0) {
  fs.mkdirSync(S3_DIR, { recursive: true });
  const log = fs.openSync(path.join(RUN, 'fake-s3.log'), 'a');
  const child = spawn(FAKE_S3, ['--port', String(port), '--bucket', 'test', '--access-key', 'test', '--dir', S3_DIR], { stdio: ['ignore', 'pipe', log], windowsHide: true });
  started.fake.push(child.pid);
  const endpoint = await new Promise((resolve, reject) => {
    let out = '';
    const timer = setTimeout(() => reject(new Error('the fake S3 did not say where it listens within 15 s')), 15000);
    child.stdout.on('data', (d) => {
      out += d;
      const m = /listening (http:\/\/\S+)/.exec(out);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    });
    child.once('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`the fake S3 exited with ${code}`));
    });
  });
  child.stdout.resume();
  fake.child = child;
  fake.endpoint = endpoint;
  fake.port = Number(new URL(endpoint).port);
  console.log(`started the fake S3, pid ${child.pid}, ${endpoint}`);
  return endpoint;
}

async function stopFakeS3() {
  const child = fake.child;
  if (!child) return;
  killPid(child.pid);
  await until('the fake S3 to exit', () => !alive(child.pid), 10000);
  fake.child = null;
}

// The fake's request log: { op, key, part, bytes, status, at }.
function s3Log() {
  try {
    return fs
      .readFileSync(path.join(S3_DIR, 'log.jsonl'), 'utf8')
      .split('\n')
      .filter(Boolean)
      .map((line) => JSON.parse(line));
  } catch (_) {
    return [];
  }
}

// Every stored object: key -> size.
function s3Objects() {
  const root = path.join(S3_DIR, 'objects');
  const out = new Map();
  const visit = (dir) => {
    for (const entry of fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }) : []) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) visit(full);
      else out.set(path.relative(root, full).split(path.sep).join('/'), fs.statSync(full).size);
    }
  };
  visit(root);
  return out;
}

const uploads = (log) => log.filter((l) => (l.op === 'put' || l.op === 'upload_part') && l.status >= 200 && l.status < 300);

// Object names stored twice, and parts uploaded twice, over the whole log. Only meeting files count: a manifest
// and the index keep their names and are written again on purpose.
function duplicates(log) {
  const puts = new Map();
  const parts = new Map();
  for (const l of uploads(log)) {
    if (!l.key.includes('/o/')) continue;
    if (l.op === 'put') puts.set(l.key, (puts.get(l.key) || 0) + 1);
    else parts.set(`${l.key}#${l.part}`, (parts.get(`${l.key}#${l.part}`) || 0) + 1);
  }
  const twice = (m) => [...m].filter(([, n]) => n > 1).map(([k, n]) => `${k} x${n}`);
  return { puts: puts.size, parts: parts.size, putTwice: twice(puts), partTwice: twice(parts) };
}

// Upload rate between from and to (wall ms): mean, the busiest 10 s window and the largest request.
function uploadRate(log, from, to) {
  const inside = uploads(log).filter((l) => l.at >= from && l.at <= to);
  const total = inside.reduce((n, l) => n + l.bytes, 0);
  const windows = new Map();
  for (const l of inside) {
    const w = Math.floor((l.at - from) / 10000);
    windows.set(w, (windows.get(w) || 0) + l.bytes);
  }
  const count = Math.max(1, Math.ceil((to - from) / 10000));
  const perWindow = Array.from({ length: count }, (_, i) => windows.get(i) || 0);
  return {
    seconds: Math.round((to - from) / 1000),
    requests: inside.length,
    bytes: total,
    meanKiBs: +(total / 1024 / ((to - from) / 1000)).toFixed(1),
    maxKiBsPer10s: +(Math.max(...perWindow) / 1024 / 10).toFixed(1),
    largestRequest: inside.reduce((n, l) => Math.max(n, l.bytes), 0)
  };
}

// Settings > Storage, through the dev hooks as the page would.
const syncHook = (action, more = {}, ms) => hook('sync', { action, ...more }, ms);
const setting = (key, value) => hook('settings', { key, value });
const syncStatus = () => syncHook('status');

async function connectSync(endpoint, secret) {
  await setting('storage.sync.service', 'other');
  await setting('storage.sync.endpoint', endpoint);
  await setting('storage.sync.region', 'us-east-1');
  await setting('storage.sync.bucket', 'test');
  await setting('storage.sync.accessKeyId', 'test');
  await syncHook('secret', { secret });
  const tested = await syncHook('test');
  await setting('storage.sync.enabled', true);
  return tested;
}

// Settings > Storage in the real app, from WebView2, in both themes and languages: tmp\live-shots\voice-storage-
// <label>-<theme>-<language>.png. The page shows the fake's loopback address and nothing else of this computer.
async function captureStorage(label) {
  const was = (await hook('status')).settings;
  const ui = was.config?.ui || {};
  const shots = [];
  for (const [theme, language] of [['dark', 'en'], ['light', 'en'], ['dark', 'nl'], ['light', 'nl']]) {
    await setting('ui.theme', theme);
    await setting('ui.language', language);
    const dir = path.join(RUN, 'captures', `${label}-${theme}-${language}`);
    const out = await hook('capture', { dir, page: 'storage' }, 90000);
    const file = path.join(SHOTS, `voice-storage-${label}-${theme}-${language}.png`);
    if (!out.settings.error) fs.copyFileSync(out.settings.file, file);
    shots.push(out.settings.error ? { theme, language, error: out.settings.error } : { theme, language, file: path.basename(file) });
  }
  if (ui.theme) await setting('ui.theme', ui.theme);
  if (ui.language) await setting('ui.language', ui.language);
  check(`Voice captured Settings > Storage (${label}) in both themes and languages`, shots.every((s) => !s.error), shots);
}

// Waits until nothing waits to upload, the worker is idle and the index went up after the last manifest, so the
// bucket's index names every meeting.
async function drained(what, ms = 300000) {
  return until(`${what}: the queue to drain and the index to follow`, async () => {
    const s = await syncStatus();
    if (s.waiting !== 0 || !(s.state === 'idle' || s.state === 'live')) return false;
    const puts = s3Log().filter((l) => l.op === 'put' && l.status === 200);
    const last = (test) => puts.filter((l) => test(l.key)).reduce((n, l) => Math.max(n, l.at), 0);
    const manifest = last((k) => k.endsWith('/manifest'));
    return manifest > 0 && last((k) => k.includes('/index/')) > manifest && s;
  }, ms, 1000);
}

// Which files under dir (a folder or one file) hold one of the needles (strings). A file a running process holds
// open without sharing can't be read; it is listed in `locked`, and the scan after every process stopped reads it.
function holders(dir, needles, locked = []) {
  const found = [];
  const look = (full) => {
    let bytes;
    try {
      bytes = fs.readFileSync(full);
    } catch (err) {
      if (err.code === 'EBUSY' || err.code === 'EPERM') return locked.push(path.relative(RUN, full));
      throw err;
    }
    needles.forEach((n, i) => {
      if (n && bytes.includes(Buffer.from(n, 'utf8'))) found.push(`${path.relative(RUN, full)} holds needle ${i + 1}`);
    });
  };
  const visit = (d) => {
    for (const entry of fs.existsSync(d) ? fs.readdirSync(d, { withFileTypes: true }) : []) {
      const full = path.join(d, entry.name);
      if (entry.isDirectory()) visit(full);
      else look(full);
    }
  };
  if (fs.existsSync(dir) && fs.statSync(dir).isFile()) look(dir);
  else visit(dir);
  return found;
}

// ---------- audio ----------

// The samples of a 16-bit mono PCM WAV.
function pcm(buf) {
  let at = 12;
  while (at + 8 <= buf.length) {
    const id = buf.toString('latin1', at, at + 4);
    const size = buf.readUInt32LE(at + 4);
    if (id === 'data') {
      const end = Math.min(buf.length, at + 8 + size);
      const bytes = Buffer.from(buf.subarray(at + 8, end - ((end - at - 8) % 2)));
      return new Int16Array(bytes.buffer, bytes.byteOffset, bytes.length / 2);
    }
    at += 8 + size + (size % 2);
  }
  throw new Error('no data chunk in the WAV');
}

// RMS per 20 ms window (320 samples at 16 kHz).
function envelope(samples) {
  const out = [];
  for (let i = 0; i + 320 <= samples.length; i += 320) {
    let sum = 0;
    for (let j = i; j < i + 320; j++) sum += samples[j] * samples[j];
    out.push(Math.sqrt(sum / 320));
  }
  return out;
}

function pearson(a, b) {
  const n = Math.min(a.length, b.length);
  if (n < 10) return 0;
  let ma = 0;
  let mb = 0;
  for (let i = 0; i < n; i++) {
    ma += a[i];
    mb += b[i];
  }
  ma /= n;
  mb /= n;
  let num = 0;
  let da = 0;
  let db = 0;
  for (let i = 0; i < n; i++) {
    num += (a[i] - ma) * (b[i] - mb);
    da += (a[i] - ma) ** 2;
    db += (b[i] - mb) ** 2;
  }
  return da && db ? num / Math.sqrt(da * db) : 0;
}

// How well a fetched mix follows the two speech files played on a loop and summed: the best correlation of their
// loudness over time, within a second of lag either way (the recorder starts its file sources as it starts).
function matchesSpeech(wav, you, them) {
  const got = pcm(wav);
  const a = pcm(you);
  const b = pcm(them);
  const mix = new Int16Array(got.length + 16000);
  for (let i = 0; i < mix.length; i++) mix[i] = Math.max(-32768, Math.min(32767, a[i % a.length] + b[i % b.length]));
  const eg = envelope(got);
  const em = envelope(mix);
  let best = { r: -1, lagMs: 0 };
  for (let lag = -50; lag <= 50; lag++) {
    const x = lag >= 0 ? eg.slice(lag) : eg;
    const y = lag >= 0 ? em : em.slice(-lag);
    const r = pearson(x, y);
    if (r > best.r) best = { r: +r.toFixed(3), lagMs: lag * 20 };
  }
  return { seconds: +(got.length / 16000).toFixed(2), ...best };
}

// Fetches url: status, headers, the bytes, time to the first byte and the rate.
async function download(url, headers = {}) {
  const t0 = performance.now();
  const res = await fetch(url, { headers });
  const chunks = [];
  let firstMs = null;
  if (res.body) {
    for await (const chunk of res.body) {
      if (firstMs === null) firstMs = performance.now() - t0;
      chunks.push(Buffer.from(chunk));
    }
  }
  const ms = performance.now() - t0;
  const bytes = Buffer.concat(chunks);
  return {
    status: res.status,
    type: res.headers.get('content-type'),
    bytes,
    firstByteMs: firstMs === null ? null : Math.round(firstMs),
    totalMs: Math.round(ms),
    mbPerS: ms > 0 ? +((bytes.length / 1e6) / (ms / 1000)).toFixed(1) : null
  };
}

// ---------- Rukoo ----------

let app;
let win;

async function startRukoo() {
  // Port 47810 for the MCP server: an installed Rukoo listens on 47800.
  const agents = path.join(RUKOO_DATA, 'data', 'agents.json');
  fs.mkdirSync(path.dirname(agents), { recursive: true });
  if (!fs.existsSync(agents)) fs.writeFileSync(agents, JSON.stringify({ version: 1, mcp: { port: MCP_PORT, remote: false, remoteHost: '' } }));
  const env = { ...process.env, SEM_DATA_DIR: RUKOO_DATA, SEM_HIDDEN: '1', SEM_AGENT_FAKE: '1', RUKOO_VOICE_BRIDGE: BRIDGE };
  // Links die after 5 s, so the run sees one refused.
  if (SYNC) env.SEM_MEDIA_LINK_TTL_MS = String(LINK_TTL_MS);
  delete env.ELECTRON_RUN_AS_NODE;
  delete env.RUKOO_VOICE_DATA_DIR;
  app = await electron.launch({ args: ['.'], cwd: ROOT, env });
  started.rukoo = app.process().pid;
  console.log(`started Rukoo, pid ${started.rukoo}`);
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await win.waitForSelector('.provider-grid', { timeout: 60000 });
  await win.click('[data-s="demo"]');
  await win.locator('.item').first().waitFor({ timeout: 30000 });
}

const rukoo = {
  connected: () => app.evaluate(() => global.__semVoice.connected),
  state: () => win.evaluate(() => window.mail.call('voiceState')),
  get: (id) => win.evaluate((m) => window.mail.call('meetingGet', m), id),
  start: (opts) => win.evaluate((o) => window.mail.call('meetingStart', o), opts),
  stop: (id) => win.evaluate((m) => window.mail.call('meetingStop', m), id),
  name: (id, speaker, name) => win.evaluate(([m, s, n]) => window.mail.call('meetingSetSpeaker', m, s, { name: n, address: '' }), [id, speaker, name]),
  frame: (id, at) => win.evaluate(([m, a]) => window.mail.call('meetingFrame', m, a), [id, at]),
  screen: (id) => win.evaluate((m) => window.mail.call('meetingScreen', m), id),
  select: (id, monitor) => win.evaluate(([m, s]) => window.mail.call('meetingScreenSelect', m, s), [id, monitor]),
  setScreen: (id, mode, monitor = null) => win.evaluate(([m, s, d]) => window.mail.call('meetingScreenSet', m, s, d), [id, mode, monitor]),
  settings: (patch) => win.evaluate((p) => window.mail.call('updateSettings', p), patch),
  voiceGet: (id) => app.evaluate((_, m) => global.__semVoice.bridge.command('meeting.get', { meeting: m }), id),
  protectedWindows: () => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().map((w) => w.isContentProtected())),
  // get_meeting_media_link as Claude Code on this computer would call it, through the tool code MCP runs. A call from
  // inside Rukoo carries no MCP token, so the link isn't bound to one.
  mediaLink: (args) =>
    app.evaluate(async (_, a) => {
      const res = await global.__semAgents.callTool({ agent: 'claude', conversationId: null, remote: false, peer: '127.0.0.1' }, 'get_meeting_media_link', a);
      return res.isError ? { error: res.content[0].text } : res.structuredContent;
    }, args),
  linkTtl: (ms) =>
    app.evaluate((_, t) => {
      global.__semAgents.mediaLinks.ttlMs = t;
    }, ms)
};

async function captureRukoo(file) {
  // A real-screen run's meeting view shows frames of the real displays; those stay out of tmp\live-shots.
  if (ARGS.realScreen) return null;
  const base64 = await app.evaluate(async ({ BrowserWindow }) => {
    const image = await BrowserWindow.getAllWindows()[0].webContents.capturePage();
    return image.isEmpty() ? null : image.toPNG().toString('base64');
  });
  if (base64) fs.writeFileSync(file, Buffer.from(base64, 'base64'));
  else await win.screenshot({ path: file });
  return file;
}

// ---------- the demo screens of a real-screen run ----------

let screens;
let displays = [];

const SLIDES = [
  ['Launch review', 'Agenda: budget, slides and the marketing timeline', 'Owner: the launch team'],
  ['Warehouse contract', 'The deadline for the contract is next Tuesday', 'Draft terms are with the supplier'],
  ['Supplier delivery', 'Call the supplier about the delivery dates', 'Two pallets arrive on Monday'],
  ['Marketing timeline', 'Campaign starts the week after the launch', 'Slides go to Maria before Friday']
];

// The page every demo window shows: a slide, a long page that scrolls, or a cover. Demo text only.
const DEMO_PAGE = `<!doctype html><meta charset="utf-8"><title>Rukoo live run</title>
<style>
  html, body { margin: 0; height: 100%; overflow: hidden; font-family: 'Segoe UI', sans-serif; }
  body.slide { background: #f5f1e8; color: #1d2433; }
  body.cover { background: #2b2f36; color: #c9ced6; display: grid; place-items: center; }
  .slide main { padding: 7vh 8vw; }
  .slide h1 { font-size: 9vh; margin: 0 0 5vh; letter-spacing: -0.01em; }
  .slide h2 { font-size: 5.5vh; margin: 0 0 4vh; color: #3c4a66; font-weight: 600; }
  .slide li { font-size: 3.6vh; margin: 0 0 2.2vh; }
  .cover p { font-size: 3vh; }
  .page { height: 100%; overflow: hidden; }
  .page article { padding: 4vh 8vw; font-size: 2.6vh; line-height: 1.6; }
  .page h3 { font-size: 3.4vh; }
</style>
<body><div id="root"></div>
<script>
  const SLIDES = ${JSON.stringify(SLIDES)};
  const root = document.getElementById('root');
  let timer = null;
  window.slide = (n) => {
    clearInterval(timer);
    document.body.className = 'slide';
    const [title, a, b] = SLIDES[(n - 1) % SLIDES.length];
    root.innerHTML = '<main><h1>Slide ' + n + '</h1><h2>' + title + '</h2><ul><li>' + a + '</li><li>' + b + '</li></ul></main>';
  };
  window.cover = () => {
    clearInterval(timer);
    document.body.className = 'cover';
    root.innerHTML = '<p>Demo cover for a Rukoo Voice live run</p>';
  };
  window.scrolling = (on) => {
    clearInterval(timer);
    if (!document.querySelector('.page')) {
      document.body.className = 'slide';
      let html = '';
      for (let i = 1; i <= 120; i++) {
        const [title, a, b] = SLIDES[i % SLIDES.length];
        html += '<h3>' + i + '. ' + title + '</h3><p>' + a + '. ' + b + '. The launch team reviews this item every week.</p>';
      }
      root.innerHTML = '<div class="page"><article>' + html + '</article></div>';
    }
    const page = document.querySelector('.page');
    if (on) timer = setInterval(() => { page.scrollTop += 4; }, 33);
  };
</script></body>`;

// The displays as Electron and Windows see them, matched by their physical bounds.
function windowsDisplays() {
  const script = [
    "Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class Dpi { [DllImport(\"user32.dll\")] public static extern bool SetProcessDPIAware(); }'",
    '[Dpi]::SetProcessDPIAware() | Out-Null',
    'Add-Type -AssemblyName System.Windows.Forms',
    '[System.Windows.Forms.Screen]::AllScreens | ForEach-Object { [pscustomobject]@{ id = $_.DeviceName; x = $_.Bounds.X; y = $_.Bounds.Y; width = $_.Bounds.Width; height = $_.Bounds.Height; primary = $_.Primary } } | ConvertTo-Json -Compress'
  ].join('; ');
  const list = JSON.parse(powershell(script));
  return Array.isArray(list) ? list : [list];
}

async function startScreens() {
  const dir = path.join(RUN, 'screens');
  fs.mkdirSync(dir, { recursive: true });
  // A bare app that only keeps running: the run makes its windows through app.evaluate.
  fs.writeFileSync(path.join(dir, 'main.cjs'), "const { app } = require('electron');\napp.on('window-all-closed', () => {});\n");
  fs.writeFileSync(path.join(dir, 'page.html'), DEMO_PAGE);
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  screens = await electron.launch({ args: [path.join(dir, 'main.cjs')], cwd: ROOT, env });
  started.screens = screens.process().pid;
  console.log(`started the demo screens, pid ${started.screens}`);
  const electronSide = await screens.evaluate(({ screen }) => {
    const primary = screen.getPrimaryDisplay().id;
    return screen.getAllDisplays().map((d) => ({ electronId: d.id, bounds: d.bounds, scaleFactor: d.scaleFactor, physical: screen.dipToScreenRect(null, d.bounds), primary: d.id === primary }));
  });
  const windowsSide = windowsDisplays();
  displays = electronSide.map((d) => {
    const match = windowsSide.find((w) => w.x === d.physical.x && w.y === d.physical.y && w.width === d.physical.width && w.height === d.physical.height);
    return { ...d, id: match ? match.id : null, width: match?.width, height: match?.height };
  });
  report.screen.displays = displays.map((d) => ({ id: d.id, width: d.width, height: d.height, primary: d.primary, scaleFactor: d.scaleFactor }));
  if (displays.some((d) => !d.id)) throw new Error(`couldn't match every Electron display to a Windows display: ${JSON.stringify({ electronSide, windowsSide })}`);
  note(`displays: ${displays.map((d) => `${d.id} ${d.width}x${d.height}${d.primary ? ' (primary)' : ''}`).join(', ')}`);
}

// Makes a window Windows' foreground window, as a click would, without sending input: the helper attaches its input
// queue to the foreground window's thread for the call, which lets SetForegroundWindow through. True when it is.
function foreground(hwnd) {
  const script = [
    "Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class Fg {",
    '[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();',
    '[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);',
    '[DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();',
    '[DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);',
    '[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);',
    '[DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);',
    'public static bool Front(long w) { IntPtr h = new IntPtr(w); uint fg = GetWindowThreadProcessId(GetForegroundWindow(), IntPtr.Zero); uint me = GetCurrentThreadId();',
    'bool attached = fg != 0 && fg != me && AttachThreadInput(me, fg, true); BringWindowToTop(h); SetForegroundWindow(h);',
    "if (attached) AttachThreadInput(me, fg, false); return GetForegroundWindow() == h; } }'",
    `; [Fg]::Front(${BigInt(hwnd)})`
  ].join(' ');
  return powershell(script).trim() === 'True';
}

// One demo window per display: the slide on `slideId`, covers on the others. focused: the slide is a normal window
// that takes the focus (monitor choice); otherwise every demo window stays on top without taking it.
async function showDemoWindows(slideId, { focused = false } = {}) {
  const file = path.join(RUN, 'screens', 'page.html');
  await screens.evaluate(
    async ({ BrowserWindow }, { displays: list, slideId: sid, focused: f, file: page }) => {
      for (const w of BrowserWindow.getAllWindows()) w.destroy();
      global.__demo = { slide: null, covers: [] };
      for (const d of list) {
        const slide = d.id === sid;
        const w = new BrowserWindow({
          ...d.bounds,
          show: false,
          frame: false,
          resizable: false,
          movable: false,
          skipTaskbar: true,
          focusable: slide && f,
          backgroundColor: slide ? '#f5f1e8' : '#2b2f36',
          webPreferences: { backgroundThrottling: false }
        });
        await w.loadFile(page);
        await w.webContents.executeJavaScript(slide ? 'slide(1)' : 'cover()');
        w.setBounds(d.bounds);
        if (slide && f) {
          w.show();
          w.focus();
        } else {
          w.setAlwaysOnTop(true, 'screen-saver');
          w.showInactive();
        }
        if (slide) global.__demo.slide = w;
        else global.__demo.covers.push(w);
      }
    },
    { displays, slideId, focused, file }
  );
  await sleep(1500);
  const where = await screens.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().map((w) => ({ bounds: w.getBounds(), visible: w.isVisible(), focused: w.isFocused() })));
  report.screen.demoWindows = where;
}

const slideCall = (code) => screens.evaluate((_, c) => global.__demo.slide.webContents.executeJavaScript(c), code);
const setSlide = (n) => slideCall(`slide(${Number(n)})`);

// The slides of a real-screen meeting: slide k from the moment part k of the call starts in its WAV, measured from the
// call's first segment; after the last part, a new slide every 10 s. Returns a stop function.
function scheduleSlides(id, offsets) {
  const timers = [];
  let stopped = false;
  (async () => {
    const segment = await until('the call track\'s first segment', () => timeline(id).find((l) => l.kind === 'segment' && l.track === 'call'), 30000, 100);
    const start = wallOf(id, segment.t);
    report.screen.slides = { firstCallSegment: segment.t, offsets };
    offsets.forEach((offset, k) => {
      if (k === 0) return;
      timers.push(setTimeout(() => !stopped && setSlide(k + 1).catch(() => {}), Math.max(0, start + offset - Date.now())));
    });
    let n = offsets.length;
    const after = start + offsets.at(-1) + 10000;
    timers.push(
      setTimeout(() => {
        const every = setInterval(() => !stopped && setSlide(++n).catch(() => {}), 10000);
        timers.push({ every });
        if (!stopped) setSlide(++n).catch(() => {});
      }, Math.max(0, after - Date.now()))
    );
  })().catch((err) => note(`slides: ${err.message}`));
  return () => {
    stopped = true;
    for (const t of timers) {
      if (t && t.every) clearInterval(t.every);
      else clearTimeout(t);
    }
  };
}

// Mean absolute difference, 0 to 255, between a frame and the slide window in a rectangle given as fractions of the
// display, over a 48 x 48 grid of points. The frame comes as a data URL; both images are decoded by Electron.
async function frameAgainstSlide(dataUrl, rect) {
  return screens.evaluate(
    async ({ nativeImage }, { url, r }) => {
      const frame = nativeImage.createFromDataURL(url);
      const slide = await global.__demo.slide.webContents.capturePage();
      const fs = frame.getSize();
      const ss = slide.getSize();
      const fb = frame.toBitmap();
      const sb = slide.toBitmap();
      let sum = 0;
      let n = 0;
      for (let i = 0; i < 48; i++) {
        for (let j = 0; j < 48; j++) {
          const u = r.x + ((i + 0.5) / 48) * r.width;
          const v = r.y + ((j + 0.5) / 48) * r.height;
          const fi = (Math.floor(v * fs.height) * fs.width + Math.floor(u * fs.width)) * 4;
          const si = (Math.floor(v * ss.height) * ss.width + Math.floor(u * ss.width)) * 4;
          for (let c = 0; c < 3; c++) sum += Math.abs(fb[fi + c] - sb[si + c]);
          n += 3;
        }
      }
      return { diff: Math.round((sum / n) * 10) / 10, frame: fs, slide: ss };
    },
    { url: dataUrl, r: rect }
  );
}

// Deletes what a real-screen run captured: the screen folders, Voice's event log with the frames' text, and Rukoo's
// profile with its copy. The report keeps counts only.
// A run that failed before folders() set the paths, as a failed build, recorded nothing to delete.
function deleteRealCaptures() {
  const dirs = [VOICE_DATA, RUKOO_DATA].filter(Boolean);
  if (!dirs.length) return;
  for (const dir of dirs) fs.rmSync(dir, { recursive: true, force: true });
  note('deleted the real-screen captures: Voice\'s data folder and Rukoo\'s profile of this run');
}

// A real-screen meeting must record the display that shows demo data. Otherwise: stop, delete its screen folder, fail.
async function guardMonitor(id, expected) {
  const first = await until('the screen journal\'s first monitor line', () => journal(id).find((l) => l.kind === 'monitor'), 30000, 100);
  if (first.id === expected) return first;
  try {
    await rukoo.stop(id);
  } catch (_) {
    await hook('stop_meeting').catch(() => {});
  }
  await until('the screen process to stop', () => !alive(screenOf(id)?.pid), 20000).catch(() => {});
  fs.rmSync(path.join(meetingDir(id), 'screen'), { recursive: true, force: true });
  throw new Error(`the screen recording started on ${first.id}, not on ${expected}: stopped the meeting and deleted its screen folder`);
}

// ---------- what Rukoo has ----------

const hasWord = (text, words) => words.some((w) => text.toLowerCase().includes(w));

function trackHas(meeting, track, words) {
  return (meeting?.passages || []).some((p) => p.track === track && hasWord(p.text || '', words));
}

// Two passages of one track over the same stretch of audio would mean it was transcribed twice. Windows split at
// pauses and never overlap; a little slack covers word timings at the edges.
function overlaps(passages) {
  const found = [];
  for (const track of ['mic', 'call']) {
    const list = passages.filter((p) => p.track === track).sort((a, b) => a.start - b.start);
    for (let i = 1; i < list.length; i++) {
      if (list[i].start < list[i - 1].end - 250) found.push([list[i - 1].id, list[i].id]);
    }
  }
  return found;
}

const slideTwo = (text) => /\bslide 2\b/i.test(text || '');

// The fragments of a video file as `rukoo-voice-screen inspect` reads them.
function inspect(file) {
  const out = spawnSync(SCREEN_EXE, ['inspect', file], { encoding: 'utf8', windowsHide: true });
  try {
    return JSON.parse(out.stdout);
  } catch (_) {
    return { error: out.stderr || out.stdout };
  }
}

// Every video file of a meeting: complete, read to the end by Media Foundation, and its fragments 1.0 to 2.0 s long
// except the last.
function checkVideos(id, label) {
  const ends = journal(id).filter((l) => l.kind === 'video');
  const files = ends.map((v) => ({ seq: v.seq, file: path.join(meetingDir(id), v.file), width: v.width, height: v.height, encoder: v.encoder, hardware: v.hardware }));
  const results = files.map((f) => {
    const r = inspect(f.file);
    const inner = (r.fragmentMs || []).slice(0, -1);
    return {
      seq: f.seq,
      size: `${f.width}x${f.height}`,
      encoder: f.encoder,
      hardware: f.hardware,
      fragments: r.fragments,
      durationMs: r.durationMs,
      minMs: inner.length ? Math.min(...inner) : null,
      maxMs: inner.length ? Math.max(...inner) : null,
      complete: r.complete,
      readerSamples: r.reader?.samples ?? null,
      fps: r.durationMs ? Math.round((r.samples / r.durationMs) * 10000) / 10 : null,
      readerSize: r.reader && r.reader.width ? `${r.reader.width}x${r.reader.height}` : null,
      ok: !r.error && r.complete && inner.every((ms) => ms >= 1000 && ms <= 2000) && (r.reader?.samples || 0) > 0,
      error: r.error || r.reader?.error || null
    };
  });
  check(`${label}: every video file inspects cleanly with 1 to 2 s fragments`, results.length > 0 && results.every((r) => r.ok), results);
  return results;
}

// ---------- the run ----------

// Whether a meeting can transcribe here: Orukeet in one of Voice's folders, or a cloud provider with its key. This
// run never downloads a model and never uses a key, so on a Mac without either the passage checks are skipped and the
// report says what to install.
function transcriptionOf(settings) {
  const cfg = settings.config || {};
  // Meetings follow the dictation provider when they are set to it; otherwise they have their own.
  const meeting = cfg.meetings || {};
  const dictation = cfg.transcription || {};
  const followsDictation = !meeting.provider || meeting.provider === 'same';
  const provider = followsDictation ? dictation.provider || 'orukeet' : meeting.provider;
  const where = followsDictation ? dictation : meeting;
  const model = settings.models?.orukeet || {};
  if (provider === 'orukeet') {
    if (model.stage === 'installed') return { ready: true, provider, via: `Orukeet at ${model.path}` };
    const missing = MODELS ? path.join(MODELS, ORUKEET_REL) : 'no --models folder and no shared model folder';
    return {
      ready: false,
      provider,
      reason:
        `Voice transcribes meetings with Orukeet, and this run found no model: ${missing} is missing the pinned files. ` +
        `Put them under <root>/${ORUKEET_REL} and pass --models <root>, or install Orukeet in Settings > Transcription; ` +
        `no cloud meeting provider has a key either, and this run never downloads a model or uses a key`
    };
  }
  const local = /^https?:\/\/(127\.0\.0\.1|localhost)[:/]/.test(where.endpoint || '');
  if (where.keySaved === true || local) return { ready: true, provider, via: `${provider}${local ? ` at ${where.endpoint}` : ''}` };
  return { ready: false, provider, reason: `meetings are set to ${provider} with no key in this profile, and this run may not use one; set that provider's key in Settings > Meetings and run again` };
}

async function pair(monitor) {
  // A measurement says whether live speaker labels run; the other runs keep Voice's default, which is on.
  writeConfig(ARGS.measure === 'screen' ? { meetings_live_speakers: SPEAKERS === true } : {});
  const voice = startVoice(1, { monitor });
  await until('the discovery file', () => fs.existsSync(BRIDGE), 60000);
  const first = await hook('status');
  check('Voice runs the real engine in a development copy', first.settings.devMode === true && first.pid === voice.pid, { pid: first.pid });
  if (MAC || ARGS.models) {
    TRANSCRIPT = transcriptionOf(first.settings);
    report.transcription = TRANSCRIPT;
    if (TRANSCRIPT.ready) note(`transcription: ${TRANSCRIPT.via}`);
    else {
      // A missing prerequisite is not a platform gap: the run says so once and keeps going, so every other check still
      // runs, but the report and the exit code carry the one failure.
      note(`transcription: not available: ${TRANSCRIPT.reason}`);
      check('meetings can transcribe on this Mac', false, TRANSCRIPT.reason);
    }
  }
  if (SPEAKERS !== null) {
    // Decided by what Voice reports, not by the platform: a Voice build with the macOS speaker worker reports ready and the run goes on.
    const sp = first.settings.speakers || {};
    if (sp.ready !== true && sp.reason === 'platform') throw new Refusal(SPEAKER_NEEDS);
    if (SPEAKERS === true) {
      check('Settings > Speaker labels is ready with Nemotron', sp.ready === true && sp.engine === 'nemotron', { reason: sp.reason, detail: sp.detail, runtime: sp.runtime });
    }
  }
  await startRukoo();
  await until('Rukoo to connect to Voice', rukoo.connected, 30000);
  const settled = await until('Settings > Rukoo to show the client', async () => {
    const s = await hook('status');
    return s.settings.rukoo.state === 'connected' && s.settings.rukoo;
  }, 15000);
  check('Settings > Rukoo shows the connected client from its hello', settled.clientVersion && settled.protocol === 2, settled);
  if (MAC) {
    report.screen.capabilities = null;
    skip('Voice found its screen process and can record the screen', 'the macOS screen process does not record yet (it exits with "needs Windows"), so Voice reports no screen capability and the run keeps --screen off');
  } else {
    const caps = await until('the screen probe', async () => (await hook('status')).settings.screen, 30000);
    report.screen.capabilities = caps;
    check('Voice found its screen process and can record the screen', caps.available === true, caps);
  }
  const def = await until("Voice's default screen mode in Rukoo", async () => (await rukoo.state()).screenDefault === ARGS.screen, 15000).catch(() => false);
  check("Rukoo knows Voice's default screen mode from status", def, { expected: ARGS.screen });
  return voice;
}

// The screen of a meeting while it records: frames with text in Rukoo, an image through meetingFrame, and slide 2 at
// its moment.
async function checkFrames(id, { realOffsets = null } = {}) {
  const capture = await until('the first capture line', () => journal(id).find((l) => l.kind === 'capture'), 30000, 100);
  const two = await until('a frame that reads "Slide 2"', () => journal(id).find((l) => l.kind === 'frame' && slideTwo(l.text)), 60000, 250);
  if (realOffsets) {
    const expected = report.screen.slides.firstCallSegment + realOffsets[1];
    check('the frame that reads "Slide 2" sits at the slide\'s moment in the call (within 2.2 s)', two.t >= expected - 100 && two.t <= expected + 2200, { at: two.t, expected });
  } else {
    check('the frame that reads "Slide 2" sits 10.0 to 12.2 s after the capture started', two.t >= capture.t + 10000 && two.t <= capture.t + 12200, { at: two.t, capture: capture.t });
  }
  const seen = await until('frames in Rukoo', async () => {
    const m = await rukoo.get(id);
    return (m.frames || []).length > 0 && m;
  }, 30000, 500);
  const recording = (await rukoo.state()).recording === id;
  const withText = seen.frames.filter((f) => (f.text || '').length > 0).length;
  check('frames with their text reached Rukoo while it records', recording && withText > 0, { frames: seen.frames.length, withText });
  const latest = seen.frames.at(-1);
  const url = await rukoo.frame(id, latest.at);
  check("Rukoo's meetingFrame returns the frame's image", typeof url === 'string' && url.startsWith('data:image/jpeg;base64,'), { at: latest.at, bytes: url ? url.length : 0 });
  report.screen.slideTwoAt = two.t;
  report.screen.captureAt = capture.t;
}

// A killed screen process comes back through Voice's watchdog; the audio goes on.
async function killScreen(id) {
  const doomed = screenOf(id);
  const killedAt = Date.now();
  killPid(doomed.pid);
  const back = await until('a new screen process', () => {
    const s = screenOf(id);
    return s && s.pid !== doomed.pid && alive(s.pid) && s;
  }, 30000, 200);
  check("Voice's watchdog restarted the killed screen process", true, { killed: doomed.pid, resumed: back.pid, after: `${Date.now() - killedAt} ms` });
  const crash = await until('a crash line in the screen journal', () => journal(id).find((l) => l.kind === 'crash'), 20000);
  const gap = await until('a screen gap in Rukoo', async () => {
    const m = await rukoo.get(id);
    return (m.marks || []).find((k) => k.kind === 'screen_gap' && k.detail === 'crash');
  }, 30000, 500);
  check('Rukoo shows a screen gap with detail crash', Boolean(gap), { at: gap.at, until: gap.until, crash: { t: crash.t, lastSeen: crash.lastSeen } });
  await sleep(3000);
  const audio = timeline(id).filter((l) => ['crash', 'device_lost', 'suspend'].includes(l.kind));
  const m = await rukoo.get(id);
  const audioGaps = (m.marks || []).filter((k) => k.kind === 'gap');
  check('neither audio track has a gap from the screen process\'s crash', audio.length === 0 && audioGaps.length === 0, { timeline: audio, marks: audioGaps });
  return back;
}

// Voice hashes a model the first time it loads it and keeps the result in <data>/models/verified.json.
function seededModelStamp() {
  const stamps = readJson(path.join(VOICE_DATA, 'models', 'verified.json'));
  const dir = path.join(VOICE_DATA, ORUKEET_DIR);
  return Boolean((stamps?.entries || []).some((e) => e.model === ORUKEET.id && e.revision === ORUKEET.revision && e.dir === dir));
}

// Without a transcriber, what the run can still prove about the audio: both synthetic tracks write their files, and
// the recorder's heartbeats count samples on each.
async function recordedTracks(id, label) {
  const bytes = (track) => {
    const dir = path.join(meetingDir(id), 'audio', track);
    return (fs.existsSync(dir) ? fs.readdirSync(dir).filter((n) => n.endsWith('.wav')) : []).reduce((n, f) => n + fs.statSync(path.join(dir, f)).size, 0);
  };
  const found = await until('both tracks to write their files', () => {
    const mic = bytes('mic');
    const call = bytes('call');
    const tracks = timeline(id).filter((l) => l.kind === 'track').map((l) => l.track);
    return mic > 32000 && call > 32000 && tracks.includes('mic') && tracks.includes('call') && { mic, call, tracks };
  }, 60000, 500);
  const beat = timeline(id).filter((l) => l.kind === 'heartbeat').at(-1);
  check(`both synthetic tracks record audio files while ${label} runs`, Boolean(beat?.samples?.mic) && Boolean(beat?.samples?.call), { ...found, samples: beat?.samples });
}

async function standardRun(voice, offsets) {
  const labels = SPEAKERS === true;
  let outage = {};
  let stopAt = null;
  let snapshot = null;
  // Sync: the fake bucket, then Settings > Storage as a user fills it in, before the meeting.
  const sync = SECRETS;
  if (SYNC) await syncSetup(sync);
  // 2. A meeting started from Rukoo, with Voice's default screen mode.
  const t0 = Date.now();
  const { meetingId: id } = await rukoo.start({ title: TITLE });
  check('Rukoo started a meeting through Voice', Boolean(id), id);
  report.meeting = id;
  const stopSlides = offsets ? scheduleSlides(id, offsets) : () => {};
  try {
    await until('the recording status in Rukoo', async () => (await rukoo.state()).recording === id, 15000);
    if (MAC) {
      skip("Rukoo's windows are protected from capture while it records", PROTECTION_SKIP);
    } else {
      const shielded = await rukoo.protectedWindows();
      check("Rukoo's windows are protected from capture while it records", shielded.length > 0 && shielded.every(Boolean), shielded);
    }
    const status = await hook('status');
    const pill = status.pill;
    check('the pill shows the recording', pill.visible && pill.phase === 'meeting' && pill.meeting?.state === 'recording', { phase: pill.phase, meeting: pill.meeting });
    const meta = await until('the meeting in Rukoo', async () => {
      const m = await rukoo.get(id);
      return m?.meeting?.screen && m;
    }, 10000);
    check("the meeting records Voice's default screen mode", meta.meeting.screen === ARGS.screen, { screen: meta.meeting.screen });
    if (ARGS.screen !== 'off') {
      check('the pill shows the screen recording', pill.meeting?.screen === ARGS.screen, { screen: pill.meeting?.screen, screenSwitch: pill.meeting?.screenSwitch });
      const live = await until('screen.json', () => screenOf(id), 30000, 100);
      check('screen.json names a live screen process', alive(live.pid), { pid: live.pid });
      if (ARGS.realScreen) await guardMonitor(id, report.screen.slideDisplay);
      await checkFrames(id, { realOffsets: ARGS.realScreen ? offsets : null });
    }

    if (MAC) {
      check('the initial screen mode remains historical', (await rukoo.get(id)).meeting.initialScreen === ARGS.screen, { screen: ARGS.screen });
      skip('the pill enables frames and Rukoo receives its timestamped mode mark', SCREEN_SKIP);
      skip('windows are protected before enabled capture', PROTECTION_SKIP);
      skip('screen mode changes keep the same audio recorder', SCREEN_SKIP);
    } else if (!ARGS.realScreen) {
      const recorderPid = recorderOf(id).pid;
      await rukoo.setScreen(id, 'off');
      await until('notifications released with screen Off', () => app.evaluate(() => !global.__semVoice.screenRecording()), 5000);
      await hook('pill_screen_set', { meeting: id, mode: 'frames' });
      const enabled = await until('a screen mode mark from the pill', async () => (await rukoo.get(id)).marks?.some((k) => k.kind === 'screen_mode' && k.label === 'frames'), 15000);
      check('the pill enables frames and Rukoo receives its timestamped mode mark', enabled);
      check('windows are protected before enabled capture', (await rukoo.protectedWindows()).every(Boolean));
      await rukoo.setScreen(id, 'video');
      await until('video after changing mode', () => journal(id).some((l) => l.kind === 'video'), 30000);
      await rukoo.setScreen(id, ARGS.screen);
      check('screen mode changes keep the same audio recorder', recorderOf(id).pid === recorderPid);
      check('the initial screen mode remains historical', (await rukoo.get(id)).meeting.initialScreen === ARGS.screen);
    }

    // 3. Passages on both tracks, while it records.
    if (TRANSCRIPT.ready) {
      const live = await until('passages with the expected words on both tracks', async () => {
        const m = await rukoo.get(id);
        return m && trackHas(m, 'mic', YOU.words) && trackHas(m, 'call', THEM.words) && m;
      }, 240000, 500);
      const stillRecording = (await rukoo.state()).recording === id;
      check('passages with the expected words reached Rukoo on both tracks before the stop', stillRecording, {
        after: `${Math.round((Date.now() - t0) / 1000)} s`,
        mic: live.passages.filter((p) => p.track === 'mic').length,
        call: live.passages.filter((p) => p.track === 'call').length
      });
      if (seededOrukeet) check('Voice hashed the seeded Orukeet once and wrote its stamp', seededModelStamp(), { stamp: path.join(VOICE_DATA, 'models', 'verified.json') });
    } else {
      skip('passages with the expected words reached Rukoo on both tracks before the stop', TRANSCRIPT.reason);
      await recordedTracks(id, 'the meeting from Rukoo');
    }

    // 3a. Speaker labels: three voices on the call before the stop, one of them named, then the worker killed.
    let named = null;
    if (labels) named = await speakersBeforeStop(id);

    // Sync while it records: audio and frames go up before the stop, and a 20 s bucket outage costs the recorder
    // nothing.
    if (SYNC) outage = await syncWhileLive(id, t0);

    // Captures while it records: Rukoo's meeting view, the pill and Settings > Rukoo.
    if (!ARGS.realScreen && !MAC) {
      await win.keyboard.press('Control+3');
      await win.locator('.meeting .mt-transcript .mt-passage').first().waitFor({ timeout: 20000 });
      await sleep(1000);
      await captureRukoo(path.join(SHOTS, 'rukoo-meeting-live.png'));
      const shots = await hook('capture', { dir: SHOTS, page: 'rukoo' }, 90000);
      check('Voice captured its pill and Settings > Rukoo from WebView2', !shots.pill.error && !shots.settings.error, shots);
    } else if (MAC) {
      // The meeting view as Rukoo paints it; Voice's own pill and Settings captures come from WebView2.
      await win.keyboard.press('Control+3');
      await win.locator('.meeting').first().waitFor({ timeout: 20000 });
      await sleep(1000);
      await captureRukoo(path.join(SHOTS, 'rukoo-meeting-live.png'));
      skip('Voice captured its pill and Settings > Rukoo from WebView2', CAPTURE_SKIP);
    }

    // 3b. The screen process dies; Voice's watchdog starts it again and the audio doesn't notice.
    let screenPid = null;
    if (ARGS.screen !== 'off') screenPid = (await killScreen(id)).pid;

    // 4. Voice restarts mid-meeting; the recorder and the screen process go on and the new Voice reattaches. With
    // sync, the kill lands while the worker still has a file to upload.
    if (SYNC) {
      const pending = await until('a file waiting to upload', async () => {
        const s = await syncStatus();
        return s.waiting >= 1 && s;
      }, 60000, 250);
      note(`Voice killed with ${pending.waiting} file(s) waiting to upload, state ${pending.state}`);
    }
    const before = await rukoo.get(id);
    const lastEnd = before.passages.length ? Math.max(...before.passages.map((p) => p.end)) : 0;
    const recorder = recorderOf(id);
    const restartAt = meetingNow(id);
    const restartWall = Date.now();
    killPid(voice.pid);
    await until('Voice to exit', () => !alive(voice.pid), 15000);
    await sleep(1500);
    check('the recorder outlives Voice', recorder && alive(recorder.pid), recorder && { pid: recorder.pid });
    if (screenPid) check('the screen process outlives Voice', alive(screenPid), { pid: screenPid });
    voice = startVoice(2, { monitor: report.screen.pinned });
    await until('Rukoo to reconnect to the new Voice', async () => (await rukoo.connected()) && (await rukoo.state()).voiceVersion, 60000);
    const reattached = await until('the reattached meeting in the pill', async () => {
      const s = await hook('status');
      return s.pill.meeting?.state === 'recording' && s;
    }, 30000);
    check('the new Voice reattached to the running recorder', recorderOf(id)?.pid === recorder.pid && reattached.tray.meeting === 'recording', { recorder: recorder.pid });
    if (screenPid) {
      await sleep(2000);
      check('the new Voice kept the same screen process', screenOf(id)?.pid === screenPid && alive(screenPid), { pid: screenOf(id)?.pid });
    }
    if (TRANSCRIPT.ready) {
      const resumed = await until('passages after the restart', async () => {
        const m = await rukoo.get(id);
        return m.passages.some((p) => p.start > lastEnd) && m;
      }, 120000, 500);
      check('the meeting goes on after the restart without passages twice', overlaps(resumed.passages).length === 0, { overlaps: overlaps(resumed.passages), passages: resumed.passages.length });
    } else {
      skip('the meeting goes on after the restart without passages twice', TRANSCRIPT.reason);
      const first = await until('the recorder\'s clock after the restart', () => timeline(id).find((l) => l.kind === 'heartbeat' && l.t > restartAt + 2000 && l.wall >= restartWall), 120000, 500).catch(() => null);
      check('the recorder clock keeps going after the Voice restart', Boolean(first) && first.t > restartAt + 1000, { restartAt, at: first?.t ?? null });
    }
    if (MAC) skip("Rukoo's windows stayed protected through the restart", PROTECTION_SKIP);
    else check("Rukoo's windows stayed protected through the restart", (await rukoo.protectedWindows()).every(Boolean));
    if (labels && named) await keepsItsId(id, named, restartAt, 'after the Voice restart');
    if (screenPid) {
      const framesBefore = (await rukoo.get(id)).frames.length;
      const more = await until('a frame after the restart', async () => (await rukoo.get(id)).frames.length > framesBefore, 30000, 500).catch(() => false);
      check('frames keep reaching Rukoo after the restart', more, { before: framesBefore });
    }

    // 5. The recorder dies; the watchdog starts it again with --resume and the gap is marked.
    const doomed = recorderOf(id);
    killPid(doomed.pid);
    const back = await until('a new recorder', () => {
      const r = recorderOf(id);
      return r && r.pid !== doomed.pid && alive(r.pid) && r;
    }, 30000);
    check('the watchdog resumed the killed recorder', true, { killed: doomed.pid, resumed: back.pid });
    const gap = await until('a gap mark in Rukoo', async () => {
      const m = await rukoo.get(id);
      return (m.marks || []).find((k) => k.kind === 'gap' && k.detail === 'crash');
    }, 60000, 500);
    check('Rukoo shows the gap with detail crash', Boolean(gap), gap);
    const tl = timeline(id);
    check('the timeline has a resumed session and a crash line', tl.some((l) => l.kind === 'session' && l.resumed) && tl.some((l) => l.kind === 'crash'));
    if (screenPid) check('the screen process followed the resumed recorder', alive(screenOf(id)?.pid), { pid: screenOf(id)?.pid });

    // 6. Stop from Rukoo: processing, then done, after the screen journal's stop.
    await sleep(labels ? 20000 : 6000);
    stopAt = Date.now();
    await rukoo.stop(id);
    if (TRANSCRIPT.ready) {
      await until('done', async () => (await rukoo.get(id)).meeting.state === 'done', 240000, 200);
      const states = await voiceStates(id);
      check('the stop went to processing, then done', stoppedThenDone(states), states);
    } else {
      const ended = await until('the meeting to end', async () => {
        const state = (await rukoo.get(id)).meeting.state;
        return ['done', 'failed'].includes(state) && state;
      }, 300000, 300);
      report.meetingStateAfterStop = ended;
      note(`the meeting ended ${ended} after the stop: without a transcriber Voice cannot finish its transcript and keeps the audio`);
      skip('the stop went to processing, then done', TRANSCRIPT.reason);
    }
    if (ARGS.screen !== 'off') {
      const j = journal(id);
      const last = j.at(-1);
      check('the screen journal ends with stop, reason meeting, at the recorder\'s stop', last?.kind === 'stop' && last.reason === 'meeting' && last.t === timeline(id).at(-1)?.t, { last, recorderStop: timeline(id).at(-1)?.t });
      check('screen.json is gone after the stop', !fs.existsSync(path.join(meetingDir(id), 'screen', 'screen.json')));
      report.screen.journal = summarizeJournal(j);
      if (ARGS.screen === 'video') report.screen.videos = checkVideos(id, 'the meeting from Rukoo');
    } else {
      check('a meeting without the screen has no screen folder', !fs.existsSync(path.join(meetingDir(id), 'screen')));
    }
    if (MAC) skip("Rukoo's windows are capturable again after the meeting", PROTECTION_SKIP);
    else {
      const ended = await until('protection to end', async () => (await rukoo.protectedWindows()).every((p) => !p), 15000);
      check("Rukoo's windows are capturable again after the meeting", ended);
    }
    await until('the recorder to exit', () => !alive(back.pid), 20000);

    // 7. Rukoo's meeting.get against Voice's.
    const mine = await rukoo.get(id);
    const theirs = await rukoo.voiceGet(id);
    if (TRANSCRIPT.ready) {
      const key = (p) => `${p.id}@${p.revision}`;
      const missing = theirs.passages.filter((p) => !mine.passages.some((q) => key(q) === key(p)));
      const extra = mine.passages.filter((p) => !theirs.passages.some((q) => q.id === p.id));
      check("Rukoo's meeting.get has every passage Voice has", missing.length === 0 && extra.length === 0 && theirs.passages.length > 0, {
        voice: theirs.passages.length,
        rukoo: mine.passages.length,
        missing: missing.map(key),
        extra: extra.map(key)
      });
      check('no passage covers the same audio twice', overlaps(mine.passages).length === 0, overlaps(mine.passages));
    } else {
      check("Rukoo's meeting.get carries the meeting Voice has", theirs.meeting?.id === id && mine.meeting?.id === id, { voice: theirs.meeting?.state, rukoo: mine.meeting?.state });
      skip("Rukoo's meeting.get has every passage Voice has", TRANSCRIPT.reason);
      skip('no passage covers the same audio twice', TRANSCRIPT.reason);
    }
    snapshot = theirs;
    if (ARGS.screen !== 'off') {
      const ids = (x) => (x.frames || []).map((f) => f.id).sort().join(',');
      check("Rukoo's frames are Voice's frames", ids(mine) === ids(theirs) && (theirs.frames || []).length > 0, { voice: (theirs.frames || []).length, rukoo: (mine.frames || []).length });
    }
    await captureRukoo(path.join(SHOTS, 'rukoo-meeting-done.png'));
    if (labels) await speakersAfterDone(id, mine, named);
  } finally {
    stopSlides();
  }

  // 8. A meeting started from Voice's side, as from its tray, with Voice's default screen mode.
  const fromVoice = (await hook('start_meeting')).meeting;
  const seen = await until("Rukoo to pick up Voice's meeting", async () => {
    const m = await rukoo.get(fromVoice);
    const s = await rukoo.state();
    return m && s.recording === fromVoice && m;
  }, 30000);
  check('a meeting Voice started shows in Rukoo as recording', seen.meeting.startedBy === 'voice', { id: fromVoice, eventId: seen.meeting.eventId, event: seen.event });
  check("the meeting Voice started records Voice's default screen mode", seen.meeting.screen === ARGS.screen, { screen: seen.meeting.screen });
  if (MAC) skip("Rukoo protects its windows for Voice's meeting too", PROTECTION_SKIP);
  else check("Rukoo protects its windows for Voice's meeting too", (await rukoo.protectedWindows()).every(Boolean));
  if (!seen.meeting.eventId) note('no calendar event in the demo account fits this time of day, so Rukoo linked none (it links one only when exactly one fits)');
  if (ARGS.screen !== 'off') {
    if (ARGS.realScreen) await guardMonitor(fromVoice, report.screen.slideDisplay);
    await until('a frame of the meeting Voice started', () => journal(fromVoice).some((l) => l.kind === 'frame'), 20000);
    const startLine = (await voiceLog("SELECT COUNT(*) AS n FROM events WHERE event = 'meeting.starting' AND meeting = ?", fromVoice))[0].n;
    check('Voice asked Rukoo to protect before it captured (meeting.starting)', startLine >= 1, { events: startLine });
  }
  if (!TRANSCRIPT.ready) await recordedTracks(fromVoice, "Voice's meeting");
  await sleep(4000);
  await hook('stop_meeting');
  if (TRANSCRIPT.ready) {
    await until("Voice's meeting to be done", async () => (await rukoo.get(fromVoice)).meeting.state === 'done', 120000, 500);
    const fromVoiceStates = await voiceStates(fromVoice);
    check('the meeting Voice started went to processing, then done', stoppedThenDone(fromVoiceStates), fromVoiceStates);
  } else {
    await until("Voice's meeting to end", async () => {
      const state = (await rukoo.get(fromVoice)).meeting.state;
      return ['done', 'failed'].includes(state) && state;
    }, 300000, 500);
    skip('the meeting Voice started went to processing, then done', TRANSCRIPT.reason);
  }

  // 9. Rukoo asks for video once, whatever Voice's default.
  if (MAC) {
    skip('an audio-only meeting can enable screen video on the same clock', SCREEN_SKIP);
  } else if (ARGS.screen !== 'video') {
    const { meetingId: vid } = await rukoo.start({ title: 'Live run: audio only, then video', screen: 'off' });
    await until('the recording status in Rukoo', async () => (await rukoo.state()).recording === vid, 15000);
    await rukoo.setScreen(vid, 'video');
    if (ARGS.realScreen) await guardMonitor(vid, report.screen.slideDisplay);
    const session = await until('the screen journal\'s session line', () => journal(vid).find((l) => l.kind === 'session'), 30000, 100);
    const video = await until('a video file', () => journal(vid).find((l) => l.kind === 'video' || l.kind === 'dropped'), 30000, 100);
    const m = await rukoo.get(vid);
    check('an audio-only meeting can enable screen video on the same clock', m.meeting.screen === 'video' && m.meeting.initialScreen === 'off' && session.mode === 'off' && video.kind === 'video', { screen: m.meeting.screen, mode: session.mode, encoder: video.encoder, hardware: video.hardware });
    await sleep(5000);
    await rukoo.stop(vid);
    await until('done', async () => (await rukoo.get(vid)).meeting.state === 'done', 120000, 300);
    checkVideos(vid, 'the video meeting from Rukoo');
  }

  const rows = await statusRows();
  check("Voice's event log keeps only the newest status event", rows === 1, { statusRows: rows });
  const final = await hook('status');
  report.settingsRukoo = final.settings.rukoo;
  check('Settings > Rukoo reports the client and how far behind it is', final.settings.rukoo.state === 'connected' && final.settings.rukoo.behind === 0, final.settings.rukoo);

  if (!SYNC) return;
  const ctx = { id, fromVoice, theirs: snapshot, t0, stopAt, outage, sync, voice: () => voice, setVoice: (v) => (voice = v) };
  await bucketChecks(ctx);
  if (MEASURE === 'sync') await measureSync(ctx);
  await linkChecks(ctx);
  if (REMOTE_FETCH) await remoteFetch(ctx);
  await restoreChecks(ctx);
}

// Counts per kind and the drops; never the frames' text.
function summarizeJournal(j) {
  const kinds = {};
  for (const l of j) kinds[l.kind] = (kinds[l.kind] || 0) + 1;
  return {
    kinds,
    dropped: j.filter((l) => l.kind === 'dropped').map((l) => ({ t: l.t, what: l.what, reason: l.reason })),
    monitors: j.filter((l) => l.kind === 'monitor').map((l) => ({ t: l.t, id: l.id, reason: l.reason, size: `${l.width}x${l.height}` })),
    ocr: j.filter((l) => l.kind === 'frame').reduce((acc, l) => ({ ...acc, [l.ocr]: (acc[l.ocr] || 0) + 1 }), {})
  };
}

// --choose-monitor: Voice picks the display of the focused window; then the pill and Rukoo's header move it.
async function chooseRun(offsets) {
  const target = ARGS.chooseMonitor;
  // The window the user works in: the slide, as Windows' foreground window. Electron's focus() only activates it
  // within its own thread when another process has the foreground, so the run brings it to the front itself.
  await screens.evaluate(() => global.__demo.slide.focus());
  const hwnd = await screens.evaluate(() => global.__demo.slide.getNativeWindowHandle().readBigUInt64LE(0).toString());
  const front = foreground(hwnd);
  report.screen.slideForeground = front;
  if (!front) throw new Error("couldn't make the slide window the foreground window, so the monitor choice can't be checked");
  const { meetingId: id } = await rukoo.start({ title: 'Live run: monitor choice' });
  check('Rukoo started a meeting through Voice', Boolean(id), id);
  report.meeting = id;
  const stopSlides = scheduleSlides(id, offsets);
  try {
    await until('the recording status in Rukoo', async () => (await rukoo.state()).recording === id, 15000);
    const first = await guardMonitor(id, target);
    check('Voice chose the display of the focused window', first.id === target && first.reason === 'start', { id: first.id, size: `${first.width}x${first.height}` });
    if (ARGS.screen === 'video') {
      const video = await until('the first video file', () => journal(id).find((l) => l.kind === 'video'), 20000, 100);
      const display = displays.find((d) => d.id === target);
      check('the video keeps the display\'s own size', video.width === display.width && video.height === display.height, { video: `${video.width}x${video.height}`, display: `${display.width}x${display.height}`, encoder: video.encoder, hardware: video.hardware });
    }
    await checkFrames(id, { realOffsets: offsets });

    const users = () => journal(id).filter((l) => l.kind === 'monitor' && l.reason === 'user');
    const userMarks = async () => ((await rukoo.get(id)).marks || []).filter((k) => k.kind === 'monitor' && k.detail === 'user');

    // The pill menu selects another monitor without changing the screen mode.
    const others = displays.filter((d) => d.id !== target).map((d) => d.id);
    await hook('pill_screen_set', { meeting: id, mode: ARGS.screen, monitor: others[0] });
    const viaPill = await until('a monitor line after the pill click', () => users()[0], 15000, 100);
    check('the pill moved the recording to the next monitor, with reason user', others.includes(viaPill.id), { id: viaPill.id });
    const mark1 = await until('a monitor mark in Rukoo', async () => (await userMarks())[0], 20000, 500);
    check('Rukoo shows the move from the pill as a monitor mark with detail user', Boolean(mark1), { at: mark1.at, label: mark1.label });
    await sleep(3000);

    // From Rukoo's header: back to the chosen display.
    const info = await rukoo.screen(id);
    check("Rukoo's header lists the monitors and the one recorded now", info.monitors.length === displays.length && info.monitor === viaPill.id, { monitor: info.monitor, monitors: info.monitors.map((m) => m.id) });
    const picked = await rukoo.select(id, target);
    const viaRukoo = await until('a monitor line after the header pick', () => users()[1], 15000, 100);
    check("Rukoo's header moved the recording back, with reason user", picked === target && viaRukoo.id === target, { picked, id: viaRukoo.id });
    const mark2 = await until('a second monitor mark in Rukoo', async () => (await userMarks())[1], 20000, 500);
    check('Rukoo shows the move from its header as a monitor mark with detail user', Boolean(mark2), { at: mark2.at, label: mark2.label });
    await sleep(4000);

    await rukoo.stop(id);
    await until('done', async () => (await rukoo.get(id)).meeting.state === 'done', 240000, 300);
    const j = journal(id);
    check('the screen journal ends with stop, reason meeting', j.at(-1)?.kind === 'stop' && j.at(-1)?.reason === 'meeting', j.at(-1));
    report.screen.journal = summarizeJournal(j);
    if (ARGS.screen === 'video') report.screen.videos = checkVideos(id, 'the monitor-choice meeting');
  } finally {
    stopSlides();
  }
}

// The first frame of a meeting Voice started, against Rukoo's window over half of the slide.
async function visibleMeeting(hide, probes, rect) {
  await rukoo.settings({ meetingHideWindow: hide });
  const id = (await hook('start_meeting')).meeting;
  await until("Rukoo to pick up Voice's meeting", async () => (await rukoo.state()).recording === id, 30000);
  await guardMonitor(id, report.screen.slideDisplay);
  const first = await until('the first frame', () => journal(id).find((l) => l.kind === 'frame'), 30000, 100);
  const text = normalize(first.text);
  const words = new Set(text.split(' '));
  const matched = probes.filter((p) => words.has(p));
  const url = await until("the first frame's image in Rukoo", async () => {
    const m = await rukoo.get(id);
    return (m.frames || []).length > 0 && rukoo.frame(id, first.t);
  }, 30000, 500);
  const pixels = await frameAgainstSlide(url, rect);
  const shielded = await rukoo.protectedWindows();
  await hook('stop_meeting');
  await until("Voice's meeting to be done", async () => (await rukoo.get(id)).meeting.state === 'done', 120000, 500);
  return { id, matched: matched.length, probes: probes.length, ocr: first.ocr, diff: pixels.diff, protectedWindows: shielded };
}

const normalize = (s) => (s || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();

// --visible-rukoo: with hiding off Rukoo shows in the first frame (the check can fail); with it on it doesn't.
async function visibleRun() {
  const slide = displays.find((d) => d.id === report.screen.slideDisplay);
  const b = slide.bounds;
  const bounds = { x: b.x + Math.round(b.width / 2), y: b.y + 60, width: Math.round(b.width / 2) - 60, height: b.height - 120 };
  const rect = { x: (bounds.x - b.x + 40) / b.width, y: (bounds.y - b.y + 40) / b.height, width: (bounds.width - 80) / b.width, height: (bounds.height - 80) / b.height };
  await app.evaluate(({ BrowserWindow }, r) => {
    const w = BrowserWindow.getAllWindows()[0];
    w.setBounds(r);
    // Larger text, so Windows reads Rukoo's strings reliably when they are in the frame.
    w.webContents.setZoomFactor(1.5);
    w.setAlwaysOnTop(true, 'screen-saver');
    w.showInactive();
  }, bounds);
  await sleep(1500);
  // Rukoo's demo strings on screen: the words of six letters or more in the text Rukoo's window paints where nothing
  // covers it, none of which is on the slide. Single words, so a line the text reader breaks still counts.
  const painted = await win.evaluate(() => {
    const out = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    const range = document.createRange();
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      if (!n.textContent.trim() || !n.parentElement) continue;
      range.selectNodeContents(n);
      const r = range.getBoundingClientRect();
      if (r.width <= 0 || r.height <= 0 || r.top < 0 || r.left < 0 || r.bottom > innerHeight || r.right > innerWidth) continue;
      const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      if (top && (top === n.parentElement || n.parentElement.contains(top) || top.contains(n.parentElement))) out.push(n.textContent);
    }
    return out;
  });
  const onSlide = new Set(normalize(SLIDES.flat().join(' ')).split(' '));
  const probes = [...new Set(normalize(painted.join(' ')).split(' '))].filter((w) => w.length >= 6 && !/^\d+$/.test(w) && !onSlide.has(w));
  report.screen.visible = { rukooBounds: bounds, probes: probes.length };
  try {
    const off = await visibleMeeting(false, probes, rect);
    report.screen.visible.off = off;
    check("with Rukoo's hiding switch off, its demo strings show in the first frame", off.matched >= 3 && off.protectedWindows.every((p) => !p), { matched: off.matched, of: off.probes, ocr: off.ocr });
    check("with hiding off, the frame doesn't show the slide where Rukoo's window is", off.diff > 20, { meanDiff: off.diff });
    const on = await visibleMeeting(true, probes, rect);
    report.screen.visible.on = on;
    check("with Rukoo's hiding switch on, none of its demo strings show in the first frame", on.matched === 0 && on.protectedWindows.every(Boolean), { matched: on.matched, of: on.probes, ocr: on.ocr });
    check("with hiding on, the frame shows the slide where Rukoo's window is", on.diff <= 12, { meanDiff: on.diff });
  } finally {
    await app.evaluate(({ BrowserWindow }) => {
      const w = BrowserWindow.getAllWindows()[0];
      w.setAlwaysOnTop(false);
      w.webContents.setZoomFactor(1);
      w.setPosition(-5000, -5000);
    });
  }
}

// ---------- measurements ----------

// The nearest-rank percentile, p from 0 to 100.
function nearestRank(values, p) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1)];
}

function dirBytes(dir) {
  let total = 0;
  if (!fs.existsSync(dir)) return 0;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    total += entry.isDirectory() ? dirBytes(p) : fs.statSync(p).size;
  }
  return total;
}

// The screen log's own timings: "frame <id>: WxH, <n> bytes, jpeg <ms> ms, ocr <ms> ms (<status>)" and the GPU memory.
function screenLog(id) {
  const file = path.join(VOICE_DATA, 'logs', `screen-${id}.log`);
  const text = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '';
  const jpeg = [];
  const ocr = [];
  const memory = [];
  for (const line of text.split('\n')) {
    const f = /jpeg (\d+) ms, ocr (\d+) ms/.exec(line);
    if (f) {
      jpeg.push(Number(f[1]));
      ocr.push(Number(f[2]));
    }
    const m = /gpu memory in use: (\d+) MB/.exec(line);
    if (m) memory.push(Number(m[1]));
  }
  return { jpegMs: { p50: nearestRank(jpeg, 50), p95: nearestRank(jpeg, 95) }, ocrMs: { p50: nearestRank(ocr, 50), p95: nearestRank(ocr, 95) }, gpuMemoryMb: memory.length ? Math.max(...memory) : null };
}

// One measured meeting: started from Rukoo with `screen`, measured for ARGS.seconds once transcription runs. On the
// synthetic screen a new slide comes every 10 s throughout; with --real-screen the slide display runs the section 7
// script: slides every 10 s for 60% of the time, a scrolling page for 20%, then still.
async function measureRun(voice, name, screen) {
  console.log(`\nmeasuring: ${name}`);
  const { meetingId: id } = await rukoo.start({ title: `Live run: measure ${name}`, screen });
  await until('the recording status in Rukoo', async () => (await rukoo.state()).recording === id, 15000);
  if (screen !== 'off' && ARGS.realScreen) await guardMonitor(id, report.screen.slideDisplay);
  await until('the first passage', async () => ((await rukoo.get(id)).passages || []).length > 0, 120000, 500);
  const pids = { 'rukoo-voice': voice.pid, recorder: recorderOf(id).pid };
  const children = childrenOf(voice.pid);
  if (children['rukoo-voice-local']) pids.local = children['rukoo-voice-local'];
  // With live speaker labels on, the speaker worker belongs to the meeting's load.
  const speakerWorker = (await labelsOf(id))?.workerPid || children['rukoo-voice-speakers'];
  if (speakerWorker) pids.speakers = speakerWorker;
  if (screen !== 'off') pids.screen = screenOf(id).pid;
  const out = path.join(RUN, `measure-${name}.json`);
  const seconds = ARGS.seconds;
  const sampler = spawn('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', path.join(VOICE, 'scripts', 'measure.ps1'), '-Pids', Object.values(pids).join(','), '-Names', Object.keys(pids).join(','), '-Seconds', String(seconds), '-Out', out], { stdio: 'inherit', windowsHide: true });
  started.measure.push(sampler.pid);
  const t0 = Date.now();
  const startAt = (await rukoo.get(id)).passages.length;
  const plan = [];
  if (ARGS.realScreen) {
    const slidesFor = Math.round(seconds * 0.6);
    for (let s = 0, n = 1; s < slidesFor; s += 10, n++) plan.push([s * 1000, `slide(${n})`]);
    plan.push([slidesFor * 1000, 'scrolling(true)']);
    plan.push([Math.round(seconds * 0.8) * 1000, 'scrolling(false)']);
  }
  const timers = plan.map(([at, code]) => setTimeout(() => slideCall(code).catch(() => {}), at));
  const exit = await new Promise((resolve) => sampler.on('exit', resolve));
  timers.forEach(clearTimeout);
  const elapsed = (Date.now() - t0) / 1000;
  await rukoo.stop(id);
  await until('done', async () => (await rukoo.get(id)).meeting.state === 'done', 300000, 500);
  const sampled = readJson(out);
  const j = journal(id);
  const video = j.find((l) => l.kind === 'video');
  const lengthH = (timeline(id).at(-1).t - (timeline(id).find((l) => l.kind === 'session')?.t || 0)) / 3600000;
  const result = {
    name,
    screen,
    source: ARGS.realScreen ? 'real' : synthetic(syntheticSize),
    liveSpeakers: SPEAKERS === true,
    monitor: j.find((l) => l.kind === 'monitor')?.id || null,
    seconds,
    elapsed: Math.round(elapsed),
    sampler: exit,
    pids,
    encoder: video ? { name: video.encoder, hardware: video.hardware, size: `${video.width}x${video.height}` } : null,
    dropped: j.filter((l) => l.kind === 'dropped').map((l) => `${l.what} ${l.reason}`),
    passages: (await rukoo.get(id)).passages.length - startAt,
    perHour: {
      videoBytes: Math.round(dirBytes(path.join(meetingDir(id), 'screen', 'video')) / lengthH),
      frameBytes: Math.round(dirBytes(path.join(meetingDir(id), 'screen', 'frames')) / lengthH),
      audioBytes: Math.round(dirBytes(path.join(meetingDir(id), 'audio')) / lengthH),
      frames: Math.round(j.filter((l) => l.kind === 'frame').length / lengthH)
    },
    screenLog: screen === 'off' ? null : screenLog(id),
    summary: sampled?.summary || null
  };
  check(`measured ${name}: ${seconds} s of samples`, exit === 0 && sampled && sampled.summary.seconds >= seconds - 2, { samples: sampled?.summary.samples, seconds: sampled?.summary.seconds, sampler: exit });
  if (screen !== 'off') check(`measured ${name}: the screen journal ends with stop`, j.at(-1)?.kind === 'stop', j.at(-1)?.reason);
  return result;
}

async function measureAll(voice, sizes) {
  report.measure = [];
  const main = displays.find((d) => d.primary);
  const other = displays.find((d) => !d.primary);
  for (const [name, screen] of [['off', 'off'], ['frames', 'frames'], ['video', 'video']]) report.measure.push(await measureRun(voice, name, screen));
  // The fourth: video on the other display, or on the synthetic screen at its size. Live speaker labels are on in all
  // four with --speakers, for the combined load, and off without.
  const fourth = ARGS.realScreen ? other && `${other.width}x${other.height}` : sizes.find((s) => s !== syntheticSize);
  if (fourth) {
    killPid(voice.pid);
    await until('Voice to exit', () => !alive(voice.pid), 15000);
    if (ARGS.realScreen) {
      report.screen.slideDisplay = other.id;
      report.screen.pinned = other.id;
      await showDemoWindows(other.id);
    } else {
      syntheticSize = fourth;
    }
    voice = startVoice(2, { monitor: ARGS.realScreen ? other.id : null });
    await until('Rukoo to reconnect to the new Voice', async () => (await rukoo.connected()) && (await rukoo.state()).voiceVersion, 60000);
    report.measure.push(await measureRun(voice, `video-${fourth}`, 'video'));
    if (ARGS.realScreen) report.screen.slideDisplay = main.id;
  }
  printMeasurements(report.measure);
}

function printMeasurements(runs) {
  console.log('\nrun                 process       CPU mean/p95/max (% of a core)   GPU mean (% per engine)');
  for (const r of runs) {
    for (const [name, s] of Object.entries(r.summary?.processes || {})) {
      const gpu = Object.entries(s.gpu || {})
        .filter(([, v]) => v.mean > 0)
        .map(([k, v]) => `${k} ${v.mean}`)
        .join(', ');
      console.log(`${r.name.padEnd(20)}${name.padEnd(14)}${`${s.cpu.mean} / ${s.cpu.p95} / ${s.cpu.max}`.padEnd(33)}${gpu}`);
    }
    if (r.summary) console.log(`${r.name.padEnd(20)}${'machine'.padEnd(14)}${`${r.summary.total.mean} / ${r.summary.total.p95} / ${r.summary.total.max} (% of all cores)`}`);
  }
}

// Before the stop: three call speakers in Rukoo with the main AMI speakers apart; MEE071's id gets a name; the
// worker is killed, comes back through the reel, and MEE071 keeps the id. Returns { speaker, name }.
async function speakersBeforeStop(id) {
  const t = Date.now();
  const found = await until('three call speakers in Rukoo with the main AMI speakers apart', async () => {
    const m = await rukoo.get(id);
    const calls = m.speakers.filter((s) => s.track === 'call');
    const ids = mainIds(callPassages(m.passages), reference(id));
    return calls.length >= 3 && MAIN.every((n) => ids[n]) && { calls: calls.map((s) => s.id), ids };
  }, 300000, 1000);
  check('three call speakers reached Rukoo before the stop, the main AMI speakers on ids of their own', true, { after: `${Math.round((Date.now() - t) / 1000)} s`, ...found });
  report.speakers = { beforeStop: found };

  const speaker = found.ids.MEE071;
  const name = 'Person A';
  const after = await rukoo.name(id, speaker, name);
  check(`the id of MEE071 (${speaker}) got a name in Rukoo`, after.find((s) => s.id === speaker)?.name === name);

  const labels = await until('the speaker worker', async () => {
    const l = await labelsOf(id);
    return l?.workerPid && l.state === 'listening' && l;
  }, 60000, 500);
  const pid = labels.workerPid;
  const killAt = meetingNow(id);
  const killed = Date.now();
  check('Settings names the running speaker worker by its process id', alive(pid), { pid, state: labels.state });
  killPid(pid);
  await until('the killed worker to exit', () => !alive(pid), 15000);
  const states = new Set();
  const back = await until('the follower to listen again with a new worker', async () => {
    const l = await labelsOf(id);
    if (l) states.add(l.state);
    return l?.state === 'listening' && l.workerPid && l.workerPid !== pid && l;
  }, 120000, 250);
  const refeed = Date.now() - killed;
  report.speakers.refeedMs = refeed;
  check('the follower started a new worker and matched the reel', alive(back.workerPid), { pid: back.workerPid, refeedMs: refeed, states: [...states] });
  check('recording went on through the worker crash', (await rukoo.state()).recording === id && alive(recorderOf(id)?.pid));
  await keepsItsId(id, { speaker, name }, killAt, 'after the worker kill');
  return { speaker, name };
}

// MEE071's passages after `from` (meeting ms) carry the named id, and Rukoo shows the name on it.
async function keepsItsId(id, { speaker, name }, from, when) {
  const later = await until(`MEE071's passages ${when}`, async () => {
    const m = await rukoo.get(id);
    const turns = reference(id);
    const own = m.passages.filter((p) => p.track === 'call' && p.start > from && referenceOf(p, turns) === 'MEE071');
    const labelled = own.filter((p) => p.speaker != null);
    return labelled.length >= 2 && { m, own, labelled };
  }, 240000, 1000);
  const wrong = later.labelled.filter((p) => p.speaker !== speaker);
  check(`MEE071 keeps ${speaker} ${when}`, wrong.length === 0, { passages: later.labelled.length, wrong: wrong.map((p) => `${p.id}:${p.speaker}`), unlabelled: later.own.length - later.labelled.length });
  check(`the name stays on ${speaker} ${when}`, later.m.speakers.find((s) => s.id === speaker)?.name === name);
}

// After done: the score over the whole meeting, and no passage moved from one id to another.
async function speakersAfterDone(id, meeting, named) {
  const passages = callPassages(meeting.passages);
  const turns = reference(id);
  const score = accuracy(passages, turns);
  const nulls = passages.filter((p) => p.speaker == null);
  const total = passages.reduce((sum, p) => sum + (p.end - p.start), 0);
  const nullTime = nulls.reduce((sum, p) => sum + (p.end - p.start), 0);
  report.speakers.accuracy = score;
  report.speakers.mapping = mapping(passages, turns);
  report.speakers.callSpeakers = meeting.speakers.filter((s) => s.track === 'call').map((s) => s.id);
  report.speakers.nullShare = total ? nullTime / total : 0;
  check('at least 90% of the call passage time went to the right speaker', score >= 0.9, {
    accuracy: Number(score.toFixed(4)),
    passages: passages.length,
    withoutSpeaker: `${nulls.length} (${((100 * nullTime) / (total || 1)).toFixed(1)}% of the time)`,
    mapping: report.speakers.mapping
  });
  const moved = movedPassages(await passageEvents(id));
  check('no passage moved from one id to another between revisions', moved.length === 0, moved.slice(0, 10));
  if (named) check(`MEE071 maps to ${named.speaker} over the whole meeting`, report.speakers.mapping[named.speaker] === 'MEE071', report.speakers.mapping);
}

// ---------- --speakers=<reason> ----------

async function withoutLabels(reason) {
  folders(RUN);
  prepareVoiceData();
  speech(path.join(RUN, 'you.wav'), YOU);
  startVoice(1, { noSpeakers: reason });
  await until('the discovery file', () => fs.existsSync(BRIDGE), 60000);
  const status = await hook('status');
  const sp = status.settings.speakers;
  // A platform that can't label speakers can't show any of the other reasons either.
  if (sp.ready !== true && sp.reason === 'platform') throw new Refusal(SPEAKER_NEEDS);
  check(`Settings > Speaker labels says why: ${reason}`, sp.ready === false && sp.reason === reason, { reason: sp.reason, missing: sp.missing, detail: sp.detail });
  await startRukoo();
  await until('Rukoo to connect to Voice', rukoo.connected, 30000);

  const { meetingId: id } = await rukoo.start({ title: `Live run: no speaker labels (${reason})` });
  check('Rukoo started a meeting through Voice', Boolean(id), id);
  report.meeting = id;
  const live = await until('passages with the expected words on both tracks', async () => {
    const m = await rukoo.get(id);
    return m && trackHas(m, 'mic', YOU.words) && trackHas(m, 'call', THEM.words) && m;
  }, 240000, 500);
  check('the meeting records and transcribes both tracks without labels', (await rukoo.state()).recording === id, { call: live.passages.filter((p) => p.track === 'call').length });
  const entry = await labelsOf(id);
  check(`the meeting's labels are off with reason ${reason}`, entry?.mode === 'off' && entry?.reason === reason, entry);
  const dir = path.join(SHOTS, `speakers-${reason}`);
  if (MAC) skip('Voice captured Settings > Speaker labels', CAPTURE_SKIP);
  else {
    const shots = await hook('capture', { dir, page: 'speakers' }, 90000);
    check('Voice captured Settings > Speaker labels', !shots.settings.error, shots.settings);
  }

  // Every AMI voice has spoken once a whole loop has gone by.
  await until('a whole loop of the AMI excerpt', () => meetingNow(id) > AMI_MS + 15000, 180000, 1000);
  await rukoo.stop(id);
  await until('done', async () => (await rukoo.get(id)).meeting.state === 'done', 240000, 200);
  const m = await rukoo.get(id);
  const calls = m.speakers.filter((s) => s.track === 'call').map((s) => s.id);
  const others = m.passages.filter((p) => p.track === 'call' && p.speaker !== 'call-1');
  check('the whole call is one speaker, call-1, as in Voice 0.1', calls.length === 1 && calls[0] === 'call-1' && others.length === 0, { calls, otherPassages: others.length });
  check('the stop went to processing, then done', stoppedThenDone(await voiceStates(id)));
}

// ---------- --measure=speakers ----------

// CPU time, working set and peak working set of each process id, through Get-Process.
function processTimes(pids) {
  const ids = pids.filter(Boolean);
  if (!ids.length) return {};
  const at = Date.now();
  const out = spawnSync(
    'powershell.exe',
    ['-NoProfile', '-Command', `Get-Process -Id ${ids.join(',')} -ErrorAction SilentlyContinue | ForEach-Object { '{0} {1} {2} {3}' -f $_.Id, [long]$_.TotalProcessorTime.TotalMilliseconds, $_.WorkingSet64, $_.PeakWorkingSet64 }`],
    { encoding: 'utf8', windowsHide: true }
  );
  const wall = (at + Date.now()) / 2;
  const result = {};
  for (const line of (out.stdout || '').split(/\r?\n/).filter(Boolean)) {
    const [pid, cpu, ws, peak] = line.trim().split(/\s+/).map(Number);
    result[pid] = { cpu, ws, peak, wall };
  }
  return result;
}

// The children of a process by image name, through CIM.
function childrenOf(pid) {
  const out = spawnSync('powershell.exe', ['-NoProfile', '-Command', `Get-CimInstance Win32_Process -Filter "ParentProcessId=${pid}" | ForEach-Object { '{0} {1}' -f $_.ProcessId, $_.Name }`], { encoding: 'utf8', windowsHide: true });
  const result = {};
  for (const line of (out.stdout || '').split(/\r?\n/).filter(Boolean)) {
    const [child, name] = line.trim().split(/\s+/);
    result[name.replace(/\.exe$/i, '')] = Number(child);
  }
  return result;
}

async function measuredPids(id, voicePid) {
  const children = childrenOf(voicePid);
  const l = await labelsOf(id);
  return {
    'rukoo-voice': voicePid,
    'rukoo-voice-local': children['rukoo-voice-local'] || null,
    'rukoo-voice-speakers': l?.workerPid || children['rukoo-voice-speakers'] || null,
    'rukoo-voice-recorder': recorderOf(id)?.pid || null
  };
}

// The wall time of a meeting time, from the newest heartbeat at or before it.
function wallAt(beats, t) {
  let best = beats[0];
  for (const b of beats) if (b.t <= t) best = b;
  return best ? best.wall + (t - best.t) : null;
}

async function measuredMeeting(label, liveSpeakers, minutes) {
  folders(path.join(RUN, label));
  prepareVoiceData(liveSpeakers ? null : { meetings_live_speakers: false });
  const voice = startVoice(label);
  await until('the discovery file', () => fs.existsSync(BRIDGE), 60000);
  const sp = (await hook('status')).settings.speakers;
  check(`${label}: Settings > Speaker labels is ${liveSpeakers ? 'ready' : 'off'}`, liveSpeakers ? sp.ready === true : sp.reason === 'off', { reason: sp.reason, runtime: sp.runtime });

  const { meeting: id } = await hook('start_meeting');
  const result = { label, meeting: id, minutes, cpu: {}, samples: [], settled: [] };
  await until('the recording', async () => (await hook('status')).pill.meeting?.state === 'recording', 30000);
  const marks = [...new Set([1, 5, minutes].filter((m) => m <= minutes))];
  let pids = null;
  for (const minute of marks) {
    while (meetingNow(id) < minute * 60000) {
      await sleep(5000);
      if (liveSpeakers && minutes > 5) {
        try {
          const row = (await transcriptDb(id, "SELECT value FROM meta WHERE key = 'speakers.settled.call'"))[0];
          result.settled.push([Math.round(meetingNow(id)), row ? Number(row.value) : null]);
        } catch (_) {
          // The app holds the file; the next read gets it.
        }
      }
    }
    pids = await measuredPids(id, voice.pid);
    result.samples.push({ minute, pids, times: processTimes(Object.values(pids)) });
    console.log(`     ${label}: minute ${minute}`);
  }
  const l = await labelsOf(id);
  result.labels = l && { state: l.state, mode: l.mode, speakers: l.speakers, reason: l.reason };

  // CPU per process between minute 1 and each later sample, in % of one core.
  const first = result.samples[0];
  for (const sample of result.samples.slice(1)) {
    const row = {};
    let total = 0;
    for (const [name, pid] of Object.entries(sample.pids)) {
      const a = first.times[first.pids[name]];
      const b = sample.times[pid];
      if (!a || !b || first.pids[name] !== pid) {
        row[name] = null;
        continue;
      }
      row[name] = Number(((100 * (b.cpu - a.cpu)) / (b.wall - a.wall)).toFixed(1));
      total += row[name];
    }
    row.total = Number(total.toFixed(1));
    result.cpu[`1-${sample.minute}`] = row;
  }
  const last = result.samples.at(-1);
  const worker = last.pids['rukoo-voice-speakers'];
  result.workerPeakMb = worker && last.times[worker] ? Math.round(last.times[worker].peak / 1048576) : null;
  result.peakMb = Object.fromEntries(Object.entries(last.pids).map(([name, pid]) => [name, pid && last.times[pid] ? Math.round(last.times[pid].peak / 1048576) : null]));

  const stopWall = Date.now();
  await hook('stop_meeting');
  await until('done', async () => (await voiceStates(id)).at(-1) === 'done', 300000, 500);

  // Latency of each call passage's first revision, from the wall time of its end to the log taking it. Passages the
  // log took after the stop come from the final windows and aren't live.
  const beats = timeline(id).filter((x) => x.kind === 'heartbeat');
  const firsts = new Map();
  for (const e of await passageEvents(id)) if (!firsts.has(e.id)) firsts.set(e.id, e);
  const latency = { call: [], mic: [] };
  for (const e of firsts.values()) {
    if (e.createdAt > stopWall) continue;
    const wall = wallAt(beats, e.end);
    if (wall != null) latency[e.track]?.push(e.createdAt - wall);
  }
  result.latency = Object.fromEntries(Object.entries(latency).map(([track, xs]) => [track, { passages: xs.length, medianMs: median(xs), p90Ms: percentile(xs, 0.9) }]));

  if (liveSpeakers) {
    const events = await passageEvents(id);
    const latest = new Map();
    for (const e of events) latest.set(e.id, e);
    const passages = callPassages([...latest.values()]);
    const turns = reference(id);
    result.accuracy = Number(accuracy(passages, turns).toFixed(4));
    result.moved = movedPassages(events).length;
    const epochs = await transcriptDb(id, 'SELECT epoch, reason FROM speaker_epochs ORDER BY epoch');
    result.epochs = epochs.map((e) => e.reason);
    if (minutes >= 21) {
      const end = Math.max(...passages.map((p) => p.end));
      const head = mainIds(passages.filter((p) => p.end <= 5 * 60000), turns);
      const tail = mainIds(passages.filter((p) => p.start >= end - 5 * 60000), turns);
      const settledLast = result.settled.filter(([, s]) => s != null).at(-1);
      const log = fs.readFileSync(path.join(path.dirname(VOICE_DATA), `voice-${label}.log`), 'utf8');
      result.compaction = { head, tail, settledLast, behindInLog: /fell behind/.test(log) };
      check(`${label}: the settled position moved past minute 20`, settledLast && settledLast[1] > 20 * 60000, settledLast);
      check(`${label}: no worker restart and no "behind" error in ${minutes} minutes`, result.epochs.length === 1 && !result.compaction.behindInLog, { epochs: result.epochs });
      check(`${label}: the main AMI speakers keep their ids from the first five minutes to the last five`, MAIN.every((n) => head[n] && head[n] === tail[n]), { head, tail });
    }
    check(`${label}: at least 90% of the call passage time went to the right speaker`, result.accuracy >= 0.9, { accuracy: result.accuracy, moved: result.moved });
  }
  killPid(voice.pid);
  await until('Voice to exit', () => !alive(voice.pid), 15000);
  return result;
}

// The worker's own real-time factor on the AMI excerpt, through --self-test.
function selfTest() {
  if (!nemoDir) {
    note('self-test skipped: no runtime folder for --runtime (set RUKOO_VOICE_NEMO_DIR)');
    return null;
  }
  const out = path.join(RUN, 'self-test.json');
  const runs = [];
  for (let i = 0; i < 3; i++) {
    const res = spawnSync(SPEAKERS_EXE, ['--self-test', '--runtime', nemoDir, '--model', NEMOTRON, '--wav', AMI_WAV, '--report', out], { encoding: 'utf8', windowsHide: true });
    if (res.status !== 0 || !fs.existsSync(out)) {
      note(`self-test failed: ${res.stderr}`);
      return null;
    }
    runs.push(JSON.parse(fs.readFileSync(out, 'utf8')));
  }
  return { runs, medianRtf: Number(median(runs.map((r) => Math.round(r.realtimeFactor * 100))) / 100) };
}

async function measureSpeakers() {
  fs.mkdirSync(RUN, { recursive: true });
  speech(path.join(RUN, 'you.wav'), YOU);
  report.measure = { selfTest: selfTest() };
  if (report.measure.selfTest) note(`worker real-time factor on the AMI excerpt: ${report.measure.selfTest.medianRtf}x (median of 3)`);
  report.measure.on = await measuredMeeting('labels-on', true, MINUTES);
  report.measure.off = await measuredMeeting('labels-off', false, Math.min(MINUTES, 5));
  const { on, off } = report.measure;
  const row = (name, a, b) => console.log(`  ${name.padEnd(34)} ${String(a ?? '-').padStart(10)} ${String(b ?? '-').padStart(10)}`);
  console.log('\n  measure                             labels on  labels off');
  for (const p of ['rukoo-voice', 'rukoo-voice-local', 'rukoo-voice-speakers', 'rukoo-voice-recorder', 'total']) row(`CPU % of one core, min 1-5: ${p}`, on.cpu['1-5']?.[p], off.cpu['1-5']?.[p]);
  if (on.cpu[`1-${MINUTES}`] && MINUTES > 5) row(`CPU total, min 1-${MINUTES}`, on.cpu[`1-${MINUTES}`].total, null);
  row('call passage latency, median ms', on.latency.call.medianMs, off.latency.call.medianMs);
  row('call passage latency, p90 ms', on.latency.call.p90Ms, off.latency.call.p90Ms);
  row('mic passage latency, median ms', on.latency.mic.medianMs, off.latency.mic.medianMs);
  row('speaker worker peak working set MB', on.workerPeakMb, null);
  row('accuracy', on.accuracy, null);
}

// ---------- bucket sync during the standard run (--sync) ----------

// The fake bucket, then Settings > Storage as a user fills it in. The secret and the recovery code stay in memory.
async function syncSetup(sync) {
  await startFakeS3();
  const off = await syncStatus();
  check('sync is off until it is turned on', off.state === 'off', { state: off.state });
  const refused = await setting('storage.sync.enabled', true).then(() => null, (err) => err.message);
  check('the switch stays off before Test connection passed', /test/i.test(refused || ''), refused);
  const tested = await connectSync(fake.endpoint, sync.secret);
  const on = await syncStatus();
  check('Test connection passed and sync is on', on.state !== 'off' && on.state !== 'failing', { tested, state: on.state });
  const shown = await syncHook('key');
  sync.code = shown.code;
  check('Voice shows a recovery code for its key', typeof sync.code === 'string' && sync.code.startsWith('RKV1-'), { id: shown.id });
}

// While it records: audio objects, frames when the meeting records the screen, then a 20 s outage of the bucket.
async function syncWhileLive(id, t0) {
  const outage = {};
  const audio = await until('audio objects in the bucket while it records', () => {
    const n = s3Objects().size;
    return n >= 3 && n; // meeting.json and a segment of each track
  }, 120000, 1000);
  check('audio objects reach the bucket before the stop', (await rukoo.state()).recording === id, { objects: audio, after: `${Math.round((Date.now() - t0) / 1000)} s` });
  if (ARGS.screen !== 'off') {
    const frames = await until('frames in the bucket while it records', async () => {
      const rows = await queueRows(VOICE_DATA, id);
      const up = rows.filter((r) => r.kind === 'frame' && r.state === 'done');
      return up.length > 0 && up.length;
    }, 120000, 1000);
    const rows = await queueRows(VOICE_DATA, id);
    check('frames reach the bucket before the stop, and no video does', (await rukoo.state()).recording === id && !rows.some((r) => r.kind === 'video'), { frames, videoRows: rows.filter((r) => r.kind === 'video').length });
  }

  // Stop the fake about 7 s before the next segment closes, so the worker tries to upload it while the bucket is
  // gone.
  const closes = () => {
    const ends = timeline(id).filter((l) => l.kind === 'segment_end' && l.track === 'mic').map((l) => l.t);
    return (ends.length ? Math.max(...ends) : 0) + SEGMENT_MS;
  };
  const nowT = () => timeline(id).filter((l) => l.kind === 'heartbeat').at(-1)?.t ?? 0;
  let target = closes();
  if (nowT() > target - 4000) target += SEGMENT_MS;
  await until('the moment before a segment closes', () => nowT() >= target - 7000, 40000, 200);
  outage.start = Date.now();
  await stopFakeS3();
  const seen = new Set();
  let maxWaiting = 0;
  while (Date.now() - outage.start < 20000) {
    const s = await syncStatus();
    seen.add(s.state);
    maxWaiting = Math.max(maxWaiting, s.waiting);
    await sleep(500);
  }
  // Same port and folder; a refused bind is retried for a few seconds.
  for (let tries = 1; ; tries++) {
    try {
      await startFakeS3(fake.port);
      break;
    } catch (err) {
      if (tries >= 10) throw err;
      await sleep(1000);
    }
  }
  outage.end = Date.now();
  // Recovered: an upload succeeded after the bucket came back, the worker no longer retries, and what waited during
  // the outage went up (frames keep coming while the screen records, so a few may wait).
  const settled = ARGS.screen === 'off' ? 1 : 4;
  const back = await until('sync to recover after the outage', async () => {
    const s = await syncStatus();
    seen.add(s.state);
    const up = uploads(s3Log()).filter((l) => l.at >= outage.end);
    return up.length >= Math.min(maxWaiting, 2) && s.state !== 'retrying' && s.state !== 'failing' && s.waiting <= settled && { s, up };
  }, 180000, 500);
  const firstUp = back.up.reduce((n, l) => Math.min(n, l.at), Infinity);
  check('the outage shows as retrying, then sync recovers and the queue drains', seen.has('retrying') && back.s.waiting <= settled, {
    states: [...seen],
    maxWaiting,
    waitingAfter: back.s.waiting,
    uploadsAfter: back.up.length,
    firstUploadAfterMs: firstUp - outage.end,
    recoveredAfter: `${Math.round((Date.now() - outage.end) / 1000)} s`
  });
  // From just before the outage until sync recovered.
  const window = heartbeats(timeline(id), outage.start - 2000, Date.now());
  outage.heartbeats = window;
  check('heartbeats stay within 1,300 ms through the outage, with no gap marks', window.beats > 20 && window.maxGapMs <= 1300 && window.trouble.length === 0, window);
  await captureStorage('live');
  return outage;
}

// ---------- after the meetings, with --sync ----------

const sha = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
const queueRows = (data, meeting) => readDb(path.join(data, 'sync', 'queue.db'), 'SELECT path, kind, size, object, state FROM files WHERE meeting = ? ORDER BY id', meeting);

// 6. Every file in the bucket, encrypted, once; neither the secret nor the code on disk in plain text.
async function bucketChecks(ctx) {
  const { id, sync } = ctx;
  const status = await drained('after both meetings');
  check('every file of both meetings reached the bucket and the index followed', status.waiting === 0, { state: status.state, lastError: status.lastError && status.lastError.code });
  await captureStorage('synced');

  const rows = await queueRows(VOICE_DATA, id);
  const byKind = {};
  for (const r of rows) byKind[`${r.kind}:${r.state}`] = (byKind[`${r.kind}:${r.state}`] || 0) + 1;
  const segments = [];
  for (const track of ['mic', 'call']) {
    const dir = path.join(VOICE_DATA, 'meetings', id, 'audio', track);
    for (const f of fs.existsSync(dir) ? fs.readdirSync(dir).filter((n) => n.endsWith('.wav')) : []) segments.push(`audio/${track}/${f}`);
  }
  const notUp = segments.filter((p) => !rows.some((r) => r.path === p && r.state === 'done'));
  check("each of the meeting's segments, meeting.json, its timeline and its transcript are in the bucket", notUp.length === 0 && ['meeting', 'timeline', 'transcript', 'manifest'].every((k) => byKind[`${k}:done`]), { segments: segments.length, notUp, rows: byKind });

  // The screen folder by its rules (voice/RECORDING.md, "Other readers"): every frame and video file and the journal
  // went up; the clips and screen.json never did; video only after the timeline's stop.
  if (ARGS.screen !== 'off') {
    const screen = path.join(VOICE_DATA, 'meetings', id, 'screen');
    const onDisk = (sub, ext) => (fs.existsSync(path.join(screen, sub)) ? fs.readdirSync(path.join(screen, sub)).filter((n) => n.endsWith(ext)).map((n) => `screen/${sub}/${n}`) : []);
    const files = [...onDisk('frames', '.jpg'), ...onDisk('video', '.mp4'), 'screen/journal.jsonl'];
    const missingScreen = files.filter((p) => !rows.some((r) => r.path === p && r.state === 'done'));
    const never = rows.filter((r) => r.path.startsWith('screen/clips/') || r.path === 'screen/screen.json' || r.path.endsWith('.tmp')).map((r) => r.path);
    check("the screen journal, every frame and every video file are in the bucket; clips and screen.json never went up", missingScreen.length === 0 && never.length === 0 && byKind['screen:done'] === 1, {
      frames: files.filter((p) => p.startsWith('screen/frames/')).length,
      videos: files.filter((p) => p.startsWith('screen/video/')).length,
      missing: missingScreen,
      never
    });
    if (ARGS.screen === 'video') {
      // The fake logs object names; the queue says which object each file became.
      const log = uploads(s3Log());
      const firstUp = (object) => log.filter((l) => l.key.endsWith(object)).reduce((n, l) => Math.min(n, l.at), Infinity);
      const stopWall = wallOf(id, timeline(id).filter((l) => l.kind === 'stop').at(-1)?.t ?? 0) ?? ctx.stopAt;
      const videoRows = rows.filter((r) => r.kind === 'video' && r.state === 'done');
      const early = videoRows.filter((r) => firstUp(r.object) < stopWall).map((r) => r.path);
      const frameUps = rows.filter((r) => r.kind === 'frame' && r.state === 'done').map((r) => firstUp(r.object));
      check('video went up only after the stop, frames before it', videoRows.length > 0 && early.length === 0 && frameUps.some((at) => at < stopWall), { videos: videoRows.length, early, framesBeforeStop: frameUps.filter((at) => at < stopWall).length });
    }
  }

  encrypted('every object in the bucket passes check-encrypted.mjs');

  const dup = duplicates(s3Log());
  check('the Voice restart left no file uploaded twice and no part twice', dup.putTwice.length === 0 && dup.partTwice.length === 0, dup);

  const cfg = JSON.parse(fs.readFileSync(path.join(VOICE_DATA, 'config.json'), 'utf8'));
  const stored = cfg.extra?.storage_sync || cfg.storage_sync || {};
  const magic = (f, m) => {
    try {
      return fs.readFileSync(path.join(VOICE_DATA, 'sync', f)).subarray(0, 4).toString('latin1') === m;
    } catch (_) {
      return false;
    }
  };
  const logs = fs.readdirSync(RUN).filter((f) => /^voice-\d+\.log$/.test(f));
  const locked = [];
  const needles = [sync.secret, sync.code];
  const leaks = [...holders(VOICE_DATA, needles, locked), ...holders(HOOKS, needles, locked), ...logs.flatMap((f) => holders(path.join(RUN, f), needles, locked))];
  check('the secret and the key are DPAPI files; neither is in plain text in the data folder or the logs', !('secret' in stored) && magic('bucket-secret.bin', 'RKVS') && magic('master-key.bin', 'RKVK') && leaks.length === 0, { leaks, configHasSecret: 'secret' in stored, lockedUntilTheEnd: locked });

  // M1 and M2 for the healthy and the failing bucket, from the run's own meeting.
  const lines = timeline(id);
  report.measurements.M1_mainMeetingBeforeOutage = uploadRate(s3Log(), ctx.t0, ctx.outage.start);
  report.measurements.M2 = {
    healthy: heartbeats(lines, ctx.t0, ctx.outage.start),
    failing: ctx.outage.heartbeats
  };
}

// check-encrypted.mjs on the fake's folder, with every meeting id, the title and words of the speech as text no
// object may hold; and no key shows a meeting id.
function encrypted(name) {
  const objects = s3Objects();
  const ids = fs.readdirSync(path.join(VOICE_DATA, 'meetings')).filter((n) => n.startsWith('m-'));
  // "Slide 2" is what the synthetic screen shows, so the journal holds it as frame text.
  const forbid = [...ids, TITLE, 'warehouse', 'Amsterdam', ...(ARGS.screen !== 'off' ? ['Slide 2'] : [])].flatMap((t) => ['--forbid', t]);
  const enc = spawnSync(process.execPath, [path.join(VOICE, 'scripts', 'check-encrypted.mjs'), S3_DIR, '--json', ...forbid], { encoding: 'utf8', windowsHide: true });
  let result = null;
  try {
    result = JSON.parse(enc.stdout);
  } catch (_) {
    result = { error: enc.stderr };
  }
  check(name, enc.status === 0 && objects.size > 0, { meetings: ids.length, objects: result.objects, bytes: result.bytes, failed: result.failed, error: result.error });
  check('no object key shows a meeting id', [...objects.keys()].every((k) => !k.includes('m-20')), { objects: objects.size });
}

// The spec's measurements: a 10-minute meeting with sync on, a 3-minute one with sync off, six short ones.
async function measureSync(ctx) {
  const pid = ctx.voice().pid;
  const record = async (seconds, label) => {
    const before = cpuIo(pid);
    const { meeting } = await hook('start_meeting');
    const start = Date.now();
    console.log(`     ${label}: ${meeting}, ${seconds} s`);
    await until(`${label} to record 3 minutes`, () => Date.now() - start >= 180000, 200000, 1000);
    const at3 = cpuIo(pid);
    await until(`${label} to record ${seconds} s`, () => Date.now() - start >= seconds * 1000, seconds * 1000 + 60000, 1000);
    const stop = Date.now();
    await hook('stop_meeting');
    await until(`${label} to be done`, async () => (await rukoo.get(meeting))?.meeting.state === 'done', 300000, 1000);
    return { meeting, start, stop, before, at3 };
  };

  const b = await record(600, 'meeting B, sync on');
  await drained('meeting B');
  const bEnd = cpuIo(pid);
  const c0 = Date.now();
  await setting('storage.sync.enabled', false);
  const c = await record(180, 'meeting C, sync off');
  const cEnd = cpuIo(pid);
  await setting('storage.sync.enabled', true);
  const shorts = [];
  for (let i = 0; i < 6; i++) {
    const { meeting } = await hook('start_meeting');
    await sleep(6000);
    await hook('stop_meeting');
    await until('a short meeting to be done', async () => (await rukoo.get(meeting))?.meeting.state === 'done', 120000, 500);
    shorts.push(meeting);
  }
  await drained('every meeting');
  encrypted("every object of the measurements' ten meetings passes check-encrypted.mjs");
  ctx.measured = { b: b.meeting, c: c.meeting, shorts };

  const m = report.measurements;
  m.M1 = uploadRate(s3Log(), b.start, b.stop);
  m.M2.syncOn10min = heartbeats(timeline(b.meeting));
  m.M2.syncOff3min = heartbeats(timeline(c.meeting));
  m.M3 = {
    syncOnFirst3min: usage(b.before, b.at3),
    syncOffFirst3min: usage(c.before, c.at3),
    syncOnWhole10minAndUpload: usage(b.before, bEnd),
    syncOffWhole3min: usage(c.before, cEnd),
    note: `meeting C started ${Math.round((c.start - c0) / 1000)} s after sync went off`
  };

  // M5 on this computer: a 10-minute audio link from local audio.
  await rukoo.linkTtl(300000);
  const t = performance.now();
  const link = await rukoo.mediaLink({ meeting_id: b.meeting, kind: 'audio', from_seconds: 0, to_seconds: 600 });
  const toolMs = Math.round(performance.now() - t);
  const got = link.error ? null : await download(link.url);
  await rukoo.linkTtl(LINK_TTL_MS);
  check('a 10-minute audio link downloads from local audio', got && got.status === 200 && got.bytes.length === 600 * 32000 + 44, { error: link.error, status: got?.status, bytes: got?.bytes.length });
  ctx.measured.bSha = got ? sha(got.bytes) : null;
  m.M5 = { local: { bytes: got?.bytes.length, toolCallMs: toolMs, firstByteMs: got?.firstByteMs, totalMs: got?.totalMs, mbPerS: got?.mbPerS } };

  // M7: what meeting B costs in the bucket against its plaintext.
  const rows = await queueRows(VOICE_DATA, b.meeting);
  const objects = s3Objects();
  let plain = 0;
  let stored = 0;
  let audioObjects = 0;
  const segmentSizes = new Set();
  const counted = new Set();
  // Newest row first: a manifest keeps its object name, so only its last generation counts.
  for (const r of rows.filter((x) => x.state === 'done').reverse()) {
    const keyed = [...objects.keys()].find((k) => k.endsWith(r.object));
    if (!keyed || counted.has(keyed)) continue;
    counted.add(keyed);
    plain += r.size;
    stored += objects.get(keyed);
    if (r.kind === 'audio') {
      audioObjects++;
      if (r.size === 960044) segmentSizes.add(objects.get(keyed));
    }
  }
  m.M7 = {
    meeting: '10 minutes, two tracks',
    files: counted.size,
    audioObjects,
    plaintextBytes: plain,
    bucketBytes: stored,
    overheadPercent: plain ? +(((stored - plain) / plain) * 100).toFixed(3) : null,
    fullSegmentObjectBytes: [...segmentSizes],
    perHourMB: +((stored / 1e6) * (3600 / ((b.stop - b.start) / 1000))).toFixed(1)
  };
}

// A frame link and, for a meeting that recorded video, a video link: the bytes of the JPEG and of an MP4 stretch.
async function screenLinks(id, label) {
  const out = {};
  const frames = (first && first.frames) || [];
  if (ARGS.screen === 'off' || !frames.length) return out;
  const at = Math.floor(frames[Math.floor(frames.length / 2)].at / 1000);
  const frameLink = await rukoo.mediaLink({ meeting_id: id, kind: 'frame', from_seconds: at });
  const frame = frameLink.error ? null : await download(frameLink.url);
  const jpeg = frame && frame.status === 200 && frame.bytes[0] === 0xff && frame.bytes[1] === 0xd8;
  check(`a frame link gives the frame's JPEG (${label})`, jpeg && frame.type === 'image/jpeg', { error: frameLink.error, status: frame?.status, type: frame?.type, bytes: frame?.bytes.length, at });
  out.frame = jpeg ? frame.bytes : null;
  if (ARGS.screen === 'video') {
    const videoLink = await rukoo.mediaLink({ meeting_id: id, kind: 'video', from_seconds: 0, to_seconds: 60 });
    const video = videoLink.error ? null : await download(videoLink.url);
    const mp4 = video && video.status === 200 && video.bytes.toString('latin1', 4, 8) === 'ftyp';
    check(`a video link gives an MP4 of one stretch (${label})`, mp4 && video.type === 'video/mp4', { error: videoLink.error, status: video?.status, type: video?.type, bytes: video?.bytes.length, to: videoLink.to_seconds });
    out.video = mp4 ? video.bytes : null;
  }
  return out;
}

// The first meeting's snapshot from Voice, for the screen links and the restore.
let first = null;

// 7. An agent's link to the meeting's audio: the speech, a range, and refused once expired; then the screen.
async function linkChecks(ctx) {
  first = ctx.theirs;
  const you = fs.readFileSync(path.join(RUN, 'you.wav'));
  const them = fs.readFileSync(AMI ? AMI_WAV : path.join(RUN, 'them.wav'));
  const link = await rukoo.mediaLink({ meeting_id: ctx.id, kind: 'audio', from_seconds: 0, to_seconds: 20 });
  check('get_meeting_media_link gives a link on the local listener', !link.error && link.url.startsWith(`http://127.0.0.1:${MCP_PORT}/media/v1/`), {
    error: link.error,
    type: link.type,
    to: link.to_seconds,
    expiresIn: link.expires_in_seconds
  });
  if (link.error) return;
  const got = await download(link.url);
  const ranged = await download(link.url, { range: 'bytes=0-43' });
  const match = got.status === 200 ? matchesSpeech(got.bytes, you, them) : null;
  check("the link gives the meeting's speech, both tracks mixed", got.status === 200 && got.type === 'audio/wav' && Math.abs(match.seconds - 20) < 0.05 && match.r >= 0.9, { status: got.status, type: got.type, bytes: got.bytes.length, match });
  check('a Range request gives those bytes', ranged.status === 206 && ranged.bytes.equals(got.bytes.subarray(0, 44)), { status: ranged.status });
  await until('the link to expire', () => Date.now() > Date.parse(link.expires_at) + 1000, 15000, 200);
  const expired = await download(link.url);
  check('the same link is refused after it expired', expired.status === 410, { status: expired.status });
  ctx.localAudio = got.bytes;
  ctx.localScreen = await screenLinks(ctx.id, 'from this computer');
}

// The link fetched from another machine on the tailnet, over ssh, read only: Rukoo's Tailscale listener opens for
// Clark with a throwaway key in this profile.
async function remoteFetch(ctx) {
  const target = REMOTE_FETCH;
  const key = crypto.randomBytes(24).toString('base64url');
  const token = crypto.createHmac('sha256', key).update('rukoo-mcp-v1').digest('base64url');
  await app.evaluate(async (_, k) => {
    const hub = global.__semAgents;
    hub.setSecret('clark', k);
    await hub.updateConfig({ mcp: { remote: true } });
  }, key);
  const at = await until('the Tailscale listener', () =>
    app.evaluate(() => {
      const hub = global.__semAgents;
      const port = hub.mcp && hub.mcp.ports().remote;
      return port ? { port, host: hub.cfg.address() } : null;
    }), 30000, 500);
  const meeting = ctx.measured ? ctx.measured.b : ctx.id;
  const to = ctx.measured ? 600 : 20;
  const want = ctx.measured ? ctx.measured.bSha : sha(ctx.localAudio);
  const ttl = 60000;
  await rukoo.linkTtl(ttl);
  const body = JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'get_meeting_media_link', arguments: { meeting_id: meeting, kind: 'audio', from_seconds: 0, to_seconds: to } } });
  const ssh = (script) => spawnSync('ssh', ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15', target, 'bash -s'], { input: script, encoding: 'utf8', windowsHide: true, timeout: 180000 });
  const first = ssh(
    [
      'set -u',
      `T='${token}'`,
      `B='${body}'`,
      `R=$(curl -s -m 60 -H "Authorization: Bearer $T" -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' --data "$B" http://${at.host}:${at.port}/mcp)`,
      `U=$(printf '%s' "$R" | grep -o '"url":"[^"]*"' | head -1 | cut -d'"' -f4)`,
      'if [ -z "$U" ]; then echo "NOURL"; exit 0; fi',
      'F=$(mktemp)',
      'echo "URL $U"',
      `curl -s -m 120 -o "$F" -w 'GOT %{http_code} %{size_download} %{time_starttransfer} %{speed_download}\\n' "$U"`,
      'echo "SHA $(sha256sum "$F" | cut -d" " -f1)"',
      'rm -f "$F"'
    ].join('\n')
  );
  const out = String(first.stdout || '');
  const url = (/^URL (\S+)$/m.exec(out) || [])[1];
  const gotLine = (/^GOT (\d+) (\d+) ([\d.]+) ([\d.]+)$/m.exec(out) || []).slice(1);
  const remoteSha = (/^SHA ([0-9a-f]{64})$/m.exec(out) || [])[1];
  const issuedAt = Date.now();
  check('a machine on the tailnet gets a link from the tool and downloads the same audio', Boolean(url) && gotLine[0] === '200' && remoteSha === want, {
    ssh: first.status,
    status: gotLine[0],
    bytes: Number(gotLine[1]),
    sameAsLocal: remoteSha === want,
    stderr: first.status ? String(first.stderr || '').slice(0, 200) : undefined
  });
  report.measurements.M5 = report.measurements.M5 || {};
  report.measurements.M5.tailscale = { bytes: Number(gotLine[1]), firstByteMs: Math.round(Number(gotLine[2]) * 1000), mbPerS: +(Number(gotLine[3]) / 1e6).toFixed(1) };
  if (url) {
    const here = await download(url.replace(/^http:\/\/[^/]+/, `http://${at.host}:${at.port}`));
    check("the link is refused from another address on the tailnet (this computer's)", here.status === 403, { status: here.status });
    await until('the remote link to expire', () => Date.now() > issuedAt + ttl + 2000, ttl + 30000, 500);
    const again = ssh(`curl -s -m 30 -o /dev/null -w '%{http_code}' '${url}'`);
    check('the remote link is refused once expired', String(again.stdout).trim() === '410', { status: String(again.stdout).trim() });
  }
  await app.evaluate(async () => {
    const hub = global.__semAgents;
    await hub.updateConfig({ mcp: { remote: false } });
    hub.setSecret('clark', '');
  });
  await rukoo.linkTtl(LINK_TTL_MS);
}

// 8. A second Voice on a fresh data folder: the code, the bucket, a restore, and the media back from the bucket.
async function restoreChecks(ctx) {
  const { id, sync } = ctx;
  await drained('before the restore');
  const old = ctx.voice();
  killPid(old.pid);
  await until('Voice to exit', () => !alive(old.pid), 15000);
  const meetings = fs.readdirSync(path.join(VOICE_DATA, 'meetings')).filter((n) => n.startsWith('m-'));
  started.dataDirs.push(VOICE_DATA_2);
  const v = startVoice(3, { data: VOICE_DATA_2, monitor: report.screen.pinned });
  ctx.setVoice(v);
  await until('the second Voice', async () => {
    try {
      return (await hook('status', {}, 5000)).pid === v.pid;
    } catch (_) {
      return false;
    }
  }, 60000, 1000);
  const imported = await syncHook('import', { code: sync.code });
  await connectSync(fake.endpoint, sync.secret);
  const list = await syncHook('restore_list');
  check('the second Voice lists every meeting in the bucket', list.meetings.length === meetings.length, { listed: list.meetings.length, meetings: meetings.length, key: imported && imported.id });
  const r0 = Date.now();
  await syncHook('restore_start');
  const done = await until('the restore', async () => {
    const s = await syncStatus();
    return ['done', 'failed'].includes(s.restore.state) && s;
  }, 600000, 250);
  const restoreMs = Date.now() - r0;
  check('the restore finished', done.restore.state === 'done', done.restore);
  report.measurements.M6 = { meetings: list.meetings.length, seconds: +(restoreMs / 1000).toFixed(1), fromBucket: 'voice-s3-fake on this computer' };

  await until('Rukoo to connect to the second Voice', async () => (await rukoo.connected()) && (await rukoo.state()).voiceVersion, 60000);
  const theirs = await until('the restored meeting through the bridge', async () => {
    try {
      const m = await rukoo.voiceGet(id);
      return m && m.passages.length >= ctx.theirs.passages.length && m;
    } catch (_) {
      return false;
    }
  }, 120000, 1000);
  const key = (p) => `${p.id}@${p.revision}`;
  const sameSet = (a, b) => a.length === b.length && a.every((p) => b.some((q) => key(q) === key(p)));
  check('the restored meeting reaches Rukoo with every passage the first Voice had', sameSet(theirs.passages, ctx.theirs.passages), { first: ctx.theirs.passages.length, restored: theirs.passages.length });
  const mine = await rukoo.get(id);
  check("Rukoo's meeting.get still has every passage once", sameSet(mine.passages, ctx.theirs.passages), { rukoo: mine.passages.length });
  const same = (f) => {
    try {
      return fs.readFileSync(path.join(VOICE_DATA, 'meetings', id, f)).equals(fs.readFileSync(path.join(VOICE_DATA_2, 'meetings', id, f)));
    } catch (_) {
      return false;
    }
  };
  const localAudio = fs.existsSync(path.join(VOICE_DATA_2, 'meetings', id, 'audio'));
  check('meeting.json and the timeline come back byte for byte; audio stays in the bucket until needed', same('meeting.json') && same('timeline.jsonl') && !localAudio, { localAudio });
  if (ARGS.screen !== 'off') {
    const restored = (sub) => fs.existsSync(path.join(VOICE_DATA_2, 'meetings', id, 'screen', sub));
    check('the screen journal comes back byte for byte with the text; frames and video stay in the bucket until needed', same('screen/journal.jsonl') && !restored('frames') && !restored('video'), {
      frames: restored('frames'),
      video: restored('video')
    });
    const ids = (x) => (x.frames || []).map((f) => f.id).sort().join(',');
    check('the restored meeting reaches Rukoo with every frame the first Voice had', ids(theirs) === ids(ctx.theirs) && (theirs.frames || []).length > 0, { first: (ctx.theirs.frames || []).length, restored: (theirs.frames || []).length });
  }
  // Speaker ids and labels live in transcript.db, which the restore brings back (contract 2.5).
  const speakers = (x) => (x.speakers || []).map((sp) => `${sp.id}=${sp.label}`).sort().join(',');
  check('the restored meeting has the same speaker ids and labels', speakers(theirs) === speakers(ctx.theirs) && (theirs.speakers || []).length > 0, { first: speakers(ctx.theirs), restored: speakers(theirs) });
  if (SPEAKERS === true) {
    const named = (mine.speakers || []).filter((sp) => sp.name).map((sp) => sp.id);
    check("Rukoo keeps the names it gave the speakers across the restore", named.length > 0, { named });
  }

  // The same audio link, now served from the bucket.
  if (ctx.localAudio) {
    const t = performance.now();
    const link = await rukoo.mediaLink({ meeting_id: id, kind: 'audio', from_seconds: 0, to_seconds: 20 });
    const toolMs = Math.round(performance.now() - t);
    const got = link.error ? null : await download(link.url);
    check("the restored meeting's audio comes back from the bucket, byte for byte", got && got.status === 200 && got.bytes.equals(ctx.localAudio), { error: link.error, status: got?.status, toolMs, firstByteMs: got?.firstByteMs });
  }
  // The frame and the video, now fetched back from the bucket.
  if (ctx.localScreen && ctx.localScreen.frame) {
    const back = await screenLinks(id, 'fetched back from the bucket');
    check("the restored meeting's frame comes back from the bucket, byte for byte", back.frame && back.frame.equals(ctx.localScreen.frame), { bytes: back.frame?.length });
    if (ctx.localScreen.video) {
      const sameVideo = Boolean(back.video && back.video.equals(ctx.localScreen.video));
      if (!sameVideo) note(`the video link from the bucket gave ${back.video?.length ?? 0} bytes, the local one ${ctx.localScreen.video.length}`);
      report.screen.restoredVideoSameBytes = sameVideo;
    }
  }
  if (ctx.measured) {
    await rukoo.linkTtl(300000);
    const t = performance.now();
    const link = await rukoo.mediaLink({ meeting_id: ctx.measured.b, kind: 'audio', from_seconds: 0, to_seconds: 600 });
    const toolMs = Math.round(performance.now() - t);
    const got = link.error ? null : await download(link.url);
    await rukoo.linkTtl(LINK_TTL_MS);
    check('the 10-minute link from the bucket gives the same bytes as from local audio', got && got.status === 200 && sha(got.bytes) === ctx.measured.bSha, { error: link.error, status: got?.status });
    report.measurements.M5.bucket = { bytes: got?.bytes.length, toolCallMs: toolMs, firstByteMs: got?.firstByteMs, totalMs: got?.totalMs, mbPerS: got?.mbPerS };
  }
  const locked = [];
  const leaks = holders(VOICE_DATA_2, [sync.secret, sync.code], locked);
  check("the second Voice's data folder holds neither the secret nor the code in plain text", leaks.length === 0, { leaks, lockedUntilTheEnd: locked });
}

// ---------- the paired run ----------

async function pairedRun() {
  folders(RUN);
  prepareVoiceData();
  const voices = MAC ? sayVoices() : null;
  const youBytes = speech(path.join(RUN, 'you.wav'), { ...YOU, voice: voices?.you });
  if (MAC) report.speech = { engine: 'say', voices: { you: voices.you, them: voices.them } };
  if (AMI) note(`speech: you.wav ${(youBytes / 32000).toFixed(1)} s; the call is the AMI excerpt, ${(AMI_MS / 1000).toFixed(1)} s, looped`);
  else {
    const themBytes = speech(path.join(RUN, 'them.wav'), { ...THEM, voice: voices?.them });
    note(`speech: you.wav ${(youBytes / 32000).toFixed(1)} s, them.wav ${(themBytes / 32000).toFixed(1)} s${voices ? ` (say: ${voices.you} and ${voices.them})` : ''}`);
  }

  let offsets = null;
  let pinned = null;
  if (ARGS.realScreen) {
    offsets = partOffsets(THEM);
    note(`the call's parts, and so the slides, start at ${offsets.map((ms) => `${(ms / 1000).toFixed(1)} s`).join(', ')} of its WAV`);
    await startScreens();
    const slide = ARGS.chooseMonitor ? displays.find((d) => d.id === ARGS.chooseMonitor) : displays.find((d) => d.primary);
    if (!slide) throw new Error(`no display ${ARGS.chooseMonitor} on this computer`);
    report.screen.slideDisplay = slide.id;
    // --choose-monitor checks the choice instead of pinning it.
    pinned = ARGS.chooseMonitor ? null : slide.id;
    report.screen.pinned = pinned;
    await showDemoWindows(slide.id, { focused: Boolean(ARGS.chooseMonitor) });
  }

  // The synthetic screen of a measurement takes the sizes of this computer's displays, read from Windows, the main
  // display's first: the cost of encoding and reading text depends on the pixels.
  let sizes = [];
  if (ARGS.measure === 'screen' && !ARGS.realScreen) {
    const listed = windowsDisplays().sort((a, b) => Number(b.primary) - Number(a.primary));
    sizes = [...new Set(listed.map((d) => `${d.width}x${d.height}`))];
    if (sizes.length) syntheticSize = sizes[0];
    report.screen.syntheticSizes = sizes;
    note(`the synthetic screen takes the displays' sizes: ${sizes.join(', ') || 'none listed, so 1920x1080'}`);
  }

  // 1. Voice, then Rukoo, paired through RUKOO_VOICE_BRIDGE.
  const voice = await pair(pinned);
  if (ARGS.measure === 'screen') await measureAll(voice, sizes);
  else if (ARGS.chooseMonitor) await chooseRun(offsets);
  else if (ARGS.visibleRukoo) await visibleRun();
  else await standardRun(voice, offsets);
}

// ---------- main ----------

async function main() {
  if (!WIN && !MAC) throw new Error('The live run needs Windows or macOS: Voice records there only.');
  fs.mkdirSync(RUN, { recursive: true });
  fs.mkdirSync(SHOTS, { recursive: true });
  if (!ARGS.skipBuild) build();
  // The screen program builds on both platforms, but only Windows runs it; the check is for the programs this run uses.
  for (const exe of [EXE, ...(WIN ? [SCREEN_EXE] : []), ...(SYNC ? [FAKE_S3] : [])]) if (!fs.existsSync(exe)) throw new Error(`${exe} is missing; build it first`);
  if (AMI) {
    assets();
    loadAmi();
  }
  if (MEASURE === 'speakers') await measureSpeakers();
  else if (REASONS.includes(SPEAKERS)) await withoutLabels(SPEAKERS);
  else await pairedRun();
}

async function cleanup() {
  // Rukoo first: its quit path, then its process id if it hangs.
  for (const electronApp of [app, screens]) {
    if (!electronApp) continue;
    const pid = electronApp.process().pid;
    try {
      await electronApp.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().forEach((w) => w.destroy()));
      await Promise.race([electronApp.close(), sleep(45000)]);
    } catch (_) {
      // Gone already.
    }
    if (alive(pid)) {
      if (WIN) spawnSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore', windowsHide: true });
      else killTree(pid);
    }
  }
  for (const pid of started.measure) if (alive(pid)) await stopPid(pid);
  for (const pid of started.voice) if (alive(pid)) await stopPid(pid);
  for (const pid of started.fake) if (alive(pid)) await stopPid(pid);
  // A recorder or a screen process still recording after a failure: by the id in its recorder.json or screen.json.
  for (const data of started.dataDirs) {
    const meetings = path.join(data, 'meetings');
    for (const id of fs.existsSync(meetings) ? fs.readdirSync(meetings) : []) {
      const dir = path.join(meetings, id);
      for (const [what, info] of [['recorder', readJson(path.join(dir, 'recorder.json'))], ['screen process', readJson(path.join(dir, 'screen', 'screen.json'))]]) {
        if (info && alive(info.pid)) {
          await stopPid(info.pid);
          note(`stopped a ${what} that was still running: ${info.pid}`);
        }
      }
    }
  }
  report.leftRunning = [...started.voice, started.rukoo, started.screens, ...started.fake].filter((pid) => pid && alive(pid));
  if (ARGS.realScreen) {
    await sleep(1000);
    deleteRealCaptures();
  }
}

let error = null;
let refused = null;
try {
  await main();
} catch (err) {
  if (err instanceof Refusal) refused = err;
  else {
    error = err;
    check('the run finished', false, err.message);
  }
} finally {
  await cleanup();
  // With every process stopped, nothing holds a file open: the whole run folder, both data folders, the hooks, the
  // logs and the bucket's objects, for the secret and the code.
  if (SYNC && SECRETS.code) {
    try {
      const locked = [];
      const leaks = holders(RUN, [SECRETS.secret, SECRETS.code], locked);
      check('after every process stopped, no file of the run holds the secret or the recovery code', leaks.length === 0 && locked.length === 0, { leaks, locked });
    } catch (err) {
      check('after every process stopped, no file of the run holds the secret or the recovery code', false, err.message);
    }
  }
  report.finished = new Date().toISOString();
  report.failed = failed;
  report.skipped = skipped;
  if (refused) report.refused = refused.message;
  fs.mkdirSync(RUN, { recursive: true });
  fs.writeFileSync(path.join(RUN, 'report.json'), JSON.stringify(report, null, 2));
  if (refused) console.error(`\n${refused.message}\nNothing was recorded; report: ${path.join(RUN, 'report.json')}`);
  else console.log(`\n${failed ? `${failed} check(s) failed` : 'every check passed'}${skipped ? `, ${skipped} skipped (see the skip lines and the report)` : ''}; report: ${path.join(RUN, 'report.json')}`);
  if (error) console.error(error);
  process.exit(failed ? 1 : refused ? 2 : 0);
}
