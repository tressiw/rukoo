// voice/scripts/lib/dev-app.mjs, which voice/scripts/dev-app-macos.mjs uses to stop the copy it started and to decide
// whether the bundle needs a rebuild. The process test runs /bin/sh and /bin/sleep through symlinks in a fake bundle
// in a temp folder, and stops only those, by pid.
'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { pathToFileURL } = require('node:url');

const LIB = pathToFileURL(path.join(__dirname, '..', 'voice', 'scripts', 'lib', 'dev-app.mjs')).href;
const lib = () => import(LIB);

function folder(t) {
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'rukoo-dev-app-')));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function until(what, fn, ms = 5000) {
  const end = Date.now() + ms;
  for (;;) {
    const value = await fn();
    if (value) return value;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(50);
  }
}

test('copyOf takes the recorded app and its workers, and nothing else from the bundle', async () => {
  const { copyOf } = await lib();
  const app = '/b/MacOS/rukoo-voice';
  const local = '/b/MacOS/rukoo-voice-local';
  const recorder = '/b/MacOS/rukoo-voice-recorder';
  const table = [
    { pid: 10, ppid: 1, exe: app },
    { pid: 11, ppid: 10, exe: local },
    { pid: 12, ppid: 10, exe: recorder },
    { pid: 13, ppid: 10, exe: '/usr/bin/other' },
    { pid: 20, ppid: 1, exe: app },
    { pid: 21, ppid: 20, exe: local },
    { pid: 30, ppid: 1, exe: '/usr/bin/reused' },
  ];
  assert.deepEqual(copyOf(table, 10, app, [local]), [10, 11]);
  // A recorded pid that now runs another program, or no longer runs, stops nothing.
  assert.deepEqual(copyOf(table, 30, app, [local]), []);
  assert.deepEqual(copyOf(table, 99, app, [local]), []);
});

test('a rebuild stops the recorded copy and its worker, and leaves its recorder and another copy running', { skip: process.platform !== 'darwin' && 'ps comm holds the full path on macOS only' }, async (t) => {
  const { alive, copyOf, processTable, runningFrom, terminate } = await lib();
  const macos = path.join(folder(t), 'Rukoo Voice Dev.app', 'Contents', 'MacOS');
  fs.mkdirSync(macos, { recursive: true });
  // Symlinks, not copies: macOS kills a copy of a system binary at launch, and `ps` reports the path that was run.
  const exe = (name, from) => {
    const file = path.join(macos, name);
    fs.symlinkSync(from, file);
    return file;
  };
  const app = exe('rukoo-voice', '/bin/sh');
  const worker = exe('rukoo-voice-local', '/bin/sleep');
  const recorder = exe('rukoo-voice-recorder', '/bin/sleep');

  const started = [];
  t.after(() => {
    for (const pid of started) {
      try { process.kill(pid, 'SIGKILL'); } catch (_) { /* gone */ }
    }
  });
  // Each "app" starts a worker and a recorder as its children, then waits.
  const startCopy = () => {
    const child = spawn(app, ['-c', '"$1" 600 & "$2" 600 & wait', 'sh', worker, recorder], { stdio: 'ignore' });
    started.push(child.pid);
    return child.pid;
  };
  const recorded = startCopy();
  const other = startCopy();
  const children = await until('both copies to start their helpers', () => {
    const table = processTable();
    const of = (parent) => table.filter((row) => row.ppid === parent).map((row) => row.pid);
    return of(recorded).length === 2 && of(other).length === 2 ? table : null;
  });
  for (const row of children) if (row.ppid === recorded || row.ppid === other) started.push(row.pid);
  const childOf = (parent, file) => children.find((row) => row.ppid === parent && row.exe === file).pid;

  const copy = copyOf(processTable(), recorded, app, [worker]);
  assert.deepEqual(copy.sort(), [recorded, childOf(recorded, worker)].sort());
  await terminate(copy, { graceMs: 2000, log: () => {} });

  assert.equal(alive(recorded), false);
  assert.equal(alive(childOf(recorded, worker)), false);
  assert.equal(alive(childOf(recorded, recorder)), true, 'the recorded copy\'s recorder keeps running');
  assert.equal(alive(other), true, 'a copy the script did not start keeps running');
  assert.equal(alive(childOf(other, worker)), true);
  // --stop's view of the bundle still finds everything that is left.
  assert.deepEqual(
    runningFrom(processTable(), [app, worker, recorder]).sort(),
    [other, childOf(other, worker), childOf(other, recorder), childOf(recorded, recorder)].sort(),
  );
});
