#!/usr/bin/env node
// Is this macOS host too stalled for Voice's timing-sensitive tests?
//
// Hosted macOS runners run those tests at a fraction of an M4's speed: a thread that asks for a
// 200 ms sleep wakes 100 ms late, and a recorder tick, a heartbeat or a fake-clock session then
// misses windows of 20 to 300 ms. The tests don't have a bug; the host stalls. This script prints
// what the host looks like and measures it. When the worst of 20 sleeps overshoots by more than
// the threshold, it sets RUKOO_VOICE_SLOW_HOST=1 for the later steps of the job (through
// GITHUB_ENV) and prints a warning. The tests listed in SKIPPED then return early and say so;
// Windows and Linux CI never set the variable, so they stay the gate for those behaviours.
//
//   node voice/scripts/host-probe.mjs                 macOS only, anywhere else it just says so
//   RUKOO_VOICE_HOST_PROBE_MAX_MS=0 node ...          forces the "slow" result, to see the warning
//
// The threshold, 50 ms, comes from the investigation in the slow-runner report: a starved Mac
// (background priority plus load) overshoots a 200 ms sleep by 87 to 193 ms, the hosted runner by
// about 100 ms, and an idle M4 by a few ms. 50 ms is also where `clock_mac`'s own window
// (150 to 250 ms for a 200 ms sleep) gives out. Tune it against a few runs of the job.
//
// The sleeps run in a small C program compiled here, because Node can't read the two mach
// counters. `mach_absolute_time` stops while the Mac sleeps and `mach_continuous_time` doesn't;
// the recorder tells a sleep from a stall by comparing them, so the log shows whether they move
// together on the host.

import { spawnSync } from 'node:child_process';
import { appendFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const SLEEPS = 20;
const SLEEP_MS = 200;
const MAX_OVERSHOOT_MS = Number(process.env.RUKOO_VOICE_HOST_PROBE_MAX_MS ?? 50);

// What RUKOO_VOICE_SLOW_HOST makes return early. Keep in step with `skip_on_slow_host` callers:
// `grep -rn skip_on_slow_host voice/crates`.
const SKIPPED = [
  'voice-recorder pause_resume_mark_and_dictation_lines',
  'voice-recorder a_new_default_device_moves_the_track',
  'voice-engine a_failing_bucket_never_slows_the_recorder',
];

const C = String.raw`
#include <mach/mach_time.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

static double ms(uint64_t ticks, mach_timebase_info_data_t t) {
  return (double)ticks * t.numer / t.denom / 1e6;
}

int main(int argc, char **argv) {
  int n = argc > 1 ? atoi(argv[1]) : 20, sleep_ms = argc > 2 ? atoi(argv[2]) : 200;
  mach_timebase_info_data_t t;
  mach_timebase_info(&t);
  struct timespec req = { sleep_ms / 1000, (sleep_ms % 1000) * 1000000L };
  for (int i = 0; i < n; i++) {
    uint64_t a0 = mach_absolute_time(), c0 = mach_continuous_time();
    nanosleep(&req, NULL);
    uint64_t a1 = mach_absolute_time(), c1 = mach_continuous_time();
    printf("%d abs %.1f -> %.1f cont %.1f -> %.1f\n", i + 1, ms(a0, t), ms(a1, t), ms(c0, t), ms(c1, t));
  }
  return 0;
}
`;

function run(cmd, args) {
  const r = spawnSync(cmd, args, { encoding: 'utf8' });
  return r.error ? `${cmd}: ${r.error.message}` : (r.stdout + r.stderr).trim();
}

function describe() {
  console.log('host');
  console.log(' ', run('sysctl', ['hw.model', 'hw.ncpu', 'machdep.cpu.brand_string', 'kern.hv_vmm_present']).replace(/\n/g, '\n  '));
  console.log(' ', run('sysctl', ['vm.loadavg']));
  // The job's shell (this script's parent) and this process: nice, priority and CPU share.
  console.log(run('ps', ['-o', 'pid,ni,pri,pcpu,comm', '-p', `${process.ppid},${process.pid}`]).replace(/^/gm, '  '));
}

function measure() {
  const dir = mkdtempSync(join(tmpdir(), 'host-probe-'));
  try {
    writeFileSync(join(dir, 'clock.c'), C);
    const build = spawnSync('cc', ['-O1', '-o', join(dir, 'clock'), join(dir, 'clock.c')], { encoding: 'utf8' });
    if (build.status !== 0) throw new Error(`cc failed: ${build.stderr || build.error}`);
    const out = spawnSync(join(dir, 'clock'), [String(SLEEPS), String(SLEEP_MS)], { encoding: 'utf8' });
    if (out.status !== 0) throw new Error(`the sampler failed: ${out.stderr || out.error}`);
    return out.stdout.trim().split('\n').map((line) => {
      const [, , a0, , a1, , c0, , c1] = line.split(' ').map(Number);
      const [absMs, contMs] = [a1 - a0, c1 - c0];
      return { absMs, contMs, overshoot: absMs - SLEEP_MS, line };
    });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function main() {
  if (process.platform !== 'darwin') {
    console.log('host-probe: macOS only, nothing to do here');
    return;
  }
  describe();
  console.log(`\n${SLEEPS} sleeps of ${SLEEP_MS} ms: before -> after of each counter in ms, and how late the sleep woke`);
  const samples = measure();
  for (const s of samples) {
    const drift = s.contMs - s.absMs;
    console.log(`  ${s.line}  late ${s.overshoot.toFixed(1)} ms  continuous minus absolute ${drift.toFixed(1)} ms`);
  }
  const worst = Math.max(...samples.map((s) => s.overshoot));
  const sorted = samples.map((s) => s.overshoot).sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  console.log(`\nlate by: median ${median.toFixed(1)} ms, worst ${worst.toFixed(1)} ms, threshold ${MAX_OVERSHOOT_MS} ms`);
  if (worst <= MAX_OVERSHOOT_MS) {
    console.log('this host keeps time well enough for the timing tests');
    return;
  }
  console.log('RUKOO_VOICE_SLOW_HOST=1');
  if (process.env.GITHUB_ENV) appendFileSync(process.env.GITHUB_ENV, 'RUKOO_VOICE_SLOW_HOST=1\n');
  console.log(
    `::warning::This macOS host wakes sleeping threads up to ${worst.toFixed(0)} ms late (limit ${MAX_OVERSHOOT_MS} ms). ` +
      `Skipped, and reported as passed: ${SKIPPED.join('; ')}. ` +
      'Their timing windows are 20 to 300 ms, so a stalled host fails them without a bug. Windows and Linux CI still run them. ' +
      'The speaker label tests judge the host by their own measurement.',
  );
}

try {
  main();
} catch (error) {
  // A broken probe must not fail the job: the tests then run as they did before it.
  console.log(`::warning::host-probe failed, so no host check was made: ${error.message}`);
}
