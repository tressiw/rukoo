// voice/scripts/lib/run.mjs, which voice/scripts/package-macos.mjs runs every command through: a failed notarization
// must not print the Apple app-specific password or Apple ID into the release workflow's public log. A stub `xcrun`
// first on PATH stands in for notarytool, so nothing reaches Apple.
'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { pathToFileURL } = require('node:url');

const LIB = pathToFileURL(path.join(__dirname, '..', 'voice', 'scripts', 'lib', 'run.mjs')).href;
const PASSWORD = 'abcd-efgh-ijkl-mnop';
const APPLE_ID = 'notary-test@example.invalid';

function folder(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rukoo-package-macos-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

// An `xcrun` that records its arguments, repeats them on stdout and stderr as a careless tool would, and fails.
function stubXcrun(dir) {
  const bin = path.join(dir, 'bin');
  fs.mkdirSync(bin);
  const stub = path.join(bin, 'xcrun');
  fs.writeFileSync(stub, [
    '#!/bin/sh',
    `printf '%s\\n' "$@" > "${path.join(dir, 'args.txt')}"`,
    'echo "stdout: $*"',
    'echo "stderr: $*" >&2',
    'exit 69',
    '',
  ].join('\n'));
  fs.chmodSync(stub, 0o755);
  return bin;
}

test('a failed notarization keeps the password and the Apple ID out of stdout, stderr and the error', { skip: process.platform === 'win32' && 'the stub xcrun is a shell script' }, (t) => {
  const dir = folder(t);
  const bin = stubXcrun(dir);
  const script = `
    import { notarize } from ${JSON.stringify(LIB)};
    try {
      notarize(${JSON.stringify(path.join(dir, 'Rukoo-Voice.zip'))});
      console.log('NO ERROR');
    } catch (error) {
      console.error('MESSAGE<' + error.message + '>');
      process.exitCode = 1;
    }
  `;
  const out = spawnSync(process.execPath, ['--input-type=module', '-e', script], {
    encoding: 'utf8',
    env: {
      ...process.env,
      PATH: `${bin}${path.delimiter}${process.env.PATH}`,
      APPLE_ID,
      APPLE_PASSWORD: PASSWORD,
      APPLE_TEAM_ID: 'TEAM123456',
    },
  });

  // The stub got the real values, so the redaction below is what keeps them out.
  const args = fs.readFileSync(path.join(dir, 'args.txt'), 'utf8').split('\n');
  assert.equal(args[args.indexOf('--password') + 1], PASSWORD);
  assert.equal(args[args.indexOf('--apple-id') + 1], APPLE_ID);

  assert.equal(out.status, 1, out.stdout + out.stderr);
  const message = /MESSAGE<([\s\S]*)>/.exec(out.stderr)?.[1];
  assert.ok(message, out.stderr);
  assert.match(message, /^xcrun notarytool submit .* failed with exit code 69$/);
  assert.match(message, /--password \*\*\*/);
  for (const [name, text] of [['stdout', out.stdout], ['stderr', out.stderr], ['message', message]]) {
    assert.ok(!text.includes(PASSWORD), `${name} holds the password: ${text}`);
    assert.ok(!text.includes(APPLE_ID), `${name} holds the Apple ID: ${text}`);
  }
  // notarytool's own output still shows, redacted.
  assert.match(out.stdout, /stdout: notarytool submit .*--password \*\*\*/);
  assert.match(out.stderr, /stderr: notarytool submit .*--password \*\*\*/);
});

test('redact replaces every secret, the longest first, and leaves other text alone', async () => {
  const { redact, secretValues } = await import(LIB);
  assert.equal(redact('a pass and a password', ['pass', 'password']), 'a *** and a ***');
  assert.equal(redact('nothing secret', []), 'nothing secret');
  assert.deepEqual(secretValues({ APPLE_PASSWORD: 'p', APPLE_ID: '', APPLE_TEAM_ID: 'T', MAC_CSC_KEY_PASSWORD: 'k' }), ['p', 'k']);
});
