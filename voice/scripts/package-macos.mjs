// A release Rukoo Voice.app, a DMG, a zip and their checksums for macOS on Apple Silicon. The counterpart of
// voice/scripts/package.ps1.
//
//   node voice/scripts/package-macos.mjs --nemo-dir <folder> [--developer-id]
//   node voice/scripts/package-macos.mjs --no-speakers [--developer-id]
//
// It builds the five release programs, has Tauri's bundler make `Rukoo Voice.app` with the four helpers through
// `externalBin` (Tauri wants a `-aarch64-apple-darwin` suffix on the source names and strips it, so they land next to
// the app in Contents/MacOS, where Voice looks for them), adds the NeMo runtime at Contents/Resources/nemo_speech,
// signs everything ad-hoc with hardened runtime, and checks the result. Everything goes to
// <cargo target dir>/package:
//
//   Rukoo-Voice-<version>-arm64.dmg      the app and an /Applications link, made with hdiutil
//   Rukoo-Voice-<version>-arm64.zip      the app, made with ditto, which keeps the signature
//   SHA256SUMS.txt                       checksums of both, in the format of the Windows package
//
// The DMG is built with hdiutil instead of Tauri's `dmg` target: Tauri's DMG step drives Finder through AppleScript
// to lay out the window, which doesn't belong on a headless runner. The app itself comes from Tauri
// (`tauri.macos.conf.json` sets `targets: ["app"]`), so its Info.plist and resources are Tauri's.
//
// Signing is inside out, never `--deep`: the six NeMo dylibs first, then each helper with its own entitlements, then
// the app. `rukoo-voice` and `rukoo-voice-recorder` get `com.apple.security.device.audio-input`,
// `rukoo-voice-speakers` alone gets `com.apple.security.cs.disable-library-validation` (it dlopens the ad-hoc signed
// NeMo dylibs), and the other helpers get none. No App Sandbox and no JIT. The result is ad-hoc signed with hardened
// runtime and not notarized, as Rukoo Mail's Mac packages are, so the first launch needs Open Anyway in Privacy &
// Security. `--developer-id` signs with APPLE_SIGNING_IDENTITY and notarizes with `xcrun notarytool` instead, using
// APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID (the names docs/development.md calls MAC_CSC_LINK,
// MAC_CSC_KEY_PASSWORD, APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID).
//
// The NeMo runtime has no macOS download yet: another change adds manifest format 2 with a macos-aarch64 archive and
// `voice/nemo-runtime/build-macos.sh`. Until then the script takes a local runtime with `--nemo-dir`, and without it
// stops the way package.ps1 stops with "Publish the NeMo runtime first". `--no-speakers` makes a test package
// without the runtime and says so loudly.
//
// The checks, all of which fail the script, are the macOS counterparts of package.ps1's:
//   - the Info.plist keys (LSUIElement, the two usage strings, the category, the development region, the minimum
//     system version) with `plutil -extract`, and the English and Dutch InfoPlist.strings;
//   - the helpers' and the dylibs' placement, and that the app and dylibs carry exactly the entitlements above with
//     `codesign -d --entitlements`;
//   - every Mach-O's imports with `otool -L`: only /usr/lib, /System/Library and, for the NeMo dylibs, each other
//     through @loader_path, so nothing comes from Homebrew or a build folder;
//   - `minos 15.0` and arm64 on every binary with `vtool -show-build` and `lipo -info`;
//   - that rukoo-voice-screen, a headless helper, links no UI framework and no third-party media library, the
//     counterpart of package.ps1's "the screen process doesn't import Media Foundation" check;
//   - `codesign --verify --deep --strict` on the app, on the app extracted from the zip and on the app inside the
//     attached DMG, and that the DMG holds the app and an Applications link.

import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const VOICE = path.join(ROOT, 'voice');
const APP = path.join(VOICE, 'crates', 'voice-app');
const APP_NAME = 'Rukoo Voice';
const BUNDLE = `${APP_NAME}.app`;
const BUNDLE_ID = 'nl.bvdm.rukoo.voice';
const TRIPLE = 'aarch64-apple-darwin';
// The CLI voice/crates/voice-app/tauri.macos.conf.json was written against, the version package.ps1 pins.
const TAURI_CLI = '2.12.4';
const PROGRAM = 'rukoo-voice';
const HELPERS = ['rukoo-voice-recorder', 'rukoo-voice-local', 'rukoo-voice-speakers', 'rukoo-voice-screen'];
const HELPER_IDS = {
  'rukoo-voice-recorder': 'recorder',
  'rukoo-voice-local': 'local',
  'rukoo-voice-speakers': 'speakers',
  'rukoo-voice-screen': 'screen',
};
const NEMO_FOLDER = 'nemo_speech';
// The six dylibs voice/nemo-runtime/build-macos.sh builds, from spike S3.
const NEMO_DYLIBS = [
  'libnemo_speech_asr_c.dylib',
  'libnemo_speech_asr.dylib',
  'libggml.dylib',
  'libggml-base.dylib',
  'libggml-cpu.dylib',
  'libggml-blas.dylib',
];
const ENTITLEMENTS_AUDIO = path.join(APP, 'macos', 'entitlements-audio.plist');
const ENTITLEMENTS_SPEAKERS = path.join(APP, 'macos', 'entitlements-speakers.plist');
const AUDIO_INPUT = 'com.apple.security.device.audio-input';
const DISABLE_LIBRARY_VALIDATION = 'com.apple.security.cs.disable-library-validation';
const USAGE = `Usage: node voice/scripts/package-macos.mjs --nemo-dir <folder> [--developer-id]
       node voice/scripts/package-macos.mjs --no-speakers [--developer-id]

Builds ${BUNDLE}, a DMG, a zip and SHA256SUMS.txt in voice/target/package, ad-hoc signed with
hardened runtime and not notarized.

Options:
  --nemo-dir <folder>  a local NeMo runtime (the six dylibs, runtime.json and licenses/) to ship in
                       Contents/Resources/${NEMO_FOLDER}; required until the macOS runtime is published
  --no-speakers        package without speaker labels, for testing only; says so loudly
  --developer-id       sign with APPLE_SIGNING_IDENTITY and notarize with APPLE_ID, APPLE_PASSWORD
                       and APPLE_TEAM_ID instead of ad-hoc
  -h, --help           this text`;

function parseArgs(argv) {
  const out = { nemoDir: null, noSpeakers: false, developerId: false, help: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const eq = arg.indexOf('=');
    const key = eq > 0 ? arg.slice(0, eq) : arg;
    const value = eq > 0 ? arg.slice(eq + 1) : null;
    switch (key) {
      case '-h':
      case '--help':
        out.help = true;
        break;
      case '--nemo-dir': {
        const taken = value ?? argv[++i];
        if (!taken) throw new Error('--nemo-dir needs a folder');
        out.nemoDir = path.resolve(taken);
        break;
      }
      case '--no-speakers':
        out.noSpeakers = true;
        break;
      case '--developer-id':
        out.developerId = true;
        break;
      default:
        throw new Error(`unknown option ${arg}`);
    }
  }
  if (out.nemoDir && out.noSpeakers) throw new Error('--nemo-dir and --no-speakers contradict each other');
  return out;
}

let ARGS;
try {
  ARGS = parseArgs(process.argv.slice(2));
} catch (error) {
  console.error(`package-macos: ${error.message}\n\n${USAGE}`);
  process.exit(2);
}

function guard() {
  if (process.platform !== 'darwin') {
    throw new Error(`this packages a macOS app and runs on macOS only; this is ${process.platform}`);
  }
  if (process.arch !== 'arm64') {
    throw new Error(`Rukoo Voice ships for Apple Silicon only; this is ${process.arch}`);
  }
}

function run(file, args, options = {}) {
  const result = spawnSync(file, args, {
    encoding: 'utf8',
    stdio: options.inherit ? 'inherit' : 'pipe',
    input: options.input,
    cwd: options.cwd,
    env: options.env ?? process.env,
  });
  if (result.error) {
    const hint = result.error.code === 'ENOENT' && file === 'cargo'
      ? ' Rust is in ~/.cargo/bin, which is not always on PATH.'
      : '';
    throw new Error(`couldn't run ${file}: ${result.error.message}.${hint}`);
  }
  if (result.status !== 0 && !options.allowFailure) {
    const said = `${result.stdout ?? ''}${result.stderr ?? ''}`.trim();
    throw new Error(`${file} ${args.join(' ')} failed with exit code ${result.status}${said ? `: ${said}` : ''}`);
  }
  return result;
}

function step(text) {
  console.log(`\n${text}`);
}

// A check that passed, with what it found. Any failure throws out of main, which fails the script.
function ok(what, detail) {
  console.log(`  ${what}: ${detail}`);
}

function sha256File(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function targetDir() {
  return process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(VOICE, 'target');
}

function workspaceVersion() {
  const text = fs.readFileSync(path.join(VOICE, 'Cargo.toml'), 'utf8');
  const section = text.split(/^\[/m).find((part) => part.startsWith('workspace.package]'));
  const match = section && /^version\s*=\s*"([^"]+)"/m.exec(section);
  if (!match) throw new Error('voice/Cargo.toml has no [workspace.package] version');
  return match[1];
}

// cargo and the C compiler take the deployment target from the environment; Tauri sets it from
// tauri.macos.conf.json only for the app it builds. The build's C links also need the Xcode SDK, not the Command
// Line Tools one this Mac has.
function buildEnv() {
  const env = { ...process.env };
  env.MACOSX_DEPLOYMENT_TARGET = '15.0';
  if (!env.SDKROOT) {
    const sdk = run('xcrun', ['--sdk', 'macosx', '--show-sdk-path'], { allowFailure: true });
    if (sdk.status === 0 && sdk.stdout.trim()) env.SDKROOT = sdk.stdout.trim();
  }
  return env;
}

// ---------- the runtime ----------

// Where voice-core's speaker lookup finds the runtime. On macOS it is not here yet: `locate_runtime` looks in
// `<exe dir>/nemo_speech` and owns the process only when `PLATFORM_SUPPORTED` covers macOS, and `verify_voice` accepts
// manifest format 1 only. The speaker task makes Contents/Resources/nemo_speech (exe_dir/../Resources/nemo_speech) a
// folder it searches and teaches it the format-2 macos-aarch64 file list. Until then this runtime is packaged and
// signed but a release copy won't find it, which this script says out loud rather than hiding.
function resolveRuntime() {
  if (ARGS.noSpeakers) {
    console.warn('');
    console.warn('  WARNING: --no-speakers packages Voice without the NeMo runtime and without speaker labels.');
    console.warn('  Every meeting will keep the whole call as one speaker. This package is for testing only.');
    return null;
  }
  if (!ARGS.nemoDir) {
    throw new Error(
      'The macOS NeMo runtime isn\'t published yet (another change adds manifest format 2 with a macos-aarch64 '
      + 'archive). Build one with voice/nemo-runtime/build-macos.sh, or pass --nemo-dir with a folder holding the six '
      + 'dylibs and runtime.json, such as the nemo_speech folder of a local build. For a test package without speaker '
      + 'labels, pass --no-speakers.',
    );
  }
  const dir = ARGS.nemoDir;
  if (!fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) {
    throw new Error(`${dir} isn't a folder`);
  }
  return { dir, ...verifyRuntime(dir) };
}

// Checks a local runtime against its own runtime.json, the way package.ps1 checks the downloaded folder against the
// committed manifest: every recorded file at its size and hash, the six dylibs, and nothing else but the licences.
function verifyRuntime(dir) {
  const runtimePath = path.join(dir, 'runtime.json');
  if (!fs.existsSync(runtimePath)) throw new Error(`${dir} has no runtime.json`);
  let runtime;
  try {
    runtime = JSON.parse(fs.readFileSync(runtimePath, 'utf8'));
  } catch (error) {
    throw new Error(`${runtimePath} isn't JSON: ${error.message}`);
  }
  if (!runtime.commit) throw new Error(`${runtimePath} names no NeMo-Speech.cpp commit`);
  const missing = NEMO_DYLIBS.filter((name) => !fs.existsSync(path.join(dir, name)));
  if (missing.length) throw new Error(`${dir} is missing ${missing.join(', ')}`);
  const top = fs.readdirSync(dir).sort();
  const allowed = new Set([...NEMO_DYLIBS, 'runtime.json', 'licenses']);
  const unexpected = top.filter((name) => !allowed.has(name));
  if (unexpected.length) throw new Error(`${dir} also holds ${unexpected.join(', ')}, which a runtime doesn't`);
  const licenses = path.join(dir, 'licenses');
  if (!fs.existsSync(licenses) || !fs.statSync(licenses).isDirectory() || fs.readdirSync(licenses).length === 0) {
    throw new Error(`${dir} has no licenses folder`);
  }
  if (!Array.isArray(runtime.files) || runtime.files.length === 0) {
    throw new Error(`${runtimePath} lists no files`);
  }
  for (const file of runtime.files) {
    const full = path.join(dir, file.path);
    if (!fs.existsSync(full)) throw new Error(`${file.path} is missing`);
    if (fs.statSync(full).size !== file.size) throw new Error(`${file.path} is not ${file.size} bytes`);
    if (sha256File(full) !== `${file.sha256}`.toLowerCase()) throw new Error(`${file.path} differs from runtime.json`);
  }
  return { name: runtime.name ?? 'nemo runtime', commit: runtime.commit, files: runtime.files.length };
}

// ---------- build ----------

function buildHelpers(env) {
  if (!process.env.SHERPA_ONNX_ARCHIVE_DIR) {
    console.log(`SHERPA_ONNX_ARCHIVE_DIR isn't set, so the build fetches sherpa-onnx into ${path.join(targetDir(), 'sherpa-onnx-prebuilt')}; set it to share one copy between target folders`);
  }
  console.log(`building ${HELPERS.join(', ')} (release) into ${targetDir()}`);
  run('cargo', ['build', '--release', '--locked', ...HELPERS.flatMap((name) => ['-p', crateFor(name)])], {
    cwd: VOICE,
    env,
    inherit: true,
  });
  const release = path.join(targetDir(), 'release');
  for (const name of HELPERS) {
    if (!fs.existsSync(path.join(release, name))) throw new Error(`cargo reported success but ${path.join(release, name)} isn't there`);
  }
  return release;
}

function crateFor(program) {
  return program.replace(/^rukoo-/, '');
}

// Tauri's externalBin wants each program with the target triple on its source name and puts it in Contents/MacOS
// under the plain name, where Voice looks for it next to itself.
function stageHelpers(release, out) {
  const staging = path.join(out, 'sidecars');
  fs.mkdirSync(staging, { recursive: true });
  const externalBin = [];
  for (const name of HELPERS) {
    const source = path.join(release, name);
    run('ditto', [source, path.join(staging, `${name}-${TRIPLE}`)]);
    externalBin.push(path.join(staging, name));
  }
  return externalBin;
}

// APP's platform config is merged over tauri.conf.json on macOS. The helper list goes only in this package-time patch,
// as package.ps1 does on Windows: tauri-build checks bundle.externalBin at compile time, so a committed list would
// make every ordinary `cargo build` of voice-app fail until package-macos.mjs has staged the copies.
function writePatch(out, externalBin) {
  const patch = { bundle: { targets: ['app'], externalBin } };
  const file = path.join(out, 'tauri.package.json');
  fs.writeFileSync(file, `${JSON.stringify(patch, null, 2)}\n`);
  return file;
}

function bundleApp(patch) {
  console.log(`building the app with @tauri-apps/cli ${TAURI_CLI}`);
  run('npx', ['--yes', `@tauri-apps/cli@${TAURI_CLI}`, 'build', '--bundles', 'app', '--config', patch, '--ci'], {
    cwd: APP,
    env: buildEnv(),
    inherit: true,
  });
}

// ---------- bundle contents ----------

// The runtime is a folder of dylibs next to runtime.json and the licences, as on Windows. It lives in Resources and
// not in Frameworks because codesign reads runtime.json and the licences under Frameworks as unsigned nested code.
function addRuntime(appDir, runtime) {
  const resources = path.join(appDir, 'Contents', 'Resources', NEMO_FOLDER);
  fs.mkdirSync(path.dirname(resources), { recursive: true });
  run('ditto', [runtime.dir, resources]);
  // The copied files are what ships; check them against runtime.json again rather than trusting the copy.
  verifyRuntime(resources);
  console.log(`  ${NEMO_FOLDER}: ${runtime.name} (${runtime.commit.slice(0, 7)}), ${runtime.files} files checked against runtime.json`);
  console.log(`  note: voice-core does not search Contents/Resources/nemo_speech yet and speaker labels are Windows-only in this build; the speaker task changes that lookup`);
}

// ---------- signing ----------

function identity() {
  if (!ARGS.developerId) return '-';
  const missing = ['APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID']
    .filter((name) => !process.env[name]);
  if (missing.length) {
    throw new Error(
      `--developer-id needs ${missing.join(', ')} in the environment. docs/development.md names the repository `
      + 'secrets MAC_CSC_LINK, MAC_CSC_KEY_PASSWORD, APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD and APPLE_TEAM_ID; the '
      + 'identity is the Developer ID Application certificate from the P12.',
    );
  }
  return process.env.APPLE_SIGNING_IDENTITY;
}

function codesign(target, { entitlements, identifier, signer }) {
  const args = ['--force', '--sign', signer, '--options', 'runtime'];
  // An ad-hoc signature can't be timestamped; a Developer ID one must be, for notarization.
  args.push(signer === '-' ? '--timestamp=none' : '--timestamp');
  if (identifier) args.push('--identifier', identifier);
  if (entitlements) args.push('--entitlements', entitlements);
  args.push(target);
  run('codesign', args);
}

function signAll(appDir, runtime, signer) {
  if (runtime) {
    // The speaker worker loads these through dlopen with library validation off, so they are ad-hoc signed and
    // hardened too. They go first: signing must run inside out.
    for (const name of NEMO_DYLIBS) {
      codesign(path.join(appDir, 'Contents', 'Resources', NEMO_FOLDER, name), { signer });
      console.log(`  signed ${name}`);
    }
  }
  for (const name of HELPERS) {
    const entitlements = name === 'rukoo-voice-recorder'
      ? ENTITLEMENTS_AUDIO
      : name === 'rukoo-voice-speakers' ? ENTITLEMENTS_SPEAKERS : null;
    const identifier = `${BUNDLE_ID}.${HELPER_IDS[name]}`;
    codesign(path.join(appDir, 'Contents', 'MacOS', name), { entitlements, identifier, signer });
    console.log(`  signed ${name} as ${identifier}${entitlements ? ` with ${path.basename(entitlements)}` : ''}`);
  }
  codesign(appDir, { entitlements: ENTITLEMENTS_AUDIO, signer });
  console.log(`  signed ${BUNDLE} with ${path.basename(ENTITLEMENTS_AUDIO)}`);
}

// ---------- checks ----------

function plistValue(file, key) {
  const out = run('plutil', ['-extract', key, 'raw', '-o', '-', file]);
  return out.stdout.replace(/\n$/, '');
}

function signatureInfo(target) {
  const out = run('codesign', ['-d', '-vv', target], { allowFailure: true });
  const text = `${out.stdout ?? ''}${out.stderr ?? ''}`;
  if (out.status !== 0) throw new Error(`codesign could not read ${target}:\n${text.trim()}`);
  return text;
}

// The entitlements a signed target carries, as a sorted list of keys. An empty signature has none, and codesign
// prints them as an XML plist.
function entitlementsOf(target) {
  const out = run('codesign', ['-d', '--entitlements', ':-', target], { allowFailure: true });
  const text = `${out.stdout ?? ''}`.trim();
  if (!text) {
    if (out.status !== 0) throw new Error(`codesign could not read the entitlements of ${target}`);
    return [];
  }
  const converted = run('plutil', ['-convert', 'json', '-o', '-', '-'], { input: text });
  let parsed;
  try {
    parsed = JSON.parse(converted.stdout);
  } catch (error) {
    throw new Error(`codesign printed entitlements for ${target} that aren't a plist: ${error.message}`);
  }
  for (const [key, value] of Object.entries(parsed)) {
    if (value !== true) throw new Error(`${target} has ${key} set to ${JSON.stringify(value)} instead of true`);
  }
  return Object.keys(parsed).sort();
}

function assertSignature(target, label, { signer, entitlements, identifier }) {
  const info = signatureInfo(target);
  if (signer === '-') {
    if (!/^Signature=adhoc$/m.test(info)) throw new Error(`${label} is not ad-hoc signed`);
    if (!/flags=0x[0-9a-f]+\([^)]*\bruntime\b/m.test(info)) throw new Error(`${label} doesn't have the hardened runtime flag`);
  } else if (!/^Authority=Developer ID Application/m.test(info)) {
    throw new Error(`${label} isn't signed by a Developer ID Application certificate`);
  }
  if (identifier) {
    const found = /^Identifier=(.+)$/m.exec(info);
    if (!found) throw new Error(`${label} has no identifier`);
    if (found[1] !== identifier) throw new Error(`${label} has identifier ${found[1]} instead of ${identifier}`);
  }
  const actual = entitlementsOf(target);
  if (actual.join(',') !== [...entitlements].sort().join(',')) {
    throw new Error(`${label} has entitlements [${actual.join(', ')}] instead of [${[...entitlements].sort().join(', ')}]`);
  }
}

function assertPlist(appDir, version) {
  const plist = path.join(appDir, 'Contents', 'Info.plist');
  const expected = [
    ['CFBundleIdentifier', BUNDLE_ID],
    ['CFBundleExecutable', PROGRAM],
    ['CFBundleShortVersionString', version],
    ['LSUIElement', 'true'],
    ['CFBundleDevelopmentRegion', 'en'],
    ['LSApplicationCategoryType', 'public.app-category.productivity'],
    ['LSMinimumSystemVersion', '15.0'],
  ];
  for (const [key, value] of expected) {
    const actual = plistValue(plist, key);
    if (actual !== value) throw new Error(`Info.plist has ${key} ${JSON.stringify(actual)} instead of ${JSON.stringify(value)}`);
  }
  const mic = plistValue(plist, 'NSMicrophoneUsageDescription');
  const audio = plistValue(plist, 'NSAudioCaptureUsageDescription');
  for (const [key, value] of [['NSMicrophoneUsageDescription', mic], ['NSAudioCaptureUsageDescription', audio]]) {
    if (!value || !value.includes(APP_NAME)) throw new Error(`Info.plist ${key} is missing or doesn't name ${APP_NAME}`);
  }
  ok('Info.plist', `${expected.length} keys and both usage strings`);
  return { mic, audio };
}

// The English and Dutch texts macOS uses for the permission prompts. plutil reads .strings files as plists.
function assertLocalizedStrings(appDir, english) {
  const resources = path.join(appDir, 'Contents', 'Resources');
  const keys = ['NSMicrophoneUsageDescription', 'NSAudioCaptureUsageDescription'];
  const values = {};
  for (const language of ['en', 'nl']) {
    const file = path.join(resources, `${language}.lproj`, 'InfoPlist.strings');
    if (!fs.existsSync(file)) throw new Error(`the bundle has no ${language}.lproj/InfoPlist.strings`);
    run('plutil', ['-lint', file]);
    values[language] = {};
    for (const key of keys) {
      const value = plistValue(file, key);
      if (!value) throw new Error(`${language}.lproj has no ${key}`);
      values[language][key] = value;
    }
  }
  for (const key of keys) {
    if (values.en[key] !== english[key === 'NSMicrophoneUsageDescription' ? 'mic' : 'audio']) {
      throw new Error(`en.lproj's ${key} differs from Info.plist`);
    }
    if (values.nl[key] === values.en[key]) throw new Error(`nl.lproj's ${key} is not translated`);
  }
  ok('InfoPlist.strings', 'en and nl carry both usage strings, and nl differs from en');
}

function importsOf(file) {
  const out = run('otool', ['-L', file]);
  return out.stdout
    .split('\n')
    .slice(1)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => line.replace(/ \(compatibility version.*$/, ''));
}

// Every Mach-O may import only the system's libraries. The NeMo dylibs may also import each other through
// @loader_path, which must point at a file in the same folder.
function assertImports(file, label, localDylibs) {
  const imports = importsOf(file);
  const local = localDylibs ? new Set(localDylibs.map((name) => `@loader_path/${name}`)) : new Set();
  const bad = imports.filter((entry) => !entry.startsWith('/usr/lib/') && !entry.startsWith('/System/Library/') && !local.has(entry));
  if (bad.length) throw new Error(`${label} imports ${bad.join(', ')}, which is not the system or a bundled dylib`);
  for (const entry of imports) {
    if (entry.startsWith('@loader_path/')) {
      const name = entry.slice('@loader_path/'.length);
      if (!fs.existsSync(path.join(path.dirname(file), name))) throw new Error(`${label} imports ${entry}, which isn't next to it`);
    }
  }
  return imports;
}

function assertMachO(file, label) {
  const described = run('file', ['-b', file]).stdout.trim();
  if (!described.includes('Mach-O 64-bit')) throw new Error(`${label} is not a 64-bit Mach-O file (${described})`);
  if (described.includes('x86_64') || described.includes('i386') || described.includes('arm64e')) {
    throw new Error(`${label} is not plain arm64 (${described})`);
  }
  const arch = run('lipo', ['-info', file]).stdout.trim();
  if (!/is architecture: arm64$/.test(arch)) throw new Error(`${label} is not arm64 (${arch})`);
  const build = run('vtool', ['-show-build', file]).stdout;
  const minos = /minos\s+(\S+)/.exec(build);
  if (!minos) throw new Error(`${label} has no minimum system version`);
  if (minos[1] !== '15.0') throw new Error(`${label} has minos ${minos[1]} instead of 15.0`);
  return described;
}

// package.ps1 checks that rukoo-voice-screen doesn't import Media Foundation at load time, so it starts on a Windows
// without it and records frames. The macOS counterpart: the screen process is a headless helper and must not pull in
// the UI frameworks the app needs, nor a third-party media library it would have to ship. It may use system frameworks
// such as ScreenCaptureKit and VideoToolbox when it records.
function assertScreenImports(file) {
  const imports = importsOf(file);
  const forbidden = imports.filter((entry) => /(AppKit|Cocoa|SwiftUI|WebKit)\.framework/.test(entry)
    || /\/(lib(avcodec|avformat|avutil|swscale|swresample|x264|x265|vpx|opencv)[^/]*)$/.test(entry)
    || entry.includes('/opt/homebrew/') || entry.includes('/usr/local/'));
  if (forbidden.length) throw new Error(`rukoo-voice-screen links ${forbidden.join(', ')}, which a headless helper shouldn't`);
  if (!imports.some((entry) => entry.startsWith('/usr/lib/libSystem'))) throw new Error('rukoo-voice-screen does not link libSystem');
  return imports.length;
}

function assertBundle(appDir, version, runtime, signer) {
  const macos = path.join(appDir, 'Contents', 'MacOS');
  const placements = fs.readdirSync(macos).sort();
  const expectedPlacements = [PROGRAM, ...HELPERS].sort();
  if (placements.join(',') !== expectedPlacements.join(',')) {
    throw new Error(`Contents/MacOS holds ${placements.join(', ')} instead of ${expectedPlacements.join(', ')}`);
  }
  for (const name of [PROGRAM, ...HELPERS]) {
    const file = path.join(macos, name);
    if ((fs.statSync(file).mode & 0o111) === 0) throw new Error(`${name} is not executable`);
    const described = assertMachO(file, name);
    const imports = assertImports(file, name, null);
    if (name === 'rukoo-voice-screen') assertScreenImports(file);
    ok(name, `${described.replace('Mach-O 64-bit ', '')}, ${imports.length} imports, minos 15.0`);
  }
  ok('helpers', 'next to the app in Contents/MacOS');

  const english = assertPlist(appDir, version);
  assertLocalizedStrings(appDir, english);

  assertSignature(appDir, BUNDLE, { signer, entitlements: [AUDIO_INPUT], identifier: BUNDLE_ID });
  ok('entitlements', `${APP_NAME}: ${AUDIO_INPUT}`);
  for (const name of HELPERS) {
    const entitlements = name === 'rukoo-voice-recorder'
      ? [AUDIO_INPUT]
      : name === 'rukoo-voice-speakers' ? [DISABLE_LIBRARY_VALIDATION] : [];
    assertSignature(path.join(macos, name), name, {
      signer,
      entitlements,
      identifier: `${BUNDLE_ID}.${HELPER_IDS[name]}`,
    });
    ok('entitlements', `${name}: ${entitlements.join(', ') || 'none'}`);
  }

  if (runtime) {
    const folder = path.join(appDir, 'Contents', 'Resources', NEMO_FOLDER);
    const files = fs.readdirSync(folder).sort();
    const expected = [...NEMO_DYLIBS, 'licenses', 'runtime.json'].sort();
    if (files.join(',') !== expected.join(',')) throw new Error(`Contents/Resources/${NEMO_FOLDER} holds ${files.join(', ')}`);
    for (const name of NEMO_DYLIBS) {
      const file = path.join(folder, name);
      assertMachO(file, name);
      assertImports(file, name, NEMO_DYLIBS);
      assertSignature(file, name, { signer, entitlements: [] });
    }
    ok(NEMO_FOLDER, `6 dylibs signed, arm64, minos 15.0, imports only the system and each other`);
  } else {
    if (fs.existsSync(path.join(appDir, 'Contents', 'Resources', NEMO_FOLDER))) {
      throw new Error(`Contents/Resources/${NEMO_FOLDER} is there although --no-speakers was given`);
    }
    ok('speaker labels', 'left out (--no-speakers)');
  }

  run('codesign', ['--verify', '--deep', '--strict', appDir]);
  ok('signature', 'codesign --verify --deep --strict passes');
}

// ---------- packaging ----------

function writeZip(appDir, zip) {
  fs.rmSync(zip, { force: true });
  run('ditto', ['-c', '-k', '--keepParent', appDir, zip]);
}

function writeDmg(appDir, dmg, out) {
  const stage = path.join(out, 'dmg');
  fs.rmSync(stage, { recursive: true, force: true });
  fs.mkdirSync(stage, { recursive: true });
  run('ditto', [appDir, path.join(stage, BUNDLE)]);
  fs.symlinkSync('/Applications', path.join(stage, 'Applications'));
  fs.rmSync(dmg, { force: true });
  run('hdiutil', ['create', '-volname', APP_NAME, '-srcfolder', stage, '-ov', '-format', 'UDZO', dmg]);
  fs.rmSync(stage, { recursive: true, force: true });
}

// The zip has to unpack to a bundle whose signature still verifies: ditto keeps the signature, a plain zip tool can
// drop resource forks and the code directory.
function checkZip(zip, out) {
  const folder = path.join(out, 'zip-check');
  fs.rmSync(folder, { recursive: true, force: true });
  try {
    run('ditto', ['-x', '-k', zip, folder]);
    const appDir = path.join(folder, BUNDLE);
    if (!fs.existsSync(appDir)) throw new Error(`the zip does not hold ${BUNDLE}`);
    run('codesign', ['--verify', '--deep', '--strict', appDir]);
    ok('zip', `unpacks to ${BUNDLE}, whose signature verifies`);
  } finally {
    fs.rmSync(folder, { recursive: true, force: true });
  }
}

// The DMG, checked the way package.ps1 checks the installer: mounted, not installed.
function checkDmg(dmg, out) {
  const mount = path.join(out, 'dmg-check');
  fs.mkdirSync(mount, { recursive: true });
  let attached = false;
  try {
    run('hdiutil', ['attach', '-nobrowse', '-readonly', '-mountpoint', mount, dmg]);
    attached = true;
    const entries = fs.readdirSync(mount).sort();
    if (entries.join(',') !== ['Applications', BUNDLE].sort().join(',')) {
      throw new Error(`the DMG holds ${entries.join(', ')} instead of ${BUNDLE} and an Applications link`);
    }
    const link = fs.readlinkSync(path.join(mount, 'Applications'));
    if (link !== '/Applications') throw new Error(`the DMG's Applications link points at ${link}`);
    run('codesign', ['--verify', '--deep', '--strict', path.join(mount, BUNDLE)]);
    ok('DMG', `holds ${BUNDLE} and an Applications link, and the app inside verifies`);
  } finally {
    if (attached) spawnSync('hdiutil', ['detach', mount, '-quiet'], { encoding: 'utf8' });
    fs.rmSync(mount, { recursive: true, force: true });
  }
}

function writeSums(files, out) {
  const lines = files.map((file) => `${sha256File(file)}  ${path.basename(file)}`);
  const sums = path.join(out, 'SHA256SUMS.txt');
  fs.writeFileSync(sums, `${lines.join('\n')}\n`);
  return sums;
}

// Notarization: submit and wait, then staple the ticket. A ticket can be stapled to the app and to the DMG, not to
// the zip, so the app is notarized through the zip first and the DMG made from the stapled app after.
function notarize(file) {
  run('xcrun', ['notarytool', 'submit', file,
    '--apple-id', process.env.APPLE_ID,
    '--password', process.env.APPLE_PASSWORD,
    '--team-id', process.env.APPLE_TEAM_ID,
    '--wait'], { inherit: true });
}

function staple(target) {
  run('xcrun', ['stapler', 'staple', target], { inherit: true });
}

// ---------- main ----------

async function main() {
  if (ARGS.help) {
    console.log(USAGE);
    return;
  }
  guard();
  const signer = identity();
  const version = workspaceVersion();
  const target = targetDir();
  const out = path.join(target, 'package');
  const release = path.join(target, 'release');
  console.log(`Rukoo Voice ${version}${signer === '-' ? ', ad-hoc signed' : `, signed as ${signer}`}`);

  const runtime = resolveRuntime();
  if (runtime && (runtime.dir === out || runtime.dir.startsWith(`${out}${path.sep}`))) {
    throw new Error(`--nemo-dir ${runtime.dir} is inside ${out}, which this script empties; keep the runtime elsewhere`);
  }
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });

  step('Building the release programs');
  const bin = buildHelpers(buildEnv());
  const externalBin = stageHelpers(bin, out);
  const patch = writePatch(out, externalBin);

  step(`Building ${BUNDLE} with Tauri`);
  bundleApp(patch);
  const builtApp = path.join(release, 'bundle', 'macos', BUNDLE);
  if (!fs.existsSync(builtApp)) throw new Error(`the bundler made no ${builtApp}`);
  const appDir = path.join(out, BUNDLE);
  fs.renameSync(builtApp, appDir);

  if (runtime) {
    step('Adding the NeMo runtime');
    addRuntime(appDir, runtime);
  }

  step(`Signing ${signer === '-' ? 'ad-hoc with hardened runtime' : `as ${signer} with hardened runtime`}`);
  signAll(appDir, runtime, signer);

  step('Checking the bundle');
  assertBundle(appDir, version, runtime, signer);

  const stem = `Rukoo-Voice-${version}-arm64`;
  const dmg = path.join(out, `${stem}.dmg`);
  const zip = path.join(out, `${stem}.zip`);

  step('Writing the zip and the DMG');
  writeZip(appDir, zip);
  writeDmg(appDir, dmg, out);
  checkZip(zip, out);
  checkDmg(dmg, out);

  if (ARGS.developerId) {
    step('Notarizing');
    notarize(zip);
    staple(appDir);
    step('Rebuilding the zip and the DMG from the stapled app');
    writeZip(appDir, zip);
    writeDmg(appDir, dmg, out);
    notarize(dmg);
    staple(dmg);
  }

  const sums = writeSums([dmg, zip], out);

  console.log('');
  console.log(signer === '-' ? 'ad-hoc signed, not notarized' : 'Developer ID signed and notarized');
  for (const file of [dmg, zip, sums]) {
    console.log(`  ${(fs.statSync(file).size / 1024 / 1024).toFixed(1)} MB  ${file}`);
  }
}

main().catch((error) => {
  console.error(`package-macos: ${error.message}`);
  process.exitCode = 1;
});
