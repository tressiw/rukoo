// Running a command for voice/scripts/package-macos.mjs without letting a secret reach the log.
//
// `notarytool submit` takes the app-specific password as `--password`: its other ways to get it are a keychain profile,
// which a CI runner doesn't have, and a prompt on the terminal, which a CI runner can't answer. So the password is on
// the command line, and every error and every line of output this helper prints has the secret values replaced with
// `***`. The release workflow's log is public.
//
// APPLE_TEAM_ID and APPLE_SIGNING_IDENTITY aren't redacted: both are in the signature of every app they sign, and the
// script prints the identity on purpose.

import { spawnSync } from 'node:child_process';

// The environment variables whose values never reach output: the names package-macos.mjs reads and the repository
// secret names of docs/development.md.
export const SECRET_ENV = ['APPLE_PASSWORD', 'APPLE_ID', 'APPLE_APP_SPECIFIC_PASSWORD', 'MAC_CSC_KEY_PASSWORD'];

export function secretValues(env = process.env) {
  return SECRET_ENV.map((name) => env[name]).filter((value) => typeof value === 'string' && value.length > 0);
}

// Longest first, so a secret that contains another is replaced whole.
export function redact(text, secrets = secretValues()) {
  let out = String(text);
  for (const secret of [...new Set(secrets)].sort((a, b) => b.length - a.length)) {
    out = out.split(secret).join('***');
  }
  return out;
}

// Runs a program and throws on a nonzero exit, with the command and what it said. `inherit` shows its output as it
// comes; a command that is given a secret should leave it off, so its output goes through `redact` before it is
// printed.
export function run(file, args, options = {}) {
  const secrets = secretValues(options.env ?? process.env);
  const result = spawnSync(file, args, {
    encoding: 'utf8',
    stdio: options.inherit ? 'inherit' : 'pipe',
    input: options.input,
    cwd: options.cwd,
    env: options.env ?? process.env,
  });
  if (result.error) {
    const hint = result.error.code === 'ENOENT' && file === 'cargo'
      ? ' Rust is in ~/.cargo/bin, which is not always on PATH.'
      : '';
    throw new Error(redact(`couldn't run ${file}: ${result.error.message}.${hint}`, secrets));
  }
  if (options.echo) {
    if (result.stdout) process.stdout.write(redact(result.stdout, secrets));
    if (result.stderr) process.stderr.write(redact(result.stderr, secrets));
  }
  if (result.status !== 0 && !options.allowFailure) {
    const said = options.echo ? '' : `${result.stdout ?? ''}${result.stderr ?? ''}`.trim();
    throw new Error(redact(`${file} ${args.join(' ')} failed with exit code ${result.status}${said ? `: ${said}` : ''}`, secrets));
  }
  return result;
}

// Notarization: submit and wait. notarytool's output is printed when it finishes, redacted, instead of as it comes.
export function notarize(file, env = process.env) {
  run('xcrun', ['notarytool', 'submit', file,
    '--apple-id', env.APPLE_ID,
    '--password', env.APPLE_PASSWORD,
    '--team-id', env.APPLE_TEAM_ID,
    '--wait'], { echo: true, env });
}
