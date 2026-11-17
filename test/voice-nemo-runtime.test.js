'use strict';

// voice/nemo-runtime/pack-macos.mjs and the runtime path of voice/scripts/package-macos.mjs. The manifest has to pin
// the dylibs as the app carries them, so these tests pack a small fixture runtime with the real codesign, read the
// manifest and the zip back, and drive package-macos's download-and-check path against a file:// archive. Apple
// Silicon macOS only, where codesign and clang are; the fixture dylibs must be native Mach-O files for codesign.

const test = require('node:test');
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');
const { pathToFileURL } = require('node:url');

const ROOT = path.join(__dirname, '..');
const PACK = path.join(ROOT, 'voice', 'nemo-runtime', 'pack-macos.mjs');
const PACKAGE = path.join(ROOT, 'voice', 'scripts', 'package-macos.mjs');
const MANIFEST = path.join(ROOT, 'voice', 'nemo-runtime', 'manifest.json');
const LIB = pathToFileURL(path.join(ROOT, 'voice', 'scripts', 'lib', 'nemo-runtime.mjs')).href;
const SIGN_LIB = pathToFileURL(path.join(ROOT, 'voice', 'scripts', 'lib', 'macos-sign.mjs')).href;
const DYLIBS = [
  'libnemo_speech_asr_c.dylib',
  'libnemo_speech_asr.dylib',
  'libggml.dylib',
  'libggml-base.dylib',
  'libggml-cpu.dylib',
  'libggml-blas.dylib',
];
const LICENSES = [
  'licenses/LICENSE',
  'licenses/NOTICE',
  'licenses/THIRD_PARTY_NOTICES.md',
  'licenses/sentencepiece/LICENSE',
  'licenses/sentencepiece/absl-LICENSE',
  'licenses/sentencepiece/darts-clone-LICENSE',
  'licenses/sentencepiece/protobuf-lite-LICENSE',
];
const SKIP = process.platform !== 'darwin'
  ? 'codesign runs on macOS'
  : process.arch !== 'arm64' ? 'the NeMo runtime is arm64' : false;

function folder(t, name) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `rukoo-${name}-`));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

function sha256(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function listFiles(dir) {
  const found = [];
  const walk = (next, prefix) => {
    for (const entry of fs.readdirSync(next, { withFileTypes: true })) {
      const full = path.join(next, entry.name);
      const name = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(full, name);
      else found.push(name);
    }
  };
  walk(dir, '');
  return found.sort();
}

function run(file, args, options = {}) {
  return spawnSync(file, args, { encoding: 'utf8', ...options });
}

function node(args, options = {}) {
  return run(process.execPath, args, options);
}

function pins() {
  return JSON.parse(fs.readFileSync(MANIFEST, 'utf8'));
}

// A runtime folder shaped like build-macos.sh's output: one small native dylib copied to the six names, the licences
// and a format 1 runtime.json that records the built, unsigned file hashes.
function runtime(dir, { target = 'macos-aarch64', commit } = {}) {
  const pinned = pins();
  const source = path.join(dir, 'fixture.c');
  const built = path.join(dir, 'fixture.dylib');
  fs.writeFileSync(source, 'int fixture(void) { return 1; }\n');
  const compile = run('clang', ['-dynamiclib', '-o', built, source]);
  assert.equal(compile.status, 0, compile.stderr);
  fs.rmSync(source);
  for (const name of DYLIBS) fs.copyFileSync(built, path.join(dir, name));
  fs.rmSync(built);
  for (const name of LICENSES) {
    fs.mkdirSync(path.dirname(path.join(dir, name)), { recursive: true });
    fs.writeFileSync(path.join(dir, name), `fixture license ${name}\n`);
  }
  const files = listFiles(dir)
    .filter((name) => name !== 'runtime.json')
    .map((name) => ({
      path: name,
      size: fs.statSync(path.join(dir, name)).size,
      sha256: sha256(path.join(dir, name)),
    }));
  const doc = {
    format: 1,
    target,
    name: pinned.name,
    repository: pinned.repository,
    commit: commit ?? pinned.commit,
    submodules: pinned.submodules,
    build: 'cpu backend (NEON, no Metal), asr profile',
    flags: pinned.targets['macos-aarch64'].flags,
    toolset: { runner: 'local', deploymentTarget: '15.0' },
    cpu: pinned.targets['macos-aarch64'].cpu,
    files,
  };
  fs.writeFileSync(path.join(dir, 'runtime.json'), `${JSON.stringify(doc, null, 2)}\n`);
  return doc;
}

function pack(runtimeDir, out) {
  return node([PACK, '--runtime', runtimeDir, '--out', out, '--repository', 'MajesteitBart/rukoo']);
}

function unpack(zip, destination) {
  fs.mkdirSync(destination, { recursive: true });
  const result = run('ditto', ['-x', '-k', zip, destination]);
  assert.equal(result.status, 0, result.stderr);
}

// A fixture runtime packed to a zip, for the tests that start from pack-macos.mjs's output.
function packed(t, name) {
  const work = folder(t, name);
  const built = path.join(work, 'built');
  fs.mkdirSync(built);
  runtime(built);
  const out = path.join(work, 'release');
  assert.equal(pack(built, out).status, 0);
  const zip = path.join(out, `${pins().name}-macos-arm64.zip`);
  return { work, built, out, zip };
}

test('pack-macos pins the signed dylibs and leaves the Windows entry alone', { skip: SKIP }, (t) => {
  const work = folder(t, 'nemo-pack');
  const runtimeDir = path.join(work, 'built');
  fs.mkdirSync(runtimeDir);
  const built = runtime(runtimeDir);
  const out = path.join(work, 'release');
  const result = pack(runtimeDir, out);
  assert.equal(result.status, 0, result.stderr + result.stdout);

  const zipName = `${pins().name}-macos-arm64.zip`;
  const zip = path.join(out, zipName);
  const manifest = JSON.parse(fs.readFileSync(path.join(out, 'manifest.json'), 'utf8'));
  const committed = pins();

  assert.equal(manifest.format, 2);
  assert.equal(manifest.name, committed.name);
  assert.deepEqual(manifest.targets['windows-x86_64'], committed.targets['windows-x86_64']);
  assert.equal(
    JSON.stringify(manifest.targets['windows-x86_64'], null, 2),
    JSON.stringify(committed.targets['windows-x86_64'], null, 2),
  );
  assert.deepEqual(
    Object.keys(manifest).filter((key) => key !== 'targets'),
    Object.keys(committed).filter((key) => key !== 'targets'),
  );

  const entry = manifest.targets['macos-aarch64'];
  assert.equal(entry.archive.url, `https://github.com/MajesteitBart/rukoo/releases/download/${committed.name}/${zipName}`);
  assert.equal(entry.archive.size, fs.statSync(zip).size);
  assert.equal(entry.archive.sha256, sha256(zip));
  assert.equal(entry.archive.url.endsWith(`/${zipName}`), true);

  // The manifest and runtime.json describe the files in the zip, and every one has the size and hash they claim.
  const unpacked = path.join(work, 'unpacked');
  unpack(zip, unpacked);
  const names = listFiles(unpacked);
  assert.deepEqual(names, [...DYLIBS, ...LICENSES, 'runtime.json'].sort());
  const recorded = JSON.parse(fs.readFileSync(path.join(unpacked, 'runtime.json'), 'utf8'));
  assert.equal(recorded.target, 'macos-aarch64');
  assert.equal(recorded.name, committed.name);
  assert.equal(recorded.commit, committed.commit);
  assert.equal(recorded.files.length, entry.files.length);
  assert.deepEqual(entry.files.map((file) => file.path), recorded.files.map((file) => file.path));
  for (const file of entry.files) {
    const full = path.join(unpacked, file.path);
    assert.equal(fs.statSync(full).size, file.size, file.path);
    assert.equal(sha256(full), file.sha256, file.path);
  }

  // Signing changed the six dylibs: their hashes differ from the built ones, and each verifies ad-hoc and hardened.
  const builtHashes = new Map(built.files.map((file) => [file.path, file.sha256]));
  for (const name of DYLIBS) {
    const file = path.join(unpacked, name);
    assert.notEqual(sha256(file), builtHashes.get(name), `${name} was not signed`);
    const verify = run('codesign', ['--verify', '--strict', file]);
    assert.equal(verify.status, 0, `${name}: ${verify.stderr}`);
    const info = run('codesign', ['-d', '-vv', file]);
    assert.match(`${info.stdout}${info.stderr}`, /^Signature=adhoc$/m, name);
    assert.match(`${info.stdout}${info.stderr}`, /flags=0x[0-9a-f]+\([^)]*\bruntime\b/m, name);
  }
  // The licences are copied through untouched.
  for (const name of LICENSES) assert.equal(sha256(path.join(unpacked, name)), builtHashes.get(name), name);

  // The folder build-macos.sh wrote keeps the record of what it built.
  const unchanged = JSON.parse(fs.readFileSync(path.join(work, 'built', 'runtime.json'), 'utf8'));
  assert.deepEqual(unchanged.files, built.files);
  for (const name of DYLIBS) assert.equal(sha256(path.join(work, 'built', name)), builtHashes.get(name), name);

  // The notes name the archive and its signature.
  const notes = fs.readFileSync(path.join(out, 'notes.md'), 'utf8');
  assert.ok(notes.includes(zipName), notes);
  assert.ok(notes.includes(entry.archive.sha256), notes);
  assert.match(notes, /ad-hoc signed with hardened runtime/);

  // The same input packs to the same zip and manifest, so a rerun before publishing changes nothing.
  const again = path.join(work, 'release-again');
  const second = pack(runtimeDir, again);
  assert.equal(second.status, 0, second.stderr + second.stdout);
  assert.equal(sha256(path.join(again, zipName)), entry.archive.sha256);
  assert.equal(fs.readFileSync(path.join(again, 'manifest.json'), 'utf8'), fs.readFileSync(path.join(out, 'manifest.json'), 'utf8'));
});

test('pack-macos refuses a Windows folder', { skip: SKIP }, (t) => {
  const work = folder(t, 'nemo-pack-win');
  const dir = path.join(work, 'built');
  fs.mkdirSync(dir);
  runtime(dir, { target: 'windows-x86_64' });
  const result = pack(dir, path.join(work, 'release'));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /windows-x86_64/);
  assert.match(result.stderr, /pack\.ps1/);
});

test('pack-macos refuses a folder that runtime.json does not describe', { skip: SKIP }, (t) => {
  const work = folder(t, 'nemo-pack-bad');
  const dir = path.join(work, 'built');
  fs.mkdirSync(dir);
  runtime(dir);
  fs.appendFileSync(path.join(dir, 'libggml.dylib'), 'extra');
  const changed = pack(dir, path.join(work, 'release'));
  assert.notEqual(changed.status, 0);
  assert.match(changed.stderr, /libggml\.dylib differs from runtime\.json/);

  const extraDir = path.join(work, 'extra');
  fs.mkdirSync(extraDir);
  runtime(extraDir);
  fs.writeFileSync(path.join(extraDir, 'notes.txt'), 'not part of a runtime\n');
  const extra = pack(extraDir, path.join(work, 'release-extra'));
  assert.notEqual(extra.status, 0);
  assert.match(extra.stderr, /also holds notes\.txt/);
});

// A manifest whose macos-aarch64 entry is the packed one, with the archive at `url`.
function testManifest(dir, packedManifest, zip, overrides = {}) {
  const manifest = JSON.parse(fs.readFileSync(packedManifest, 'utf8'));
  const entry = manifest.targets['macos-aarch64'];
  entry.archive = {
    ...entry.archive,
    url: pathToFileURL(zip).href,
    ...overrides.archive,
  };
  if (overrides.files) entry.files = overrides.files(entry.files);
  const file = path.join(dir, 'manifest.json');
  fs.writeFileSync(file, `${JSON.stringify(manifest, null, 2)}\n`);
  return file;
}

function runtimeOnly(manifest, targetDir) {
  return node([PACKAGE, '--runtime-only', '--runtime-manifest', manifest], {
    env: { ...process.env, CARGO_TARGET_DIR: targetDir },
  });
}

// The same call without blocking this process, so a test can serve the archive from an HTTP server in it.
function runtimeOnlyAsync(manifest, targetDir) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [PACKAGE, '--runtime-only', '--runtime-manifest', manifest], {
      env: { ...process.env, CARGO_TARGET_DIR: targetDir },
    });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (chunk) => { stdout += chunk; });
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    child.on('close', (status) => resolve({ status, stdout, stderr }));
  });
}

test('package-macos downloads and checks the pinned runtime archive', { skip: SKIP }, (t) => {
  const work = folder(t, 'nemo-download');
  const built = path.join(work, 'built');
  fs.mkdirSync(built);
  runtime(built);
  const out = path.join(work, 'release');
  assert.equal(pack(built, out).status, 0);
  const zip = path.join(out, `${pins().name}-macos-arm64.zip`);
  const manifest = testManifest(work, path.join(out, 'manifest.json'), zip);

  const target = path.join(work, 'target');
  const result = runtimeOnly(manifest, target);
  assert.equal(result.status, 0, result.stderr + result.stdout);
  assert.match(result.stdout, /files and runtime\.json checked against/);
  assert.match(result.stdout, new RegExp(sha256(zip)));

  const unpacked = path.join(target, 'nemo-runtime', pins().name, 'nemo_speech');
  const entry = JSON.parse(fs.readFileSync(manifest, 'utf8')).targets['macos-aarch64'];
  assert.ok(fs.existsSync(path.join(unpacked, 'runtime.json')));
  for (const file of entry.files) {
    const full = path.join(unpacked, file.path);
    assert.equal(fs.statSync(full).size, file.size, file.path);
    assert.equal(sha256(full), file.sha256, file.path);
  }
  // The app carries these bytes: the dylibs are the packed, signed ones.
  const verify = run('codesign', ['--verify', '--strict', path.join(unpacked, 'libggml.dylib')]);
  assert.equal(verify.status, 0, verify.stderr);
});

test('package-macos downloads the archive over HTTP too', { skip: SKIP }, async (t) => {
  const work = folder(t, 'nemo-http');
  const built = path.join(work, 'built');
  fs.mkdirSync(built);
  runtime(built);
  const out = path.join(work, 'release');
  assert.equal(pack(built, out).status, 0);
  const zip = path.join(out, `${pins().name}-macos-arm64.zip`);
  const server = http.createServer((request, response) => {
    const body = fs.readFileSync(zip);
    response.writeHead(200, { 'content-type': 'application/zip', 'content-length': body.length });
    response.end(body);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const manifest = testManifest(work, path.join(out, 'manifest.json'), zip);
  const parsed = JSON.parse(fs.readFileSync(manifest, 'utf8'));
  parsed.targets['macos-aarch64'].archive.url =
    `http://127.0.0.1:${server.address().port}/${path.basename(zip)}`;
  fs.writeFileSync(manifest, `${JSON.stringify(parsed, null, 2)}\n`);

  const result = await runtimeOnlyAsync(manifest, path.join(work, 'target'));
  assert.equal(result.status, 0, result.stderr + result.stdout);
  assert.match(result.stdout, new RegExp(sha256(zip)));
  const unpacked = path.join(work, 'target', 'nemo-runtime', pins().name, 'nemo_speech');
  assert.ok(fs.existsSync(path.join(unpacked, 'runtime.json')));
});

test('package-macos refuses an archive with another hash', { skip: SKIP }, (t) => {
  const work = folder(t, 'nemo-bad-hash');
  const built = path.join(work, 'built');
  fs.mkdirSync(built);
  runtime(built);
  const out = path.join(work, 'release');
  assert.equal(pack(built, out).status, 0);
  const zip = path.join(out, `${pins().name}-macos-arm64.zip`);
  const manifest = testManifest(work, path.join(out, 'manifest.json'), zip, {
    archive: { sha256: '0'.repeat(64) },
  });
  const result = runtimeOnly(manifest, path.join(work, 'target'));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /has \d+ bytes and SHA-256 [0-9a-f]+ instead of \d+ and 0{64}/);
});

test('package-macos refuses a folder that differs from the pinned files', { skip: SKIP }, (t) => {
  const work = folder(t, 'nemo-bad-file');
  const built = path.join(work, 'built');
  fs.mkdirSync(built);
  runtime(built);
  const out = path.join(work, 'release');
  assert.equal(pack(built, out).status, 0);
  const zip = path.join(out, `${pins().name}-macos-arm64.zip`);
  const manifest = testManifest(work, path.join(out, 'manifest.json'), zip, {
    files: (files) => files.map((file, index) => (index === 0 ? { ...file, sha256: '0'.repeat(64) } : file)),
  });
  const result = runtimeOnly(manifest, path.join(work, 'target'));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /doesn't match .*manifest\.json: libggml-base\.dylib differs from/);
});

test('package-macos still stops when the manifest names no macOS archive', { skip: SKIP }, (t) => {
  const work = folder(t, 'nemo-unpublished');
  const manifest = path.join(work, 'manifest.json');
  const committed = pins();
  committed.targets['macos-aarch64'].archive = null;
  committed.targets['macos-aarch64'].files = [];
  fs.writeFileSync(manifest, `${JSON.stringify(committed, null, 2)}\n`);
  const result = runtimeOnly(manifest, path.join(work, 'target'));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /names no macos-aarch64 archive/);
  assert.match(result.stderr, /--nemo-dir/);
});

test('a packed dylib carries the shared signature, and signing it again keeps its hash', { skip: SKIP }, async (t) => {
  const { work, built, zip } = packed(t, 'nemo-sign');
  const unpacked = path.join(work, 'unpacked');
  unpack(zip, unpacked);
  const { codesign, isAdHocHardened } = await import(SIGN_LIB);
  const packedDylib = path.join(unpacked, 'libggml.dylib');

  // The linker's signature is ad-hoc, but without the hardened runtime flag, so it isn't the packed signature.
  assert.equal(isAdHocHardened(path.join(built, 'libggml.dylib')), false);
  assert.equal(isAdHocHardened(packedDylib), true);

  // Nor is a plain ad-hoc signature without the hardened runtime.
  const plain = path.join(work, 'plain');
  fs.mkdirSync(plain);
  const plainDylib = path.join(plain, 'libggml.dylib');
  fs.copyFileSync(path.join(built, 'libggml.dylib'), plainDylib);
  const bare = run('codesign', ['--force', '--sign', '-', plainDylib]);
  assert.equal(bare.status, 0, bare.stderr);
  assert.equal(isAdHocHardened(plainDylib), false);

  // package-macos.mjs treats this state as already signed; signing it with the shared rule writes the same bytes,
  // so its hash stays the one the manifest pins.
  const before = sha256(packedDylib);
  codesign(packedDylib);
  assert.equal(sha256(packedDylib), before);
  assert.equal(isAdHocHardened(packedDylib), true);
});

test('the runtime in the app has to keep matching the manifest after signing', { skip: SKIP }, async (t) => {
  const { work, out, zip } = packed(t, 'nemo-post-sign');
  const { pinnedFolderProblem, readRuntimeManifest } = await import(LIB);
  const manifest = readRuntimeManifest(path.join(out, 'manifest.json'));

  const app = path.join(work, 'Rukoo Voice.app', 'Contents', 'Resources', 'nemo_speech');
  unpack(zip, app);
  assert.equal(pinnedFolderProblem(manifest, app), null);

  // A dylib signed again, or any other file changed after signing, is what the check after signAll must catch.
  fs.appendFileSync(path.join(app, 'libggml.dylib'), 'signed again');
  assert.match(pinnedFolderProblem(manifest, app), /libggml\.dylib differs from .*manifest\.json/);

  const license = path.join(work, 'license-copy');
  unpack(zip, license);
  fs.appendFileSync(path.join(license, 'licenses', 'NOTICE'), 'changed');
  assert.match(pinnedFolderProblem(manifest, license), /licenses\/NOTICE differs from/);

  const extra = path.join(work, 'extra-copy');
  unpack(zip, extra);
  fs.writeFileSync(path.join(extra, 'not-part-of-the-runtime.txt'), 'x');
  assert.match(pinnedFolderProblem(manifest, extra), /also holds not-part-of-the-runtime\.txt/);
});

test('package-macos refuses --developer-id with a pinned runtime archive', { skip: SKIP }, (t) => {
  const { work, out, zip } = packed(t, 'nemo-developer-id');
  const manifest = testManifest(work, path.join(out, 'manifest.json'), zip);
  const env = { ...process.env, CARGO_TARGET_DIR: path.join(work, 'target') };
  // The refusal comes before the Apple credentials are read, so this must not need them.
  for (const name of ['APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID']) delete env[name];
  const result = node([PACKAGE, '--developer-id', '--runtime-manifest', manifest], { env });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /--developer-id signs the NeMo dylibs/);
  assert.match(result.stderr, /manifest that pins those hashes/);
  assert.doesNotMatch(result.stderr, /needs APPLE_SIGNING_IDENTITY/);
});

test('only https is accepted without --runtime-manifest', { skip: SKIP }, async () => {
  const { archiveUrlProblem } = await import(LIB);
  assert.equal(archiveUrlProblem(new URL('https://github.com/x/y.zip')), null);
  assert.match(archiveUrlProblem(new URL('http://github.com/x/y.zip')), /committed manifest archive has to be https:/);
  assert.match(archiveUrlProblem(new URL('file:///tmp/y.zip')), /committed manifest archive has to be https:/);
  assert.match(archiveUrlProblem(new URL('http://127.0.0.1:8080/y.zip')), /committed manifest archive has to be https:/);

  // A --runtime-manifest may point at this computer, and nowhere else without https.
  assert.equal(archiveUrlProblem(new URL('file:///tmp/y.zip'), { local: true }), null);
  assert.equal(archiveUrlProblem(new URL('http://127.0.0.1:8080/y.zip'), { local: true }), null);
  assert.match(archiveUrlProblem(new URL('http://example.invalid/y.zip'), { local: true }), /127\.0\.0\.1/);
  assert.match(
    archiveUrlProblem(new URL('http://example.invalid/y.zip'), { local: true, final: true }),
    /redirects to http:\/\/example\.invalid\/y\.zip/,
  );
});
