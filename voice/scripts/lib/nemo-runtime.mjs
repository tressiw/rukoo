// What voice/nemo-runtime/manifest.json pins and how the macOS packaging checks it, in one place: which folder holds
// exactly an entry's files, where the pins come from, and which archive URLs may be downloaded.
//
// voice/nemo-runtime/pack-macos.mjs rewrites a packed runtime.json for its signed files. voice/scripts/package-macos.mjs
// checks a downloaded folder against the manifest's pinned files and reads only the build's name and commit from
// runtime.json; voice-core checks the app's copy against the same pins (crates/voice-core/src/speakers/runtime.rs,
// verify_voice). Tested by test/voice-nemo-runtime.test.js without a package build.

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

export const NEMO_TARGET = 'macos-aarch64';

export function sha256File(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

// Every file under `dir`, relative with `/` between parts, sorted.
export function listFiles(dir) {
  const found = [];
  const walk = (next, prefix) => {
    for (const entry of fs.readdirSync(next, { withFileTypes: true })) {
      const full = path.join(next, entry.name);
      const name = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(full, name);
      else if (entry.isFile()) found.push(name);
      else throw new Error(`${full} is not a regular file or a folder`);
    }
  };
  walk(dir, '');
  return found.sort();
}

// The committed manifest, or --runtime-manifest, with this platform's entry. `pins` is the whole document, `entry`
// the macos-aarch64 object, and `path` what the messages name.
export function readRuntimeManifest(file) {
  let pins;
  try {
    pins = JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (error) {
    throw new Error(`${file} isn't a runtime manifest: ${error.message}`);
  }
  if (pins.format !== 2) {
    throw new Error(`${file} is format ${pins.format}, not the per-target format 2`);
  }
  const entry = pins.targets && pins.targets[NEMO_TARGET];
  if (!entry) throw new Error(`${file} pins no ${NEMO_TARGET} runtime`);
  return { path: file, pins, entry };
}

// Why a runtime folder doesn't match the manifest's entry, or null when it does: runtime.json's name and commit are
// the build's, every pinned file has its size and hash, and the folder holds nothing else. The pinned list, not
// runtime.json's own file list, decides the hashes: pack-macos.mjs rewrote that list for the signed files.
export function pinnedFolderProblem(manifest, dir) {
  const runtimePath = path.join(dir, 'runtime.json');
  if (!fs.existsSync(runtimePath)) return 'runtime.json is missing';
  let runtime;
  try {
    runtime = JSON.parse(fs.readFileSync(runtimePath, 'utf8'));
  } catch (error) {
    return `runtime.json is not JSON: ${error.message}`;
  }
  if (runtime.name !== manifest.pins.name || runtime.commit !== manifest.pins.commit) {
    return `runtime.json describes ${runtime.name} at ${runtime.commit}`;
  }
  const expected = new Set(['runtime.json']);
  for (const file of manifest.entry.files) {
    expected.add(file.path);
    const full = path.join(dir, file.path);
    if (!fs.existsSync(full) || !fs.statSync(full).isFile()) return `${file.path} is missing`;
    if (fs.statSync(full).size !== file.size || sha256File(full) !== `${file.sha256}`.toLowerCase()) {
      return `${file.path} differs from ${manifest.path}`;
    }
  }
  const extra = listFiles(dir).filter((name) => !expected.has(name));
  if (extra.length) return `it also holds ${extra.join(', ')}`;
  return null;
}

// Why an archive URL can't be downloaded, or null. The committed manifest may only name an https: URL: its archive is
// a GitHub release asset. A --runtime-manifest may also name a file: URL or a plain http://127.0.0.1 one, so a test
// package can pin a zip on this computer. A redirect is checked with the same rule, with `final`.
export function archiveUrlProblem(url, { local = false, final = false } = {}) {
  if (url.protocol === 'https:') return null;
  if (local && url.protocol === 'file:') return null;
  if (local && url.protocol === 'http:' && url.hostname === '127.0.0.1') return null;
  const allowed = local
    ? 'https:, or file: and http://127.0.0.1 for a --runtime-manifest test package'
    : 'https:';
  return `the runtime archive URL ${final ? 'redirects to' : 'is'} ${url.href}, but a `
    + `${local ? '--runtime-manifest' : 'committed manifest'} archive has to be ${allowed}`;
}
