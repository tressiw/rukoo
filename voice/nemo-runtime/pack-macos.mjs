#!/usr/bin/env node
// Packs a built macOS NeMo runtime folder into the release zip and writes the manifest that pins it. The counterpart
// of pack.ps1, which packs the Windows runtime.
//
//   node voice/nemo-runtime/pack-macos.mjs --runtime tmp/nemo/nemo_speech --out tmp/nemo/release
//
// It checks the folder build-macos.sh wrote against its runtime.json (every file present with its size and SHA-256,
// nothing else), refuses a folder whose runtime.json names another target or another build, signs the six dylibs the
// way voice/scripts/package-macos.mjs signs them, and writes three files to --out:
//
//   <name>-macos-arm64.zip   the runtime folder's files at the top of the archive
//   manifest.json            the committed per-target manifest with the macos-aarch64 entry filled in: the archive's
//                            download URL, size and SHA-256 and every file's size and hash. The windows-x86_64 entry
//                            stays as committed.
//   notes.md                 the release notes: the source, flags, toolset, signature, licenses and every hash
//
// After the zip is published as the release asset, that manifest.json replaces voice/nemo-runtime/manifest.json
// (README.md, "Publishing"). Nothing here publishes anything.
//
// Signing, and why runtime.json is rewritten: a signature is part of a Mach-O file, so signing a dylib changes its
// bytes and its hash. build-macos.sh records the hashes of the files it built, which are only linker-signed; the zip
// holds the signed files. So the script signs copies of the folder, rewrites the copied runtime.json to name the signed
// files, and leaves --runtime as the build wrote it. package-macos.mjs and speaker-assets.mjs check a runtime folder
// against its runtime.json, and voice-core checks the app's copy against the manifest, so the two records agree on the
// signed files.
//
// The signature is ad-hoc with hardened runtime, no identifier override and no entitlements, exactly what
// package-macos.mjs gives a NeMo dylib (voice/scripts/lib/macos-sign.mjs). package-macos.mjs leaves a dylib in that
// state alone, so the files in Rukoo Voice.app keep the hashes pinned here.
//
// The zip is reproducible: entries are sorted, every entry is dated 1980-01-01 00:00:00 (the ZIP epoch), the dylibs
// have Unix mode 0755 and the rest 0644, there are no extra fields, and the data is deflated. The same input and the
// same Node version give the same zip; another zlib version could compress differently, and the published hashes pin
// the archive that was built.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { NEMO_DYLIBS, codesign } from '../scripts/lib/macos-sign.mjs';
import { listFiles, sha256File } from '../scripts/lib/nemo-runtime.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const MANIFEST = path.join(HERE, 'manifest.json');
const TARGET = 'macos-aarch64';
const USAGE = `Usage: node voice/nemo-runtime/pack-macos.mjs --runtime <folder> --out <folder> [--repository <owner/repo>]

Checks a built macOS NeMo runtime folder against its runtime.json, signs the six dylibs, and writes
<name>-macos-arm64.zip, manifest.json and notes.md to --out. It publishes nothing.

Options:
  --runtime <folder>       the runtime folder build-macos.sh wrote, with its runtime.json
  --out <folder>           where the zip, manifest.json and notes.md go
  --repository <owner/repo>  the GitHub repository the release belongs to
                           (default MajesteitBart/rukoo)
  -h, --help               this text`;

function parseArgs(argv) {
  const out = { runtime: null, out: null, repository: 'MajesteitBart/rukoo', help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const eq = arg.indexOf('=');
    const key = eq > 0 ? arg.slice(0, eq) : arg;
    const value = eq > 0 ? arg.slice(eq + 1) : null;
    const take = () => {
      const taken = value ?? argv[++i];
      if (!taken) throw new Error(`${key} needs a value`);
      return taken;
    };
    switch (key) {
      case '-h':
      case '--help':
        out.help = true;
        break;
      case '--runtime':
        out.runtime = path.resolve(take());
        break;
      case '--out':
        out.out = path.resolve(take());
        break;
      case '--repository':
        out.repository = take();
        break;
      default:
        throw new Error(`unknown option ${arg}`);
    }
  }
  if (!out.help && !out.runtime) throw new Error('--runtime is required');
  if (!out.help && !out.out) throw new Error('--out is required');
  return out;
}

let ARGS;
try {
  ARGS = parseArgs(process.argv.slice(2));
} catch (error) {
  console.error(`pack-macos: ${error.message}\n\n${USAGE}`);
  process.exit(2);
}

function record(full, name) {
  return { path: name, size: fs.statSync(full).size, sha256: sha256File(full) };
}

// Checks the built folder against its runtime.json the way pack.ps1 checks a Windows folder: the target, the build's
// name and commit, every recorded file at its size and hash, and nothing besides runtime.json and those files.
function checkBuilt(runtimeDir, pins) {
  const runtimePath = path.join(runtimeDir, 'runtime.json');
  if (!fs.existsSync(runtimePath)) {
    throw new Error(`${runtimeDir} has no runtime.json; build it with build-macos.sh`);
  }
  let built;
  try {
    built = JSON.parse(fs.readFileSync(runtimePath, 'utf8'));
  } catch (error) {
    throw new Error(`${runtimePath} isn't JSON: ${error.message}`);
  }
  const target = built.target || TARGET;
  if (target !== TARGET) {
    throw new Error(
      `${runtimeDir} holds the ${target} runtime. This script packs the macOS one; use pack.ps1 for a Windows folder.`,
    );
  }
  if (built.name !== pins.name || built.commit !== pins.commit) {
    throw new Error(`${runtimeDir} holds ${built.name} at ${built.commit}, not ${pins.name} at ${pins.commit}`);
  }
  if (!Array.isArray(built.files) || built.files.length === 0) {
    throw new Error(`${runtimePath} lists no files`);
  }
  const recorded = new Set(['runtime.json']);
  for (const file of built.files) {
    const full = path.join(runtimeDir, file.path);
    if (!fs.existsSync(full) || !fs.statSync(full).isFile()) {
      throw new Error(`${file.path} is missing from ${runtimeDir}`);
    }
    if (fs.statSync(full).size !== file.size || sha256File(full) !== String(file.sha256).toLowerCase()) {
      throw new Error(`${file.path} differs from runtime.json`);
    }
    recorded.add(file.path);
  }
  const extra = listFiles(runtimeDir).filter((name) => !recorded.has(name));
  if (extra.length) throw new Error(`${runtimeDir} also holds ${extra.join(', ')}, which runtime.json doesn't list`);
  const missing = NEMO_DYLIBS.filter((name) => !recorded.has(name));
  if (missing.length) throw new Error(`${runtimePath} doesn't list ${missing.join(', ')}, the dylibs this script signs`);
  return built;
}

// ---------- the zip ----------

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// A zip with fixed timestamps, fixed modes and sorted entries, so the same folder packs to the same bytes. Written by
// hand because Node's standard library has no zip writer and the platform's tools stamp the files' own times.
function writeZip(dir, zipPath, names) {
  const local = [];
  const central = [];
  let offset = 0;
  const dosTime = 0;
  // 1980-01-01, the first date the format can hold.
  const dosDate = (0 << 9) | (1 << 5) | 1;
  for (const name of [...names].sort()) {
    const data = fs.readFileSync(path.join(dir, name));
    const packed = zlib.deflateRawSync(data, { level: 9 });
    const crc = crc32(data);
    const nameBytes = Buffer.from(name, 'utf8');
    // The dylibs keep the executable bit the build gave them; the licences and runtime.json are plain files.
    const mode = name.endsWith('.dylib') ? 0o100755 : 0o100644;

    const header = Buffer.alloc(30);
    header.writeUInt32LE(0x04034b50, 0);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(0x0800, 6); // names are UTF-8
    header.writeUInt16LE(8, 8); // deflate
    header.writeUInt16LE(dosTime, 10);
    header.writeUInt16LE(dosDate, 12);
    header.writeUInt32LE(crc, 14);
    header.writeUInt32LE(packed.length, 18);
    header.writeUInt32LE(data.length, 22);
    header.writeUInt16LE(nameBytes.length, 26);
    header.writeUInt16LE(0, 28);
    local.push(header, nameBytes, packed);

    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE((3 << 8) | 20, 4); // made by Unix, version 2.0
    entry.writeUInt16LE(20, 6);
    entry.writeUInt16LE(0x0800, 8);
    entry.writeUInt16LE(8, 10);
    entry.writeUInt16LE(dosTime, 12);
    entry.writeUInt16LE(dosDate, 14);
    entry.writeUInt32LE(crc, 16);
    entry.writeUInt32LE(packed.length, 20);
    entry.writeUInt32LE(data.length, 24);
    entry.writeUInt16LE(nameBytes.length, 28);
    entry.writeUInt16LE(0, 30); // extra field
    entry.writeUInt16LE(0, 32); // comment
    entry.writeUInt16LE(0, 34); // disk number
    entry.writeUInt16LE(0, 36); // internal attributes
    entry.writeUInt32LE((mode << 16) >>> 0, 38); // Unix mode
    entry.writeUInt32LE(offset, 42);
    central.push(entry, nameBytes);

    offset += header.length + nameBytes.length + packed.length;
  }
  const directory = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(names.length, 8);
  end.writeUInt16LE(names.length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);
  fs.writeFileSync(zipPath, Buffer.concat([...local, directory, end]));
  return { size: fs.statSync(zipPath).size, sha256: sha256File(zipPath) };
}

// ---------- the manifest and the notes ----------

// The committed manifest with this target's entry replaced by the built one plus its archive. The other targets are
// copied as they were parsed, so the Windows entry keeps every value it had.
function writeManifest(pins, built, files, archive, out) {
  const entry = {};
  for (const [key, value] of Object.entries(built)) {
    if (['format', 'target', 'name', 'repository', 'commit', 'submodules'].includes(key)) continue;
    if (key === 'files') entry.archive = archive;
    entry[key] = key === 'files' ? files : value;
  }
  const manifest = { ...pins, targets: { ...pins.targets, [TARGET]: entry } };
  fs.writeFileSync(path.join(out, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
}

function writeNotes(built, files, archive, zipName, out) {
  const short = built.commit.slice(0, 7);
  const flags = Object.entries(built.flags ?? {}).map(([name, value]) => `${name}=${value}`).join(', ');
  const toolset = built.toolset ?? {};
  const lines = [];
  lines.push(
    'The NeMo-Speech.cpp runtime that Rukoo Voice\'s speaker worker, `rukoo-voice-speakers`, loads from its '
    + '`Contents/Resources/nemo_speech/` folder to label speakers with NVIDIA Nemotron 3 Diarization. It is not a '
    + 'release of Rukoo or Rukoo Voice; Voice\'s macOS package carries these files, and '
    + '`voice/nemo-runtime/manifest.json` pins them.\n',
  );
  lines.push(
    `- Source: [NVIDIA/NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) at `
    + `[\`${short}\`](https://github.com/NVIDIA/NeMo-Speech.cpp/commit/${built.commit}), with ggml at `
    + `\`${built.submodules?.ggml}\` and llama.cpp at \`${built.submodules?.['llama.cpp']}\`.`,
  );
  lines.push(`- Build: ${built.build}. ggml flags: ${flags}.`);
  lines.push(
    `- Toolset: ${toolset.runner ?? 'local'}, ${toolset.xcode ?? 'Xcode unknown'}, `
    + `${toolset.clang ?? 'clang unknown'}, ${toolset.cmake ?? 'CMake unknown'}, deployment target macOS `
    + `${toolset.deploymentTarget ?? '15.0'}.`,
  );
  lines.push('- Runs on Apple Silicon, on macOS 15 or later.');
  lines.push(
    '- Signature: the six dylibs are ad-hoc signed with hardened runtime (`codesign --force --sign - --options '
    + 'runtime --timestamp=none`), exactly as `voice/scripts/package-macos.mjs` signs them. That script leaves a '
    + 'dylib in this state alone, so the hashes below are the ones in a finished `Rukoo Voice.app`.',
  );
  lines.push(
    '- Licenses: NeMo-Speech.cpp is Apache-2.0, Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. `licenses/LICENSE`, '
    + '`licenses/NOTICE` and `licenses/THIRD_PARTY_NOTICES.md` come with it and name what the dylibs contain, such as '
    + 'ggml (MIT). `licenses/sentencepiece/` holds the licenses of the libraries the static SentencePiece build '
    + 'compiles in: `LICENSE` (SentencePiece, Apache-2.0), `absl-LICENSE`, `darts-clone-LICENSE` and '
    + '`protobuf-lite-LICENSE`. `libggml-blas` and `libggml-cpu` import Accelerate, part of macOS; there is no vcpkg '
    + 'and no Visual C++ runtime to ship.\n',
  );
  lines.push('| File | Bytes | SHA-256 |');
  lines.push('|---|---:|---|');
  lines.push(`| \`${zipName}\` | ${archive.size} | \`${archive.sha256}\` |`);
  for (const file of files) lines.push(`| \`${file.path}\` | ${file.size} | \`${file.sha256}\` |`);
  fs.writeFileSync(path.join(out, 'notes.md'), `${lines.join('\n')}\n`);
}

// ---------- main ----------

function main() {
  if (ARGS.help) {
    console.log(USAGE);
    return;
  }
  const pins = JSON.parse(fs.readFileSync(MANIFEST, 'utf8'));
  if (pins.format !== 2) {
    throw new Error(`voice/nemo-runtime/manifest.json is format ${pins.format}, not the per-target format 2`);
  }
  const built = checkBuilt(ARGS.runtime, pins);

  // Sign copies, never the build's folder: this step changes dylib bytes, and build-macos.sh's runtime.json stays the
  // record of what it built.
  const staging = fs.mkdtempSync(path.join(os.tmpdir(), 'nemo-pack-macos-'));
  try {
    fs.cpSync(ARGS.runtime, staging, { recursive: true });
    for (const name of NEMO_DYLIBS) {
      codesign(path.join(staging, name));
      console.log(`Signed ${name} ad-hoc with hardened runtime`);
    }
    // The signed files, which are what the manifest and speaker labels must describe.
    const files = listFiles(staging)
      .filter((name) => name !== 'runtime.json')
      .map((name) => record(path.join(staging, name), name));
    const packedRuntime = { ...built, files };
    fs.writeFileSync(path.join(staging, 'runtime.json'), `${JSON.stringify(packedRuntime, null, 2)}\n`);

    // Everything the zip holds and the manifest names, in one list: nothing may be signed after it was hashed.
    const names = ['runtime.json', ...files.map((file) => file.path)];
    for (const file of files) {
      const full = path.join(staging, file.path);
      if (fs.statSync(full).size !== file.size || sha256File(full) !== file.sha256) {
        throw new Error(`${file.path} changed while it was packed`);
      }
    }

    fs.mkdirSync(ARGS.out, { recursive: true });
    const zipName = `${pins.name}-macos-arm64.zip`;
    const zipPath = path.join(ARGS.out, zipName);
    fs.rmSync(zipPath, { force: true });
    const zip = writeZip(staging, zipPath, names);
    const archive = {
      url: `https://github.com/${ARGS.repository}/releases/download/${pins.name}/${zipName}`,
      size: zip.size,
      sha256: zip.sha256,
    };
    writeManifest(pins, built, files, archive, ARGS.out);
    writeNotes(built, files, archive, zipName, ARGS.out);

    console.log(`\nPacked ${zipName} (${archive.size.toLocaleString('en')} bytes, SHA-256 ${archive.sha256})`);
    console.log(`Wrote manifest.json and notes.md in ${ARGS.out}`);
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}

try {
  main();
} catch (error) {
  console.error(`pack-macos: ${error.message}`);
  process.exitCode = 1;
}
