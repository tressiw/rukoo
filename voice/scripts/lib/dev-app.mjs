// The parts of voice/scripts/dev-app-macos.mjs that build `open`'s arguments, find and stop processes and fingerprint
// the icon, apart from the script so test/voice-dev-app-macos.test.js can run them on processes it started itself.

import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// The arguments `open` gets for a development copy: `-n` starts another copy, and the data folder and the discovery
// file go in as `--env`, because an app started with `open` doesn't inherit the environment of the shell that ran the
// script. `dumpEmptyDictation` travels the same way when the script's environment sets it, and is left out entirely
// when it doesn't. App flags stay behind `--args`.
export function openArgs({ bundle, data, bridge, forward = [], dumpEmptyDictation }) {
  const args = ['-n', '--env', `RUKOO_VOICE_DATA_DIR=${data}`, '--env', `RUKOO_VOICE_BRIDGE=${bridge}`];
  if (dumpEmptyDictation !== undefined) {
    args.push('--env', `RUKOO_VOICE_DUMP_EMPTY_DICTATION=${dumpEmptyDictation}`);
  }
  args.push(bundle);
  if (forward.length) args.push('--args', ...forward);
  return args;
}

// Every process with its parent and executable. `ps` on macOS reports the full executable path in comm, so a match on
// it never matches a process name or another Voice.
export function processTable() {
  const out = spawnSync('ps', ['-ww', '-axo', 'pid=,ppid=,comm='], { encoding: 'utf8' });
  if (out.status !== 0) throw new Error(`ps failed: ${(out.stderr || '').trim() || `exit ${out.status}`}`);
  return out.stdout
    .split('\n')
    .map((line) => /^\s*(\d+)\s+(\d+)\s+(.*?)\s*$/.exec(line))
    .filter(Boolean)
    .map((match) => ({ pid: Number(match[1]), ppid: Number(match[2]), exe: path.resolve(match[3]) }))
    .filter((row) => row.pid !== process.pid);
}

// The pids running one of these executables.
export function runningFrom(table, exes) {
  const wanted = new Set(exes.map((exe) => path.resolve(exe)));
  return table.filter((row) => wanted.has(row.exe)).map((row) => row.pid);
}

// One copy of the app: its pid, when it still runs `exe`, and its children that run one of `workers`. Anything else,
// another copy from the same bundle or a child running another program, is left out.
export function copyOf(table, app, exe, workers) {
  const own = table.find((row) => row.pid === app);
  if (!own || own.exe !== path.resolve(exe)) return [];
  const wanted = new Set(workers.map((worker) => path.resolve(worker)));
  return [app, ...table.filter((row) => row.ppid === app && wanted.has(row.exe)).map((row) => row.pid)];
}

export function alive(pid) {
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

// SIGTERM to each pid, then SIGKILL to what still runs after `graceMs`.
export async function terminate(pids, { graceMs = 5000, log = console.log } = {}) {
  for (const pid of pids) signal(pid, 'SIGTERM');
  const end = Date.now() + graceMs;
  let left = pids.filter(alive);
  while (left.length && Date.now() < end) {
    await sleep(100);
    left = left.filter(alive);
  }
  if (!left.length) return;
  log(`still running after ${graceMs / 1000} s, sending SIGKILL to ${left.join(', ')}`);
  for (const pid of left) signal(pid, 'SIGKILL');
  await sleep(500);
  left = left.filter(alive);
  if (left.length) throw new Error(`still running after SIGKILL: ${left.join(', ')}`);
}

// The icon as the build stamp keeps it: its name and the hash of its contents, so a new icon under the same name
// rebuilds the bundle too.
export function iconStamp(dir, name) {
  if (!name) return null;
  const sha256 = crypto.createHash('sha256').update(fs.readFileSync(path.join(dir, name))).digest('hex');
  return { name, sha256 };
}

// Why the icon makes a rebuild, or null. A stamp from before the hash kept the name alone, which counts as a change.
export function iconChange(stamped, want) {
  const name = (icon) => (typeof icon === 'string' ? icon : icon?.name ?? null);
  if (!want) return stamped ? 'the icon went away' : null;
  if (name(stamped) !== want.name) return `the icon is now ${want.name}`;
  if (typeof stamped === 'string' || stamped.sha256 !== want.sha256) return `${want.name} changed`;
  return null;
}
