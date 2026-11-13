#!/usr/bin/env node
// Makes the tests that skip themselves visible in a CI job.
//
// A test that can't run on its host returns early with a message on stderr, and libtest hides the
// stderr of a passing test, so a green job looks the same whether the test ran or skipped. Those tests
// also append one line, `<crate> <test>: <reason>`, to the file named by RUKOO_VOICE_SKIP_LOG
// (`log_skip` in each crate's test helpers; `grep -rn log_skip voice/crates` lists them). Without
// the variable they write nothing.
//
//   node voice/scripts/skip-log.mjs init     before the tests: names the file in GITHUB_ENV and empties it
//   node voice/scripts/skip-log.mjs report   after the tests, with `if: always()`: one ::warning:: per
//                                            line, then a count, so a skip shows in the run's annotations
//
// Both commands only read the environment and the file, so they run the same on every runner.

import { appendFileSync, existsSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const NAME = 'RUKOO_VOICE_SKIP_LOG';

// What a workflow command's message may not hold as is.
const escape = (text) => text.replace(/%/g, '%25').replace(/\r/g, '%0D').replace(/\n/g, '%0A');

function init(env) {
  const file = join(env.RUNNER_TEMP || tmpdir(), 'voice-skips.log');
  rmSync(file, { force: true });
  console.log(`${NAME}=${file}`);
  if (env.GITHUB_ENV) appendFileSync(env.GITHUB_ENV, `${NAME}=${file}\n`);
}

// The lines of the skip log, as `::warning::` commands followed by the count.
function report(env) {
  const file = env[NAME];
  if (!file) return ['no skip log is set (RUKOO_VOICE_SKIP_LOG), so skipped tests are not counted'];
  const lines = existsSync(file) ? readFileSync(file, 'utf8').split('\n').filter((line) => line.trim() !== '') : [];
  const out = lines.map((line) => `::warning title=Test skipped::${escape(line)}`);
  out.push(
    lines.length === 0
      ? 'No test skipped itself.'
      : `${lines.length} test${lines.length === 1 ? '' : 's'} skipped themselves and libtest reports them as passed.`,
  );
  return out;
}

const command = process.argv[2];
if (command === 'init') {
  init(process.env);
} else if (command === 'report') {
  for (const line of report(process.env)) console.log(line);
} else {
  console.error('usage: skip-log.mjs init|report');
  process.exit(2);
}
