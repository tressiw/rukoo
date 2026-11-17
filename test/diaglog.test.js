'use strict';

// The log in the profile (src/main/diaglog.js): one line per entry, a size cap with one older file, nothing that names
// a person or holds a token, and a window that can't flood it.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { tempDir } = require('./tmp');
const { DiagLog, scrub } = require('../src/main/diaglog');
const { Engine } = require('../src/main/engine');

const read = (file) => (fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '');

test('a line has the time, the level and where it came from, and the file turns over at its size', () => {
  const dir = tempDir('rukoo-diag-');
  let now = Date.UTC(2026, 10, 16, 9, 30);
  const log = new DiagLog(dir, { maxBytes: 400, now: () => now });
  log.info('sync', 'account=1234abcd demo ms=12 folders=6 new=0');
  assert.equal(read(path.join(dir, 'rukoo.log')), '2026-11-16T09:30:00.000Z INFO  sync: account=1234abcd demo ms=12 folders=6 new=0\n');
  for (let i = 0; i < 10; i++) {
    now += 1000;
    log.warn('window main', `refresh failed ${i}`);
  }
  const current = read(path.join(dir, 'rukoo.log'));
  const older = read(path.join(dir, 'rukoo.1.log'));
  assert.ok(Buffer.byteLength(current) <= 400 && Buffer.byteLength(older) <= 400, 'both files stay under the cap');
  assert.match(current, /refresh failed 9\n$/);
  assert.match(older, /refresh failed/);
  assert.ok(!fs.existsSync(path.join(dir, 'rukoo.2.log')), 'one older file only');

  // A new start goes on in the same file, with its size counted.
  const again = new DiagLog(dir, { maxBytes: 400, now: () => now });
  again.info('app', 'start');
  assert.match(read(path.join(dir, 'rukoo.log')), /INFO  app: start\n$/);
});

test('addresses, tokens, paths in the home folder and line breaks stay out', () => {
  const home = os.homedir();
  const out = scrub(`Login failed for anna.de.vries@example.com: token ya29.a0AfB_byCAbcdefghijklmnopqrstuvwxyz0123456789 at\n    at refreshOnce (file:///${home.replace(/\\/g, '/')}/AppData/Local/Programs/rukoo/resources/app.asar/src/renderer/app.js:201:22)\n    at ${home}\\other\\place.js:1:1`);
  assert.doesNotMatch(out, /anna|example\.com/);
  assert.doesNotMatch(out, /abcdefghijklmnopqrstuvwxyz0123456789/);
  assert.ok(!out.includes(home), 'the home folder');
  assert.match(out, /<address>/);
  assert.match(out, /\(src\/renderer\/app\.js:201:22\)/);
  assert.doesNotMatch(out, /\n/);
  assert.ok(scrub('x'.repeat(5000)).length <= 2001, 'capped');
});

// The scrub runs in the main process on what a window sends. Long text without spaces, such as a data URL or a
// one-line stack, took about 0.8 s an entry when the patterns tried again from every character.
test('long text without spaces scrubs in one pass', () => {
  const log = new DiagLog(tempDir('rukoo-diag-'));
  for (const unit of ['a', 'a.b', '/x', 'a@b', 'x@y.']) {
    const s = unit.repeat(Math.ceil(20000 / unit.length)).slice(0, 20000);
    const started = process.hrtime.bigint();
    log.fromWindow('main', { kind: 'error', message: s, stack: s, reason: s });
    const ms = Number(process.hrtime.bigint() - started) / 1e6;
    assert.ok(ms < 150, `${JSON.stringify(unit)} took ${Math.round(ms)} ms`);
  }
});

test("a window's lines: known kinds and fields only, capped, and at most 30 a minute", () => {
  const dir = tempDir('rukoo-diag-');
  let now = Date.UTC(2026, 10, 16, 9, 30);
  const log = new DiagLog(dir, { now: () => now });
  assert.equal(log.fromWindow('main', { kind: 'refresh', message: 'Something broke', stack: 'Error: Something broke\n at x', failures: 2, subject: 'Private', from: 'a@b.cd' }), true);
  assert.equal(log.fromWindow('main', { kind: 'anything', message: 'no' }), false);
  assert.equal(log.fromWindow('main', null), false);
  assert.equal(log.fromWindow('main', { kind: 'list', reason: 'stale', ms: 4321.4 }), true);
  const lines = read(path.join(dir, 'rukoo.log')).trim().split('\n');
  assert.equal(lines.length, 2);
  assert.match(lines[0], /WARN  window main: refresh failures=2 Something broke stack: Error: Something broke \| at x$/);
  assert.doesNotMatch(lines[0], /Private|a@b/);
  assert.match(lines[1], /INFO  window main: list reason=stale ms=4321$/);

  // A token or an address where a field's cap falls is taken out whole, before the cap cuts it short.
  const token = 'A'.repeat(64);
  log.fromWindow('main', { kind: 'error', message: `${'x '.repeat(240)}token=${token}` });
  log.fromWindow('main', { kind: 'error', stack: `${'y '.repeat(740)}at anna.de.vries@example.com` });
  // Past the bound on what is read at all (20,000 characters), the word cut in two goes too: here 22 characters of a
  // token, which would land well inside the cap once the run before it shrank to <redacted>.
  log.fromWindow('main', { kind: 'error', message: `${'B'.repeat(19970)} secret ${'C'.repeat(60)}` });
  const capped = read(path.join(dir, 'rukoo.log')).trim().split('\n').slice(-3);
  assert.doesNotMatch(capped.join('\n'), /AAAA|anna|example|CCCC/);
  assert.match(capped[0], /x <redacted>$/);
  assert.match(capped[1], /y at <address>$/);
  assert.match(capped[2], /error <redacted> secret$/);

  for (let i = 0; i < 37; i++) log.fromWindow('main', { kind: 'error', message: `boom ${i}` });
  assert.equal(read(path.join(dir, 'rukoo.log')).trim().split('\n').length, 30, 'two before, then 28 more this minute');
  assert.equal(log.fromWindow('compose', { kind: 'error', message: 'its own allowance' }), true);
  now += 60000;
  log.fromWindow('main', { kind: 'error', message: 'next minute' });
  assert.match(read(path.join(dir, 'rukoo.log')), /WARN  window main: 12 more lines left out\n.*window main: error next minute\n$/);
});

test('each account sync writes one line: how long, how many folders and new emails, and an error as a code', async () => {
  const dir = tempDir('sem-test-');
  const lines = [];
  const engine = new Engine({ dataDir: dir, log: (level, target, message) => lines.push(`${level} ${target}: ${message}`) }).init();
  const acc = await engine.addAccount({ type: 'demo' });
  lines.length = 0;
  await engine.syncAccount(acc.id);
  assert.equal(lines.length, 1);
  assert.match(lines[0], new RegExp(`^info sync: account=${acc.id.slice(0, 8)} demo start=\\d{4}-\\d\\d-\\d\\dT[\\d:.]+Z ms=\\d+ folders=\\d+ new=0$`));
  assert.doesNotMatch(lines[0], /@/);

  // A sync that fails names what went wrong as a code, never the server's text.
  engine.session(acc).listFolders = async () => {
    throw Object.assign(new Error('Connection timed out for demo@example.com'), { code: 'ETIMEDOUT' });
  };
  await assert.rejects(engine.syncAccount(acc.id));
  assert.match(lines[1], /^warn sync: account=\S+ demo .* folders=0 new=0 error=timeout$/);
  assert.doesNotMatch(lines[1], /example/);
  await engine.close();
});
