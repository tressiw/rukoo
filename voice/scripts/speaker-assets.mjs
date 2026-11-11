#!/usr/bin/env node
// Fetches what the speaker label tests need on macOS and Windows: the NeMo runtime, the Nemotron
// model and the AMI meeting excerpt. The cross-platform counterpart of speaker-assets.ps1, which
// stays the Windows script; both read the same pins and print the same three variables.
//
// Downloads each file once into --out and checks its size and SHA-256 against a pin; a file
// already there with the right hash is kept. Nothing is read from or written to Ownkey's folders.
//
//   <out>/nemo_speech/                               --runtime: the published NeMo-Speech.cpp
//                                                    runtime of voice/nemo-runtime/manifest.json
//                                                    for this platform, every file checked
//   <out>/nemotron/Nemotron-3-Diarization.q8_0.gguf  --model: NVIDIA Nemotron 3 Diarization, the
//                                                    pin of voice-core's ModelSpec::nemotron()
//   <out>/ami/                                        --ami: the three files of
//                                                    voice/fixtures/ami/manifest.json
//
// Without a switch it does all three. Until a platform's runtime is published, its manifest entry
// has no archive: --runtime says so and skips it, and --nemo-dir takes a local build
// (voice/nemo-runtime/build-macos.sh) instead and checks its runtime.json.
//
// Usage:
//   node voice/scripts/speaker-assets.mjs --out tmp/speaker-assets
//   node voice/scripts/speaker-assets.mjs --out tmp/speaker-assets --nemo-dir tmp/nemo/nemo_speech
//   node voice/scripts/speaker-assets.mjs --out tmp/speaker-assets --model --ami
//
// Node's standard library only; a zip is unpacked with the platform's own tool (ditto on macOS,
// tar on Windows, unzip elsewhere) and every file is checked against the manifest afterwards.

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// The pin of ModelSpec::nemotron() in voice/crates/voice-core/src/local/registry.rs.
// voice-speakers' tests/assets.rs and voice-core's tests/models.rs check that this script and
// speaker-assets.ps1 carry the same URL, size and SHA-256.
const NEMOTRON_URL =
  'https://huggingface.co/nvidia/Nemotron-3-Diarization/resolve/f667ed73aee57d40cc39428eb768b4fd87a0a29e/Nemotron-3-Diarization.q8_0.gguf';
const NEMOTRON_SIZE = 107012128;
const NEMOTRON_SHA256 = '08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1';

const VOICE = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** The manifest target of the platform this runs on, as nemo-runtime/manifest.json names it. */
function targetKey() {
  if (process.platform === 'win32' && process.arch === 'x64') return 'windows-x86_64';
  if (process.platform === 'darwin' && process.arch === 'arm64') return 'macos-aarch64';
  return null;
}

function parse(argv) {
  const options = { out: null, nemoDir: null, runtime: false, model: false, ami: false };
  for (let i = 0; i < argv.length; i += 1) {
    const name = argv[i];
    const value = () => {
      i += 1;
      if (i >= argv.length) throw new Error(`${name} needs a value`);
      return argv[i];
    };
    if (name === '--out') options.out = value();
    else if (name === '--nemo-dir') options.nemoDir = value();
    else if (name === '--runtime') options.runtime = true;
    else if (name === '--model') options.model = true;
    else if (name === '--ami') options.ami = true;
    else if (name === '-h' || name === '--help') options.help = true;
    else throw new Error(`unknown argument ${name}`);
  }
  if (!options.help && !options.out) throw new Error('--out is required');
  if (!options.runtime && !options.model && !options.ami && !options.help) {
    options.runtime = true;
    options.model = true;
    options.ami = true;
  }
  return options;
}

function usage() {
  const text = readFileSync(fileURLToPath(import.meta.url), 'utf8');
  const start = text.indexOf('// Usage:');
  const end = text.indexOf('\n\n', start);
  return text
    .slice(start, end)
    .split('\n')
    .map((line) => line.replace(/^\/\/ ?/, ''))
    .join('\n');
}

function sha256File(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function sizeOf(path) {
  return statSync(path).size;
}

/** A file with the pinned size and SHA-256. */
function pinned(path, size, sha256) {
  return (
    existsSync(path) &&
    statSync(path).isFile() &&
    sizeOf(path) === size &&
    sha256File(path) === sha256.toLowerCase()
  );
}

/**
 * Downloads `url` to `path` unless a file with the pinned size and hash is already there. The
 * download lands next to `path` first and replaces it only once it checks out.
 */
async function fetchPinned(url, path, size, sha256) {
  const name = basename(path);
  if (pinned(path, size, sha256)) {
    console.log(`  ${name} is there`);
    return;
  }
  mkdirSync(dirname(path), { recursive: true });
  const part = `${path}.part`;
  let lastError = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    console.log(`  ${name}: downloading ${size.toLocaleString('en')} bytes`);
    try {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`HTTP ${response.status} ${response.statusText}`);
      writeFileSync(part, Buffer.from(await response.arrayBuffer()));
      lastError = null;
      break;
    } catch (error) {
      lastError = error;
      if (attempt < 3) await new Promise((done) => setTimeout(done, 5000 * attempt));
    }
  }
  if (lastError) throw new Error(`could not download ${url}: ${lastError.message}`);
  const got = sizeOf(part);
  if (got !== size) {
    rmSync(part, { force: true });
    throw new Error(`${url} has ${got} bytes instead of ${size}`);
  }
  const hash = sha256File(part);
  if (hash !== sha256.toLowerCase()) {
    rmSync(part, { force: true });
    throw new Error(`${url} has SHA-256 ${hash} instead of ${sha256}`);
  }
  rmSync(path, { force: true });
  renameSync(part, path);
}

/** Every file under `dir`, relative with `/` between parts. */
function listFiles(dir) {
  const found = [];
  const walk = (next, prefix) => {
    for (const entry of readdirSync(next, { withFileTypes: true })) {
      const path = join(next, entry.name);
      const name = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(path, name);
      else found.push(name);
    }
  };
  walk(dir, '');
  return found;
}

/**
 * Why a runtime folder doesn't match, or null when it does. Every file has its size and SHA-256,
 * runtime.json names the same build, and nothing else is in the folder. The manifest's files are
 * the pins once the target is published; before that the local build's runtime.json is.
 */
function runtimeProblem(manifest, target, dir) {
  const runtimePath = join(dir, 'runtime.json');
  if (!existsSync(runtimePath)) return 'runtime.json is missing';
  let runtime;
  try {
    runtime = JSON.parse(readFileSync(runtimePath, 'utf8'));
  } catch (error) {
    return `runtime.json is not JSON: ${error.message}`;
  }
  if (runtime.name !== manifest.name || runtime.commit !== manifest.commit) {
    return `runtime.json describes ${runtime.name}`;
  }
  if (runtime.target && runtime.target !== target.key) {
    return `runtime.json is a ${runtime.target} build`;
  }
  const files = target.pinned && target.pinned.length ? target.pinned : runtime.files;
  if (!Array.isArray(files) || files.length === 0) return 'runtime.json lists no files';
  const expected = new Set(['runtime.json']);
  for (const file of files) {
    expected.add(file.path);
    if (!pinned(join(dir, file.path), file.size, file.sha256)) {
      return `${file.path} is missing or differs`;
    }
  }
  const extra = listFiles(dir).filter((path) => !expected.has(path));
  if (extra.length) return `it also holds ${extra.join(', ')}`;
  return null;
}

/** Unpacks `zip` into `destination` with the platform's own tool. */
function unpack(zip, destination) {
  const tool =
    process.platform === 'darwin'
      ? ['ditto', ['-x', '-k', zip, destination]]
      : process.platform === 'win32'
        ? ['tar', ['-xf', zip, '-C', destination]]
        : ['unzip', ['-q', '-o', zip, '-d', destination]];
  const result = spawnSync(tool[0], tool[1], { stdio: 'inherit' });
  if (result.error) throw new Error(`could not run ${tool[0]}: ${result.error.message}`);
  if (result.status !== 0) throw new Error(`${tool[0]} exited with ${result.status}`);
}

async function main() {
  const options = parse(process.argv.slice(2));
  if (options.help) {
    console.log(usage());
    return;
  }
  const out = resolve(options.out);
  mkdirSync(out, { recursive: true });
  const variables = [];

  const key = targetKey();
  let manifest = null;
  let target = null;
  if (options.runtime || options.nemoDir) {
    if (!key) {
      throw new Error(
        `speaker labels have no NeMo runtime on ${process.platform}/${process.arch}; ` +
          'Windows on x86-64 and macOS on Apple Silicon only'
      );
    }
    manifest = JSON.parse(readFileSync(join(VOICE, 'nemo-runtime', 'manifest.json'), 'utf8'));
    if (manifest.format !== 2) {
      throw new Error(`voice/nemo-runtime/manifest.json is format ${manifest.format}, not 2`);
    }
    const entry = manifest.targets[key];
    if (!entry) {
      throw new Error(`voice/nemo-runtime/manifest.json pins no ${key} runtime`);
    }
    target = { key, entry, pinned: entry.files ?? [], archive: entry.archive ?? null };
  }

  if (options.nemoDir) {
    const dir = resolve(options.nemoDir);
    console.log(`Runtime in ${dir}`);
    const problem = runtimeProblem(manifest, target, dir);
    if (problem) throw new Error(`${dir} is not the runtime the manifest pins: ${problem}`);
    console.log(`  ${target.pinned.length || listFiles(dir).length - 1} files checked`);
    variables.push(['RUKOO_VOICE_TEST_NEMO_DIR', dir]);
  } else if (options.runtime) {
    console.log(`Runtime ${manifest.name} (${key})`);
    if (!target.archive) {
      console.log(
        '  not published yet: voice/nemo-runtime/manifest.json has no archive for this platform, ' +
          'so the runtime is skipped. Point --nemo-dir at a local build instead ' +
          '(voice/nemo-runtime/README.md).'
      );
    } else {
      const dir = join(out, 'nemo_speech');
      const zip = join(out, target.archive.url.slice(target.archive.url.lastIndexOf('/') + 1));
      if (existsSync(dir) && runtimeProblem(manifest, target, dir) === null) {
        console.log('  nemo_speech is there');
      } else {
        await fetchPinned(target.archive.url, zip, target.archive.size, target.archive.sha256);
        const staging = `${dir}.staging`;
        rmSync(staging, { recursive: true, force: true });
        unpack(zip, staging);
        const problem = runtimeProblem(manifest, target, staging);
        if (problem) {
          rmSync(staging, { recursive: true, force: true });
          throw new Error(`the runtime archive doesn't match voice/nemo-runtime/manifest.json: ${problem}`);
        }
        rmSync(dir, { recursive: true, force: true });
        renameSync(staging, dir);
        console.log('  nemo_speech checked');
      }
      variables.push(['RUKOO_VOICE_TEST_NEMO_DIR', dir]);
    }
  }

  if (options.model) {
    console.log('Nemotron 3 Diarization');
    const gguf = join(out, 'nemotron', 'Nemotron-3-Diarization.q8_0.gguf');
    await fetchPinned(NEMOTRON_URL, gguf, NEMOTRON_SIZE, NEMOTRON_SHA256);
    variables.push(['RUKOO_VOICE_TEST_NEMOTRON', gguf]);
  }

  if (options.ami) {
    const fixtures = JSON.parse(readFileSync(join(VOICE, 'fixtures', 'ami', 'manifest.json'), 'utf8'));
    console.log('AMI meeting excerpt');
    const dir = join(out, 'ami');
    for (const file of fixtures.files) {
      await fetchPinned(`${fixtures.source}${file.name}`, join(dir, file.name), file.size, file.sha256);
    }
    variables.push(['RUKOO_VOICE_TEST_AMI', dir]);
  }

  console.log('');
  for (const [name, value] of variables) console.log(`${name}=${value}`);
}

main().catch((error) => {
  console.error(`speaker-assets: ${error.message}`);
  process.exitCode = 1;
});
