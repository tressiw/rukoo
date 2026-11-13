// The Rust crate list of voice/THIRD_PARTY_NOTICES.md, for both shipping targets, and the license texts a package ships.
// package.ps1 and package-macos.mjs both call it, so a dependency change can't ship stale notices on either platform.
//
//   node voice/scripts/notices.mjs --check
//   node voice/scripts/notices.mjs --update
//   node voice/scripts/notices.mjs --licenses <file> --target <triple> --nemo-dir <folder> [--nemo-label <text>]
//
// --check exits 1 when the generated part of THIRD_PARTY_NOTICES.md differs from what the lockfile gives, and
// --update rewrites it. The list is the union of the normal dependencies of the five programs on
// x86_64-pc-windows-msvc and aarch64-apple-darwin, without procedural macros (they run only while compiling) and
// without Voice's own crates. Each crate is listed once per version, and a crate only one target uses says so.
// cargo resolves both targets from the lockfile on either operating system, so a Mac can regenerate the Windows part
// and the reverse. It downloads the crates of the other target into the cargo registry when they aren't there.
//
// --licenses writes THIRD_PARTY_LICENSES.txt for one target: Inter's license, the license files of the NeMo runtime
// folder, then the license files each crate of that target carries in its source package.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const VOICE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const NOTICES = path.join(VOICE, 'THIRD_PARTY_NOTICES.md');
const INTER_LICENSE = path.join(VOICE, 'ui', 'fonts', 'Inter-LICENSE.txt');

export const WINDOWS = 'x86_64-pc-windows-msvc';
export const MACOS = 'aarch64-apple-darwin';
export const TARGETS = [WINDOWS, MACOS];
const TARGET_NAMES = { [WINDOWS]: 'Windows', [MACOS]: 'macOS' };
const PROGRAM_CRATES = ['voice-app', 'voice-recorder', 'voice-local', 'voice-speakers', 'voice-screen'];

const START = /<!-- rust-crates:start[^\n]*-->\n/;
const END = '<!-- rust-crates:end -->';

function cargo(args) {
  const result = spawnSync('cargo', args, { cwd: VOICE, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
  if (result.error) {
    throw new Error(`couldn't run cargo: ${result.error.message}. Rust is in ~/.cargo/bin, which is not always on PATH.`);
  }
  if (result.status !== 0) {
    throw new Error(`cargo ${args.join(' ')} failed with exit code ${result.status}: ${(result.stderr || '').trim()}`);
  }
  return result.stdout;
}

const compare = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

// One spelling per license: Cargo accepts "MIT/Apache-2.0" and either order of a plain OR.
export function formatLicense(expression) {
  const e = expression.trim();
  if (e === '') return 'No license field (see the crate)';
  if (/\bAND\b|\(/.test(e)) return e;
  const parts = [...new Set(e.split(/\s*\/\s*|\s+OR\s+/).filter((part) => part !== ''))];
  return parts.sort(compare).join(' OR ');
}

// The crates linked into the five programs on one target, as { 'name version': license }.
export function shippedCrates(target) {
  const out = cargo([
    'tree', '--locked', '-e', 'normal,no-proc-macro', '--target', target,
    ...PROGRAM_CRATES.flatMap((name) => ['-p', name]),
    '--prefix', 'none', '-f', '{p}|{l}',
  ]);
  const crates = new Map();
  for (const raw of out.split('\n')) {
    const line = raw.replace(/ \(\*\)$/, '').trim();
    const match = /^(\S+) v(\S+)(?: \((.+)\))?\|(.*)$/.exec(line);
    if (!match) continue;
    const [, name, version, source, license] = match;
    // Voice's own crates have a path instead of a registry source.
    if (source && (source.startsWith('/') || /^[A-Za-z]:[\\/]/.test(source))) continue;
    if (source) throw new Error(`${name} ${version} comes from ${source}, not from the registry; the notices don't cover that`);
    crates.set(`${name} ${version}`, formatLicense(license));
  }
  if (crates.size === 0) throw new Error(`cargo tree listed no crates for ${target}`);
  return crates;
}

// The union of both targets: [{ key, license, targets: [triple, ...] }], sorted by key.
export function allCrates() {
  const merged = new Map();
  for (const target of TARGETS) {
    for (const [key, license] of shippedCrates(target)) {
      const known = merged.get(key);
      if (known && known.license !== license) {
        throw new Error(`${key} has license ${known.license} on one target and ${license} on another`);
      }
      if (known) known.targets.push(target);
      else merged.set(key, { key, license, targets: [target] });
    }
  }
  return [...merged.values()].sort((a, b) => compare(a.key, b.key));
}

export function crateBlock(crates) {
  const only = (target) => crates.filter((c) => c.targets.length === 1 && c.targets[0] === target).length;
  const both = crates.filter((c) => c.targets.length === TARGETS.length).length;
  let text = `\n${crates.length} crates: ${both} in both builds, ${only(WINDOWS)} only in the Windows build and ${only(MACOS)} only in the macOS build.\n`;
  for (const license of [...new Set(crates.map((c) => c.license))].sort(compare)) {
    text += `\n### ${license}\n\n`;
    for (const crate of crates.filter((c) => c.license === license)) {
      const note = crate.targets.length === TARGETS.length ? '' : ` (${TARGET_NAMES[crate.targets[0]]} only)`;
      text += `- ${crate.key}${note}\n`;
    }
  }
  return `${text}\n`;
}

// The file with its generated part replaced, and whether that changed anything.
export function syncedNotices(crates) {
  const raw = fs.readFileSync(NOTICES, 'utf8');
  const eol = raw.includes('\r\n') ? '\r\n' : '\n';
  const text = raw.replaceAll('\r\n', '\n');
  const start = START.exec(text);
  const end = start ? text.indexOf(END, start.index + start[0].length) : -1;
  if (!start || end < 0) throw new Error(`${NOTICES} has no rust-crates markers`);
  const updated = text.slice(0, start.index + start[0].length) + crateBlock(crates) + text.slice(end);
  return { text: updated.replaceAll('\n', eol), current: updated === text };
}

export function updateNotices() {
  const crates = allCrates();
  const { text, current } = syncedNotices(crates);
  if (!current) fs.writeFileSync(NOTICES, text);
  return { crates, changed: !current };
}

// Throws when THIRD_PARTY_NOTICES.md lists other crates than the lockfile gives.
export function checkNotices() {
  const crates = allCrates();
  if (!syncedNotices(crates).current) {
    throw new Error(
      `The crate list in ${NOTICES} is out of date. Run node voice/scripts/notices.mjs --update (or `
      + 'voice/scripts/package.ps1 -UpdateNotices) and commit the change.',
    );
  }
  return crates;
}

const LICENSE_NAME = /^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)/;

function readText(file) {
  return fs.readFileSync(file, 'utf8').replaceAll('\r\n', '\n').trim();
}

function walk(dir) {
  const files = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...walk(full));
    else if (entry.isFile()) files.push(full);
  }
  return files;
}

// THIRD_PARTY_LICENSES.txt for one target: Inter's and the NeMo runtime's licenses, then each crate's license files.
export function licenseTexts(target, { nemoDir, nemoLabel }) {
  const crates = shippedCrates(target);
  const packages = new Map();
  const metadata = JSON.parse(cargo(['metadata', '--locked', '--format-version', '1', '--filter-platform', target]));
  for (const pkg of metadata.packages) packages.set(`${pkg.name} ${pkg.version}`, pkg);
  let out = 'License files of the third-party code in Rukoo Voice. THIRD_PARTY_NOTICES.md summarizes them.\n';
  out += `\n==== Inter (voice/ui/fonts) ====\n\n${readText(INTER_LICENSE)}\n`;
  if (nemoDir) {
    const licenseDir = path.join(nemoDir, 'licenses');
    out += `\n==== NeMo-Speech.cpp and the libraries in its runtime (${nemoLabel}) ====\n`;
    const files = walk(licenseDir).map((file) => ({ file, name: path.relative(licenseDir, file).split(path.sep).join('/') }));
    for (const { file, name } of files.sort((a, b) => compare(a.name, b.name))) {
      out += `\n-- ${name}\n\n${readText(file)}\n`;
    }
  }
  for (const key of [...crates.keys()].sort(compare)) {
    const pkg = packages.get(key);
    if (!pkg) throw new Error(`cargo metadata has no package ${key}`);
    const dir = path.dirname(pkg.manifest_path);
    if (!fs.existsSync(dir)) throw new Error(`the source of ${key} isn't in ${dir}; run cargo fetch --locked`);
    const names = new Set(fs.readdirSync(dir, { withFileTypes: true })
      .filter((entry) => entry.isFile() && LICENSE_NAME.test(entry.name))
      .map((entry) => entry.name));
    if (pkg.license_file && fs.existsSync(path.join(dir, pkg.license_file))) names.add(pkg.license_file);
    out += `\n==== ${key} (${crates.get(key)}) ====\n`;
    if (names.size === 0) out += `\nThe package has no license file. Its license is ${crates.get(key)}.\n`;
    for (const name of [...names].sort(compare)) {
      out += `\n-- ${path.basename(name)}\n\n${readText(path.join(dir, name))}\n`;
    }
  }
  return out;
}

function main(argv) {
  const flags = new Map();
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (!arg.startsWith('--')) throw new Error(`unexpected argument ${arg}`);
    const eq = arg.indexOf('=');
    if (eq > 0) flags.set(arg.slice(2, eq), arg.slice(eq + 1));
    else if (argv[i + 1] && !argv[i + 1].startsWith('--')) flags.set(arg.slice(2), argv[++i]);
    else flags.set(arg.slice(2), true);
  }
  if (flags.has('update')) {
    const { crates, changed } = updateNotices();
    console.log(`${changed ? 'Updated' : 'Already current:'} the crate list in ${NOTICES} (${crates.length} crates).`);
  } else if (flags.has('check')) {
    const crates = checkNotices();
    console.log(`THIRD_PARTY_NOTICES.md is current (${crates.length} crates).`);
  } else if (flags.has('licenses')) {
    const target = flags.get('target');
    if (!TARGETS.includes(target)) throw new Error(`--target must be one of ${TARGETS.join(', ')}`);
    const nemoDir = flags.get('nemo-dir');
    const text = licenseTexts(target, {
      nemoDir: typeof nemoDir === 'string' ? path.resolve(nemoDir) : null,
      nemoLabel: flags.get('nemo-label') || 'nemo_speech/licenses',
    });
    fs.writeFileSync(path.resolve(flags.get('licenses')), text);
  } else {
    throw new Error('usage: node voice/scripts/notices.mjs --check | --update | --licenses <file> --target <triple> [--nemo-dir <folder>]');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`notices: ${error.message}`);
    process.exitCode = 1;
  }
}
