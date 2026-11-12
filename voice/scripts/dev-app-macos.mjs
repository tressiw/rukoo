// A development Rukoo Voice.app on macOS. Builds the five programs, puts them in
// tmp/voice-dev/Rukoo Voice Dev.app, signs that bundle ad-hoc with hardened runtime and starts it with `open`, so
// macOS gives the microphone and the other permissions to the app instead of to the terminal that started it.
//
//   node voice/scripts/dev-app-macos.mjs [--release] [--data <folder>] [--bridge <file>] [--rebuild] [--no-open]
//                                       [--stop] [--fake-engine[=<scenario>]] [--dev-hooks <folder>]
//                                       [--self-test <folder>] [-- <app flag> ...]
//
// The bundle is `nl.bvdm.rukoo.voice.dev`, named "Rukoo Voice Dev", so its grants never mix with an installed Voice.
// `Contents/MacOS` holds `rukoo-voice` and the four helpers; the app finds them next to itself. The version comes
// from voice/Cargo.toml, and `voice-app/icons`' .icns becomes the icon when there is one.
//
// Signing: each helper first, then the app, never `--deep`. `rukoo-voice` and `rukoo-voice-recorder` get
// `com.apple.security.device.audio-input`, `rukoo-voice-speakers` alone gets
// `com.apple.security.cs.disable-library-validation` because it dlopens ad-hoc signed NeMo dylibs, and the other two
// get none. No App Sandbox. A helper signs as `<bundle id>.<suffix>`, so its identifier stays the same from build to
// build instead of the `<name>-<hash>` codesign makes up for a binary without an Info.plist.
//
// macOS ties permissions to an app's code signature, and a build whose programs changed has another signature, so a
// rebuilt app loses its permissions while System Settings still lists the old entries as switched on. The script
// therefore rebuilds only when something changed: it hashes the five programs, the rendered Info.plist, the
// entitlements files and the icon into tmp/voice-dev/build-stamp.json and compares that with a fresh build and with
// what the bundle holds. A run without a change copies and signs nothing, so the grants of the last build still
// apply. `--rebuild` forces a rebuild. A rebuild stops the copy this script started, then writes each file next to
// its place and renames it over the old one, so a copy the script didn't start keeps running on its old signed
// files.
//
// After a rebuild it clears the old entries of the dev bundle alone with
// `tccutil reset <service> nl.bvdm.rukoo.voice.dev` (Microphone, AudioCapture, ListenEvent, Accessibility and
// ScreenCapture; a service this macOS refuses is reported and the rest still run) and prints the System Settings
// panes to switch on again. It never resets another bundle id, and it never touches
// ~/Library/Application Support/Rukoo Voice or an installed Voice.
//
// The app runs with RUKOO_VOICE_DATA_DIR=tmp/voice-dev/data and RUKOO_VOICE_BRIDGE=tmp/voice-dev/bridge.json, and
// its log is tmp/voice-dev/data/logs/app.log. `--data` and `--bridge` override those paths. `--fake-engine`,
// `--dev-hooks` and `--self-test` pass through to the app, with relative folders made absolute, because the working
// directory of an app started with `open` is not the repository. The app's pid goes into tmp/voice-dev/pids.json;
// `open` hands back no pid, so the script finds the process by this bundle's exact executable path, which is also how
// `--stop` finds the app and the four helpers: SIGTERM, then SIGKILL after five seconds. Nothing is stopped by name.
// `--no-open` builds and signs only.

import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const VOICE = path.join(ROOT, 'voice');
const DEV = path.join(ROOT, 'tmp', 'voice-dev');
const BUNDLE = path.join(DEV, 'Rukoo Voice Dev.app');
const CONTENTS = path.join(BUNDLE, 'Contents');
const MACOS = path.join(CONTENTS, 'MacOS');
const RESOURCES = path.join(CONTENTS, 'Resources');
const STAMP = path.join(DEV, 'build-stamp.json');
const PIDS = path.join(DEV, 'pids.json');
const APP_TEMPLATE = path.join(VOICE, 'crates', 'voice-app', 'macos', 'Info.dev.plist');
const ENTITLEMENTS_AUDIO = path.join(VOICE, 'crates', 'voice-app', 'macos', 'entitlements-audio.plist');
const ENTITLEMENTS_SPEAKERS = path.join(VOICE, 'crates', 'voice-app', 'macos', 'entitlements-speakers.plist');
const ICONS = path.join(VOICE, 'crates', 'voice-app', 'icons');

const BUNDLE_ID = 'nl.bvdm.rukoo.voice.dev';
const APP_NAME = 'Rukoo Voice Dev';
const CRATES = ['voice-app', 'voice-recorder', 'voice-local', 'voice-speakers', 'voice-screen'];
const PROGRAM = 'rukoo-voice';
const HELPERS = ['rukoo-voice-recorder', 'rukoo-voice-local', 'rukoo-voice-speakers', 'rukoo-voice-screen'];
const PROGRAMS = [PROGRAM, ...HELPERS];
// Each helper signs as `<bundle id>.<suffix>`, so its identifier doesn't change from build to build as the
// generated `<name>-<hash>` would.
const HELPER_IDS = {
  'rukoo-voice-recorder': 'recorder',
  'rukoo-voice-local': 'local',
  'rukoo-voice-speakers': 'speakers',
  'rukoo-voice-screen': 'screen',
};
const MAIN_EXE = path.join(MACOS, PROGRAM);
const TCC_SERVICES = ['Microphone', 'AudioCapture', 'ListenEvent', 'Accessibility', 'ScreenCapture'];
const TCC_PANES = [
  'Privacy & Security > Microphone: dictation and the microphone side of a meeting',
  'Privacy & Security > Screen & System Audio Recording > System Audio Recording Only: the other side of a meeting',
  'Privacy & Security > Screen & System Audio Recording: a meeting that records the screen',
  'Privacy & Security > Input Monitoring: the dictation key, when macOS asks for it',
  'Privacy & Security > Accessibility: typing into another app and reading the selection',
];
const APP_FLAGS = new Set(['--dev-hooks', '--self-test']);
const DATA_DEFAULT = path.join(DEV, 'data');
const BRIDGE_DEFAULT = path.join(DEV, 'bridge.json');
const ICON_MARKER = '\t<!-- scripts/dev-app-macos.mjs puts CFBundleIconFile here when voice-app/icons has an .icns. -->\n';
const STOP_GRACE_MS = 5000;

const USAGE = `Usage: node voice/scripts/dev-app-macos.mjs [options] [-- <app flag> ...]

Builds and runs tmp/voice-dev/${APP_NAME}.app (${BUNDLE_ID}) on macOS arm64.

Options:
  --release              build the release programs instead of the debug ones
  --data <folder>        the app's data folder (default ${path.relative(ROOT, DATA_DEFAULT)})
  --bridge <file>        the discovery file (default ${path.relative(ROOT, BRIDGE_DEFAULT)})
  --rebuild              rebuild and sign the bundle even when nothing changed
  --no-open              build and sign only
  --stop                 stop the app and the helpers running from this bundle, by pid
  -h, --help             this text

App flags, passed to the bundle:
  --fake-engine[=<scenario>]  --dev-hooks <folder>  --self-test <folder>
Anything after -- is passed to the app unchanged.`;

function parseArgs(argv) {
  const out = {
    release: false,
    rebuild: false,
    noOpen: false,
    stop: false,
    help: false,
    data: DATA_DEFAULT,
    bridge: BRIDGE_DEFAULT,
    forward: [],
  };
  const raw = [];
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--') {
      raw.push(...argv.slice(i + 1));
      break;
    }
    const eq = arg.indexOf('=');
    const key = eq > 0 ? arg.slice(0, eq) : arg;
    const value = eq > 0 ? arg.slice(eq + 1) : null;
    const valueFor = () => {
      const taken = value ?? argv[++i];
      if (taken === undefined || taken === '') throw new Error(`${key} needs a value`);
      return taken;
    };
    switch (key) {
      case '-h':
      case '--help':
        out.help = true;
        break;
      case '--release':
        out.release = true;
        break;
      case '--rebuild':
        out.rebuild = true;
        break;
      case '--no-open':
        out.noOpen = true;
        break;
      case '--stop':
        out.stop = true;
        break;
      case '--data':
        out.data = path.resolve(valueFor());
        break;
      case '--bridge':
        out.bridge = path.resolve(valueFor());
        break;
      case '--fake-engine':
        if (value === '') throw new Error('--fake-engine= needs a scenario');
        raw.push(value === null ? '--fake-engine' : `--fake-engine=${value}`);
        break;
      case '--dev-hooks':
      case '--self-test':
        if (value === null) raw.push(key, valueFor());
        else {
          if (value === '') throw new Error(`${key}= needs a folder`);
          raw.push(arg);
        }
        break;
      default:
        throw new Error(`unknown option ${arg}; app flags go behind --`);
    }
  }
  out.forward = passThrough(raw);
  if (out.stop && out.forward.length) throw new Error('--stop takes no app flags');
  return out;
}

// The app's flags for folders, with relative paths against where this script was started, because an app started
// with `open` runs in another working directory.
function passThrough(raw) {
  const out = [];
  for (let i = 0; i < raw.length; i++) {
    const arg = raw[i];
    const eq = arg.indexOf('=');
    const key = eq > 0 ? arg.slice(0, eq) : arg;
    if (!APP_FLAGS.has(key)) {
      out.push(arg);
      continue;
    }
    if (eq > 0) {
      const folder = arg.slice(eq + 1);
      if (!folder) throw new Error(`${key}= needs a folder`);
      out.push(`${key}=${path.resolve(folder)}`);
    } else {
      const folder = raw[++i];
      if (!folder) throw new Error(`${key} needs a folder`);
      out.push(key, path.resolve(folder));
    }
  }
  return out;
}

let ARGS;
try {
  ARGS = parseArgs(process.argv.slice(2));
} catch (error) {
  console.error(`dev-app-macos: ${error.message}\n\n${USAGE}`);
  process.exit(2);
}

const PROFILE = ARGS.release ? 'release' : 'debug';
const DATA = ARGS.data;
const BRIDGE = ARGS.bridge;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function until(what, fn, ms = 20000, every = 100) {
  const end = Date.now() + ms;
  for (;;) {
    const value = fn();
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
  } catch (error) {
    return error.code === 'EPERM';
  }
}

function signal(pid, name) {
  try {
    process.kill(pid, name);
  } catch (_) {
    // Already gone.
  }
}

// The pids of the processes whose executable is exactly this path. `ps` on macOS reports the full executable path in
// comm, so this never matches on a process name or on another Voice.
function processes(exe) {
  const out = spawnSync('ps', ['-ww', '-axo', 'pid=,comm='], { encoding: 'utf8' });
  if (out.status !== 0) throw new Error(`ps failed: ${(out.stderr || '').trim() || `exit ${out.status}`}`);
  return out.stdout
    .split('\n')
    .map((line) => /^\s*(\d+)\s+(.*?)\s*$/.exec(line))
    .filter((match) => match && path.resolve(match[2]) === exe)
    .map((match) => Number(match[1]))
    .filter((pid) => pid !== process.pid);
}

function sha256File(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function sha256Text(text) {
  return crypto.createHash('sha256').update(text).digest('hex');
}

function guard() {
  if (process.platform !== 'darwin') {
    throw new Error(`this builds a macOS app and runs on macOS only; this is ${process.platform}. Start a development copy with cargo and RUKOO_VOICE_DATA_DIR instead.`);
  }
  if (process.arch !== 'arm64') {
    throw new Error(`Rukoo Voice builds for Apple Silicon only; this is ${process.arch}.`);
  }
}

// ---------- build ----------

function targetDir() {
  return process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(VOICE, 'target');
}

function build() {
  const target = targetDir();
  const bin = path.join(target, PROFILE);
  if (!process.env.SHERPA_ONNX_ARCHIVE_DIR) {
    console.log(`SHERPA_ONNX_ARCHIVE_DIR isn't set, so the build uses the sherpa-onnx archive in ${path.join(target, 'sherpa-onnx-prebuilt')}; set it to share one copy between target folders`);
  }
  console.log(`building ${CRATES.join(', ')} (${PROFILE}) into ${target}`);
  const out = spawnSync('cargo', ['build', ...(ARGS.release ? ['--release'] : []), ...CRATES.flatMap((crate) => ['-p', crate])], {
    cwd: VOICE,
    stdio: 'inherit',
  });
  if (out.error) {
    const hint = out.error.code === 'ENOENT' ? ' Rust is in ~/.cargo/bin, which is not always on PATH.' : '';
    throw new Error(`couldn't run cargo: ${out.error.message}.${hint}`);
  }
  if (out.status !== 0) throw new Error(`cargo build failed with exit code ${out.status}`);
  for (const name of PROGRAMS) {
    const file = path.join(bin, name);
    if (!fs.existsSync(file)) throw new Error(`cargo reported success but ${file} isn't there`);
  }
  return bin;
}

function workspaceVersion() {
  const text = fs.readFileSync(path.join(VOICE, 'Cargo.toml'), 'utf8');
  const section = text.split(/^\[/m).find((part) => part.startsWith('workspace.package]'));
  const match = section && /^version\s*=\s*"([^"]+)"/m.exec(section);
  if (!match) throw new Error('voice/Cargo.toml has no [workspace.package] version');
  return match[1];
}

// The bundle's icon: an .icns from voice-app/icons, which the app's own icon script doesn't make yet. No icon is
// generated here; without one macOS shows its generic application icon.
function bundleIcon() {
  if (!fs.existsSync(ICONS)) return null;
  const names = fs.readdirSync(ICONS).filter((name) => name.endsWith('.icns')).sort();
  if (!names.length) return null;
  return names.includes('icon.icns') ? 'icon.icns' : names[0];
}

function renderPlist(version, icon) {
  const template = fs.readFileSync(APP_TEMPLATE, 'utf8');
  if (!template.includes('__VERSION__')) throw new Error(`${APP_TEMPLATE} has no __VERSION__ to fill in`);
  if (!template.includes(ICON_MARKER)) throw new Error(`${APP_TEMPLATE} has no line where CFBundleIconFile goes`);
  const iconKeys = icon ? `\t<key>CFBundleIconFile</key>\n\t<string>${icon}</string>\n` : '';
  return template.replaceAll('__VERSION__', version).replace(ICON_MARKER, iconKeys);
}

function inputs(bin, version, plist, icon) {
  return {
    version,
    profile: PROFILE,
    plist: sha256Text(plist),
    icon,
    entitlements: {
      audio: sha256File(ENTITLEMENTS_AUDIO),
      speakers: sha256File(ENTITLEMENTS_SPEAKERS),
    },
    built: Object.fromEntries(PROGRAMS.map((name) => [name, sha256File(path.join(bin, name))])),
  };
}

function readStamp() {
  try {
    return JSON.parse(fs.readFileSync(STAMP, 'utf8'));
  } catch (_) {
    return null;
  }
}

// Why the bundle has to be built again: an empty list means the copy and the signature that are there still cover
// this build, so the permissions macOS granted them still belong to the app.
function differences(stamp, want) {
  if (!stamp) return ['no earlier build'];
  const out = [];
  if (stamp.version !== want.version) out.push(`the version changed from ${stamp.version} to ${want.version}`);
  if (stamp.profile !== want.profile) out.push(`the profile changed from ${stamp.profile} to ${want.profile}`);
  if (stamp.plist !== want.plist) out.push('the rendered Info.plist changed');
  if (stamp.icon !== want.icon) out.push(want.icon ? `the icon is now ${want.icon}` : 'the icon went away');
  if (stamp.entitlements?.audio !== want.entitlements.audio) out.push('entitlements-audio.plist changed');
  if (stamp.entitlements?.speakers !== want.entitlements.speakers) out.push('entitlements-speakers.plist changed');
  for (const name of PROGRAMS) {
    if (stamp.built?.[name] !== want.built[name]) out.push(`${name} changed`);
    const copied = path.join(MACOS, name);
    if (!fs.existsSync(copied)) out.push(`${name} is missing from the bundle`);
    else if (sha256File(copied) !== stamp.signed?.[name]) out.push(`${name} in the bundle isn't the one that was signed`);
  }
  if (!fs.existsSync(path.join(CONTENTS, 'Info.plist'))) out.push('the bundle has no Info.plist');
  return out;
}

function codesign(target, entitlements, identifier) {
  const args = ['--force', '--sign', '-', '--options', 'runtime', '--timestamp=none'];
  if (identifier) args.push('--identifier', identifier);
  if (entitlements) args.push('--entitlements', entitlements);
  args.push(target);
  const out = spawnSync('codesign', args, { encoding: 'utf8' });
  if (out.status !== 0) throw new Error(`codesign failed for ${target}: ${(out.stderr || '').trim() || `exit ${out.status}`}`);
}

function entitlementsFor(name) {
  if (name === PROGRAM || name === 'rukoo-voice-recorder') return ENTITLEMENTS_AUDIO;
  if (name === 'rukoo-voice-speakers') return ENTITLEMENTS_SPEAKERS;
  return null;
}

// A program is written next to its destination and renamed over it in one step. A copy of this bundle that still
// runs (one the script didn't start, or a helper that outlived its app) keeps its old file and its valid signature
// instead of being killed when the pages under it change.
function replaceFile(dest, mode, write) {
  const temp = path.join(path.dirname(dest), `.${path.basename(dest)}.new`);
  write(temp);
  fs.chmodSync(temp, mode);
  fs.renameSync(temp, dest);
}

// A temporary file a run left behind would sit inside the bundle and break its seal.
function removeLeftovers(dir) {
  if (!fs.existsSync(dir)) return;
  for (const name of fs.readdirSync(dir)) {
    if (name.startsWith('.') && name.endsWith('.new')) fs.rmSync(path.join(dir, name));
  }
}

function leftBehind(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir).filter((name) => name.startsWith('.') && name.endsWith('.new'));
}

function rebuild(bin, plist, icon) {
  fs.mkdirSync(MACOS, { recursive: true });
  fs.mkdirSync(RESOURCES, { recursive: true });
  for (const dir of [CONTENTS, MACOS, RESOURCES]) removeLeftovers(dir);
  for (const name of PROGRAMS) {
    replaceFile(path.join(MACOS, name), 0o755, (temp) => fs.copyFileSync(path.join(bin, name), temp));
  }
  replaceFile(path.join(CONTENTS, 'Info.plist'), 0o644, (temp) => fs.writeFileSync(temp, plist));
  for (const old of fs.readdirSync(RESOURCES).filter((name) => name.endsWith('.icns'))) {
    if (old !== icon) fs.rmSync(path.join(RESOURCES, old));
  }
  if (icon) replaceFile(path.join(RESOURCES, icon), 0o644, (temp) => fs.copyFileSync(path.join(ICONS, icon), temp));
  else console.log(`voice-app/icons has no .icns, so ${APP_NAME}.app gets no icon and shows the generic one`);

  for (const dir of [CONTENTS, MACOS, RESOURCES]) {
    const stray = leftBehind(dir);
    if (stray.length) throw new Error(`a temporary file was left in ${dir}: ${stray.join(', ')}`);
  }

  for (const name of HELPERS) {
    const entitlements = entitlementsFor(name);
    const identifier = `${BUNDLE_ID}.${HELPER_IDS[name]}`;
    codesign(path.join(MACOS, name), entitlements, identifier);
    console.log(`  signed ${name} as ${identifier}${entitlements ? ` with ${path.basename(entitlements)}` : ''}`);
  }
  codesign(BUNDLE, entitlementsFor(PROGRAM));
  console.log(`  signed ${APP_NAME}.app with ${path.basename(ENTITLEMENTS_AUDIO)}`);

  const out = spawnSync('codesign', ['--verify', '--deep', '--strict', BUNDLE], { encoding: 'utf8' });
  if (out.status !== 0) throw new Error(`codesign --verify --deep --strict failed: ${(out.stderr || '').trim() || `exit ${out.status}`}`);
  console.log('codesign --verify --deep --strict passes');
}

function writeStamp(want) {
  const signed = Object.fromEntries(PROGRAMS.map((name) => [name, sha256File(path.join(MACOS, name))]));
  const stamp = { ...want, signed, at: new Date().toISOString() };
  fs.writeFileSync(STAMP, `${JSON.stringify(stamp, null, 2)}\n`);
}

// The permissions macOS gave an earlier build belong to that build's code signature, so a new one starts without
// them. Only the dev bundle's own entries are cleared, so System Settings doesn't fill up with entries that no
// longer apply.
function resetPermissions() {
  console.log(`clearing the permission entries of ${BUNDLE_ID} from earlier builds`);
  for (const service of TCC_SERVICES) {
    const out = spawnSync('tccutil', ['reset', service, BUNDLE_ID], { encoding: 'utf8' });
    const said = `${out.stdout ?? ''}${out.stderr ?? ''}`.trim().replace(/\s*\n\s*/g, ' ');
    if (out.status === 0) console.log(`  ${service}: cleared`);
    else if (/No such bundle identifier/i.test(said)) {
      console.log(`  ${service}: nothing recorded for ${BUNDLE_ID} yet`);
    } else {
      console.log(`  ${service}: tccutil said "${said || `exit ${out.status}`}"; carrying on with the rest`);
    }
  }
  console.log('switch these on again in System Settings when the app asks for them:');
  for (const pane of TCC_PANES) console.log(`  ${pane}`);
}

// ---------- run ----------

function bundleProcesses() {
  const found = [];
  for (const exe of [MAIN_EXE, ...HELPERS.map((name) => path.join(MACOS, name))]) {
    for (const pid of processes(exe)) if (!found.includes(pid)) found.push(pid);
  }
  return found;
}

async function terminate(pids) {
  for (const pid of pids) signal(pid, 'SIGTERM');
  const end = Date.now() + STOP_GRACE_MS;
  let left = pids.filter(alive);
  while (left.length && Date.now() < end) {
    await sleep(100);
    left = left.filter(alive);
  }
  if (!left.length) return;
  console.log(`still running after ${STOP_GRACE_MS / 1000} s, sending SIGKILL to ${left.join(', ')}`);
  for (const pid of left) signal(pid, 'SIGKILL');
  await sleep(500);
  left = left.filter(alive);
  if (left.length) throw new Error(`still running after SIGKILL: ${left.join(', ')}`);
}

function recordedApp() {
  try {
    const app = Number(JSON.parse(fs.readFileSync(PIDS, 'utf8')).app);
    return Number.isInteger(app) && app > 0 ? app : null;
  } catch (_) {
    return null;
  }
}

function forgetPids() {
  try {
    fs.rmSync(PIDS);
  } catch (_) {
    // Nothing was recorded for this bundle.
  }
}

// A build from an earlier run is replaced, so the copy that still runs it goes first. Only a pid this script
// recorded in pids.json counts, and only while it is the app inside this bundle.
async function stopEarlierRun() {
  const app = recordedApp();
  if (!app || !alive(app) || !processes(MAIN_EXE).includes(app)) return;
  console.log(`stopping the copy from the earlier build (pid ${app}) before replacing the bundle`);
  await terminate(bundleProcesses());
  forgetPids();
}

function openBundle(before) {
  fs.mkdirSync(DATA, { recursive: true });
  fs.mkdirSync(path.dirname(BRIDGE), { recursive: true });
  if (before.length) console.log(`another copy from this bundle runs as pid ${before.join(', ')}; starting one next to it`);
  const args = [
    '-n',
    '--env', `RUKOO_VOICE_DATA_DIR=${DATA}`,
    '--env', `RUKOO_VOICE_BRIDGE=${BRIDGE}`,
    BUNDLE,
  ];
  if (ARGS.forward.length) args.push('--args', ...ARGS.forward);
  console.log(`opening ${BUNDLE}`);
  const out = spawnSync('open', args, { encoding: 'utf8' });
  if (out.status !== 0) throw new Error(`open failed: ${(out.stderr || '').trim() || `exit ${out.status}`}`);
}

async function startApp() {
  const before = processes(MAIN_EXE);
  openBundle(before);
  const pid = await until(`the app from ${BUNDLE} to start`, () => processes(MAIN_EXE).find((one) => !before.includes(one)));
  const record = {
    bundle: BUNDLE,
    bundleId: BUNDLE_ID,
    startedAt: new Date().toISOString(),
    app: pid,
    data: DATA,
    bridge: BRIDGE,
    args: ARGS.forward,
    executables: [MAIN_EXE, ...HELPERS.map((name) => path.join(MACOS, name))],
  };
  fs.writeFileSync(PIDS, `${JSON.stringify(record, null, 2)}\n`);
  console.log(`${APP_NAME} runs as pid ${pid}`);
  console.log(`  data folder ${DATA}`);
  console.log(`  discovery file ${BRIDGE}`);
  console.log(`  log ${path.join(DATA, 'logs', 'app.log')}`);
  console.log('stop it with node voice/scripts/dev-app-macos.mjs --stop');
}

async function stopApp() {
  const targets = bundleProcesses();
  if (targets.length) {
    console.log(`stopping ${targets.length} process(es) from ${BUNDLE} by pid: ${targets.join(', ')}`);
    await terminate(targets);
    console.log('stopped');
  } else {
    console.log(`nothing runs from ${BUNDLE}`);
  }
  forgetPids();
}

async function main() {
  if (ARGS.help) {
    console.log(USAGE);
    return;
  }
  guard();
  if (ARGS.stop) {
    await stopApp();
    return;
  }
  fs.mkdirSync(DEV, { recursive: true });
  const bin = build();
  const version = workspaceVersion();
  const icon = bundleIcon();
  const plist = renderPlist(version, icon);
  const want = inputs(bin, version, plist, icon);
  const stamp = readStamp();
  const reasons = differences(stamp, want);
  if (ARGS.rebuild) reasons.unshift('--rebuild');
  if (!reasons.length) {
    console.log(`${BUNDLE} is unchanged since ${stamp.at}: skipping the copy and the signing, so its permissions stay`);
  } else {
    console.log(`rebuilding ${BUNDLE}: ${reasons.join('; ')}`);
    await stopEarlierRun();
    rebuild(bin, plist, icon);
    writeStamp(want);
    resetPermissions();
  }
  if (ARGS.noOpen) {
    console.log(`bundle ready: ${BUNDLE}`);
    return;
  }
  await startApp();
}

main().catch((error) => {
  console.error(`dev-app-macos: ${error.message}`);
  process.exitCode = 1;
});
