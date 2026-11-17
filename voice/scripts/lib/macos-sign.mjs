// The signature Rukoo Voice gives the NeMo runtime's six dylibs, in one place: voice/nemo-runtime/pack-macos.mjs pins
// their hashes in voice/nemo-runtime/manifest.json, and voice/scripts/package-macos.mjs signs the copy inside the app.
// If the two signed with different flags, every dylib's bytes and hash would change when the runtime is added to
// Rukoo Voice.app, and voice-core would never verify the folder it ships (crates/voice-core/src/speakers/runtime.rs,
// verify_voice).
//
// An ad-hoc signature is deterministic for the same bytes under the same file name: codesign takes a dylib's identifier
// from its file name, and `--timestamp=none` leaves no time in the signature. So a dylib pack-macos.mjs signed is
// byte-for-byte the one package-macos.mjs would write, and package-macos.mjs leaves it alone (isAdHocHardened).

import { run } from './run.mjs';

// The six dylibs voice/nemo-runtime/build-macos.sh builds and voice-core's macOS runtime folder holds.
export const NEMO_DYLIBS = [
  'libnemo_speech_asr_c.dylib',
  'libnemo_speech_asr.dylib',
  'libggml.dylib',
  'libggml-base.dylib',
  'libggml-cpu.dylib',
  'libggml-blas.dylib',
];

// The codesign arguments for one target. Inside out, never `--deep`, and never a timestamp for an ad-hoc signature: an
// ad-hoc signature can't be timestamped, and a timestamp would make the bytes differ between runs.
export function codesignArgs(target, { signer = '-', entitlements, identifier } = {}) {
  const args = ['--force', '--sign', signer, '--options', 'runtime'];
  args.push(signer === '-' ? '--timestamp=none' : '--timestamp');
  if (identifier) args.push('--identifier', identifier);
  if (entitlements) args.push('--entitlements', entitlements);
  args.push(target);
  return args;
}

export function codesign(target, options = {}) {
  run('codesign', codesignArgs(target, options));
}

// What `codesign -d -vv` says about a target: Signature=adhoc, flags=0x...(...runtime...), Authority=...
export function signatureInfo(target) {
  const out = run('codesign', ['-d', '-vv', target], { allowFailure: true });
  const text = `${out.stdout ?? ''}${out.stderr ?? ''}`;
  if (out.status !== 0) throw new Error(`codesign could not read ${target}:\n${text.trim()}`);
  return text;
}

// The entitlements a signed target carries, as a sorted list of keys. An empty signature has none, and codesign prints
// them as an XML plist.
export function entitlementsOf(target) {
  const out = run('codesign', ['-d', '--entitlements', ':-', target], { allowFailure: true });
  const text = `${out.stdout ?? ''}`.trim();
  if (!text) {
    if (out.status !== 0) throw new Error(`codesign could not read the entitlements of ${target}`);
    return [];
  }
  const converted = run('plutil', ['-convert', 'json', '-o', '-', '-'], { input: text });
  let parsed;
  try {
    parsed = JSON.parse(converted.stdout);
  } catch (error) {
    throw new Error(`codesign printed entitlements for ${target} that aren't a plist: ${error.message}`);
  }
  for (const [key, value] of Object.entries(parsed)) {
    if (value !== true) throw new Error(`${target} has ${key} set to ${JSON.stringify(value)} instead of true`);
  }
  return Object.keys(parsed).sort();
}

// True when the file already carries exactly the ad-hoc hardened-runtime signature with no entitlements that
// package-macos.mjs gives a NeMo dylib. Signing it again would produce the same bytes, but leaving it alone keeps the
// hashes voice/nemo-runtime/manifest.json pins, whatever codesign version runs the packaging.
export function isAdHocHardened(target) {
  const out = run('codesign', ['-d', '-vv', target], { allowFailure: true });
  const text = `${out.stdout ?? ''}${out.stderr ?? ''}`;
  if (out.status !== 0) return false;
  if (!/^Signature=adhoc$/m.test(text)) return false;
  if (!/flags=0x[0-9a-f]+\([^)]*\bruntime\b/m.test(text)) return false;
  const entitlements = run('codesign', ['-d', '--entitlements', ':-', target], { allowFailure: true });
  return `${entitlements.stdout ?? ''}`.trim().length === 0;
}
