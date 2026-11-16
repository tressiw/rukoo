'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { tempDir } = require('./tmp');
const { onPath, resolveExe, start, cliPath, cleanEnv } = require('../src/main/agents/proc');
const { CodexAdapter } = require('../src/main/agents/codex');

function fixture(t) {
  const dir = tempDir('rukoo-agent-discovery-');
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test('POSIX discovery finds executable symlinks without which and skips non-executable files', { skip: process.platform === 'win32' }, async (t) => {
  const dir = fixture(t);
  const first = path.join(dir, 'first');
  const second = path.join(dir, 'bin with spaces');
  fs.mkdirSync(first);
  fs.mkdirSync(second);
  const blocked = path.join(first, 'codex');
  fs.writeFileSync(blocked, 'not executable', { mode: 0o600 });
  const script = path.join(dir, 'launcher');
  fs.writeFileSync(script, '#!/bin/sh\nprintf "agent-ready"\n', { mode: 0o700 });
  const executable = path.join(second, 'codex');
  fs.symlinkSync(script, executable);
  const oldPath = process.env.PATH;
  t.after(() => { if (oldPath === undefined) delete process.env.PATH; else process.env.PATH = oldPath; });
  process.env.PATH = [first, second].join(path.delimiter);
  assert.deepEqual(await onPath('codex', { platform: 'linux' }), [executable]);
  assert.equal(await resolveExe([blocked], 'codex'), executable);
  assert.equal(await resolveExe([blocked], null), null);
  assert.equal(await resolveExe([second], null), null);
  const child = start(executable, []);
  let output = '';
  child.stdout.on('data', (data) => { output += data; });
  const code = await new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', resolve);
  });
  assert.equal(code, 0);
  assert.equal(output, 'agent-ready');
});

test('Codex finds a per-user Linux installation when the desktop PATH omits it', { skip: process.platform === 'win32' }, async (t) => {
  const home = fixture(t);
  const bin = path.join(home, '.local', 'bin');
  fs.mkdirSync(bin, { recursive: true });
  const executable = path.join(bin, 'codex');
  fs.writeFileSync(executable, '#!/bin/sh\nexit 0\n', { mode: 0o700 });
  t.mock.method(os, 'homedir', () => home);
  const adapter = new CodexAdapter({ config: () => ({ enabled: true }) });
  assert.equal(await adapter.exe(), executable);
  assert.equal(adapter.defaultExe(), executable);
});

// No ~/.nvm: reading its versions folder fails as it does on a Mac without nvm.
const noNvm = () => {
  throw Object.assign(new Error('ENOENT: no such file or directory'), { code: 'ENOENT' });
};
const HOME = '/Users/Mail User';
const MANAGERS = [`${HOME}/.volta/bin`, `${HOME}/.local/share/fnm/aliases/default/bin`, `${HOME}/Library/Application Support/fnm/aliases/default/bin`];

test('macOS supplements Finder PATH with local, Homebrew and Node version manager locations without changing inherited precedence', () => {
  const base = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', KEEP: 'yes', CODEX_THREAD_ID: 'private' };
  const options = { platform: 'darwin', home: HOME, readdir: noNvm };
  const expected = ['/usr/bin:/bin:/usr/sbin:/sbin', `${HOME}/.local/bin`, '/opt/homebrew/bin', '/usr/local/bin', ...MANAGERS].join(':');
  assert.equal(cliPath(base, options), expected);
  assert.equal(cliPath({}, options), expected);
  assert.deepEqual(cleanEnv({}, base, options), { PATH: expected, KEEP: 'yes' });
  assert.equal(base.PATH, '/usr/bin:/bin:/usr/sbin:/sbin', 'does not mutate the parent environment');
  assert.equal(
    cliPath({ PATH: '/usr/local/bin:/usr/bin:/opt/homebrew/bin' }, options),
    ['/usr/local/bin:/usr/bin:/opt/homebrew/bin', `${HOME}/.local/bin`, ...MANAGERS].join(':')
  );
  for (const platform of ['linux', 'win32']) {
    let read = false;
    const readdir = () => ((read = true), ['v24.6.0']);
    assert.equal(cliPath(base, { platform, home: HOME, readdir }), base.PATH);
    assert.deepEqual(cleanEnv({}, { KEEP: 'yes' }, { platform, home: HOME, readdir }), { KEEP: 'yes' });
    assert.equal(read, false, `${platform} never looks for nvm`);
  }
});

test('macOS adds the bin folder of every nvm version, newest first, after the other locations', () => {
  const asked = [];
  const readdir = (dir) => {
    asked.push(dir);
    return ['v9.11.2', 'v24.6.0', 'system', 'v24.10.1', '.DS_Store', 'v20.19.0'];
  };
  const nvm = (version) => `${HOME}/.nvm/versions/node/${version}/bin`;
  const result = cliPath({ PATH: `/usr/bin:${nvm('v20.19.0')}` }, { platform: 'darwin', home: HOME, readdir }).split(':');
  assert.deepEqual(asked, [`${HOME}/.nvm/versions/node`]);
  assert.deepEqual(result, [
    '/usr/bin',
    // A version already on the inherited PATH keeps its place there.
    nvm('v20.19.0'),
    `${HOME}/.local/bin`,
    '/opt/homebrew/bin',
    '/usr/local/bin',
    ...MANAGERS,
    // Compared per number, so v24.10.1 comes before v24.6.0 and v9.11.2 last.
    nvm('v24.10.1'),
    nvm('v24.6.0'),
    nvm('v9.11.2')
  ]);
  assert.deepEqual(cliPath({ PATH: '/usr/bin' }, { platform: 'darwin', home: HOME, readdir: () => [] }).split(':'), ['/usr/bin', `${HOME}/.local/bin`, '/opt/homebrew/bin', '/usr/local/bin', ...MANAGERS]);
});

test('a Finder launch on macOS finds Claude Code installed with nvm', { skip: process.platform === 'win32' }, async (t) => {
  const home = fixture(t);
  const bin = path.join(home, '.nvm', 'versions', 'node', 'v24.6.0', 'bin');
  fs.mkdirSync(bin, { recursive: true });
  fs.mkdirSync(path.join(home, '.nvm', 'versions', 'node', 'v22.1.0', 'bin'), { recursive: true });
  const executable = path.join(bin, 'claude');
  fs.writeFileSync(executable, '#!/bin/sh\nexit 0\n', { mode: 0o700 });
  const finder = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin' };
  assert.deepEqual(await onPath('claude', { env: finder, platform: 'darwin', home }), [executable]);
  // The child gets the same folders, so the launcher's /usr/bin/env node finds nvm's node.
  assert.ok(cleanEnv({}, finder, { platform: 'darwin', home }).PATH.split(':').includes(bin));
});

test('macOS discovery and child PATH agree for npm launchers that need env node', { skip: process.platform === 'win32' }, async (t) => {
  const home = fixture(t);
  const bin = path.join(home, '.local', 'bin');
  fs.mkdirSync(bin, { recursive: true });
  const executable = path.join(bin, 'rukoo-fake-agent');
  fs.writeFileSync(executable, '#!/usr/bin/env node\nprocess.stdout.write("node-ready");\n', { mode: 0o700 });
  fs.symlinkSync(process.execPath, path.join(bin, 'node'));
  const options = { platform: 'darwin', home };
  const base = { PATH: path.join(home, 'empty-path') };
  assert.deepEqual(await onPath('rukoo-fake-agent', { ...options, env: base }), [executable]);
  const child = start(executable, [], { env: cleanEnv({}, base, options) });
  let output = '';
  child.stdout.on('data', (data) => { output += data; });
  const code = await new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', resolve);
  });
  assert.equal(code, 0);
  assert.equal(output, 'node-ready');
});
