import { runInDurableObject } from 'cloudflare:test';
import { env } from 'cloudflare:workers';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { hmac, toBase64url } from '../src/bytes.js';
import {
  INVITE,
  KDF,
  accountName,
  accountStub,
  call,
  device,
  key,
  login,
  newEmail,
  push,
  recordId,
  salt,
  sealed,
  signup
} from './helpers.js';

afterEach(() => vi.useRealTimers());

const MINUTE = 60 * 1000;

function signupBody(overrides = {}) {
  return {
    email: newEmail(),
    invite: INVITE,
    kdf: KDF,
    salt: salt(),
    authKey: key(),
    wrappedKey: sealed(),
    recoveryAuth: key(),
    recoveryWrappedKey: sealed(),
    device: device(),
    ...overrides
  };
}

function wrongLogin(person) {
  return call('POST', '/v1/login', { body: { email: person.email, authKey: key(), device: device() } });
}

// A signed-in call that checks the password again: a new recovery key.
function recovery(token, authKey) {
  return call('POST', '/v1/recovery', { token, body: { authKey, recoveryAuth: key(), recoveryWrappedKey: sealed() } });
}

// The signed-in call that only checks the password, which a device sends before it deletes the booking page.
function check(token, authKey) {
  return call('POST', '/v1/password/check', { token, body: { authKey } });
}

// Moves the clock forward for the Worker and its objects, which run in the same isolate as the tests.
function later(ms) {
  vi.setSystemTime(Date.now() + ms);
}

async function tables(email) {
  return runInDurableObject(await accountStub(email), (instance, state) =>
    state.storage.sql
      .exec("SELECT name FROM sqlite_master WHERE type = 'table'")
      .toArray()
      .map((r) => r.name)
  );
}

// 32 bytes that can't pass for a sealed value: the version byte is wrong. A random key() starts with 0x01 once in 256.
const notSealed = () => toBase64url(new Uint8Array(32).fill(2));

describe('sign-up', () => {
  it('makes an account with an invite code and returns a session and the account', async () => {
    const body = signupBody({ email: '  Someone.New@Example.COM ', invite: ' second-code ' });
    const res = await call('POST', '/v1/signup', { body });
    expect(res.status).toBe(201);
    const name = await accountName('someone.new@example.com');
    expect(res.body.token).toMatch(new RegExp(`^${name}\\.[A-Za-z0-9_-]{43}$`));
    const { account } = res.body;
    expect(account.email).toBe('someone.new@example.com');
    expect(account.kdf).toEqual(KDF);
    expect(account.usage).toEqual({ records: 0, bytes: 0 });
    expect(account.limits).toEqual({ records: 250000, bytes: 104857600, recordBytes: 16384, batch: 500 });
    expect(account.devices).toHaveLength(1);
    expect(account.devices[0]).toMatchObject({ name: 'Test desktop', platform: 'linux', current: true });
    expect(account.devices[0].id).toMatch(/^[A-Za-z0-9_-]{22}$/);
    expect(account.devices[0].createdAt).toBe(account.createdAt);
  });

  it('needs a valid invite code in invite mode', async () => {
    const missing = await call('POST', '/v1/signup', { body: signupBody({ invite: undefined }) });
    expect(missing.status).toBe(403);
    expect(missing.body.error).toBe('invite_required');
    const wrong = await call('POST', '/v1/signup', { body: signupBody({ invite: 'third-code' }) });
    expect(wrong.status).toBe(403);
    expect(wrong.body.error).toBe('invite_required');
    const none = await call('POST', '/v1/signup', { body: signupBody(), vars: { INVITE_CODES: undefined } });
    expect(none.status).toBe(403);
    expect(none.body.error).toBe('invite_required');
  });

  it('needs no invite code when sign-ups are open', async () => {
    const res = await call('POST', '/v1/signup', {
      body: signupBody({ invite: undefined }),
      vars: { SIGNUPS: 'open' }
    });
    expect(res.status).toBe(201);
  });

  it('refuses every sign-up when closed', async () => {
    const res = await call('POST', '/v1/signup', { body: signupBody(), vars: { SIGNUPS: 'closed' } });
    expect(res.status).toBe(403);
    expect(res.body.error).toBe('signups_closed');
  });

  it('refuses a second account for the same address, however it is written', async () => {
    const person = await signup();
    const res = await call('POST', '/v1/signup', { body: signupBody({ email: ` ${person.email.toUpperCase()}` }) });
    expect(res.status).toBe(409);
    expect(res.body.error).toBe('exists');
    expect((await login(person)).status).toBe(200);
  });

  it('checks every field', async () => {
    const bad = {
      email: [
        'not-an-address',
        'a@b',
        'two@at@example.com',
        'sp ace@example.com',
        42,
        `${'a'.repeat(250)}@example.com`
      ],
      salt: [
        toBase64url(new Uint8Array(15)),
        toBase64url(new Uint8Array(17)),
        'AAAAAAAAAAAAAAAAAAAAAA==',
        'AAAAAAAAAAAAAAAAAAAAAB',
        '!!'
      ],
      authKey: [toBase64url(new Uint8Array(31)), null, toBase64url(new Uint8Array(32)).replace(/A$/, 'B')],
      recoveryAuth: [toBase64url(new Uint8Array(33)), ''],
      wrappedKey: [toBase64url(new Uint8Array(61)), sealed(28), sealed(300), 'not base64!'],
      recoveryWrappedKey: [toBase64url(Uint8Array.from([2, ...new Uint8Array(60)]))],
      kdf: [
        { ...KDF, alg: 'argon2i' },
        { ...KDF, m: 19455 },
        { ...KDF, m: 1048577 },
        { ...KDF, t: 1 },
        { ...KDF, t: 11 },
        { ...KDF, p: 0 },
        { ...KDF, p: 5 },
        { ...KDF, m: 65536.5 },
        { ...KDF, m: '65536' },
        { ...KDF, extra: 1 },
        { alg: 'argon2id', m: 65536, t: 3 },
        'argon2id'
      ],
      device: [undefined, 'desktop', { platform: 'linux' }, { name: 5 }, { name: 'x', platform: 7 }]
    };
    for (const [field, values] of Object.entries(bad)) {
      for (const value of values) {
        const res = await call('POST', '/v1/signup', { body: signupBody({ [field]: value }) });
        expect(res.status, `${field}: ${JSON.stringify(value)}`).toBe(400);
        expect(res.body.error).toBe('invalid');
      }
    }
  });

  it('refuses bodies that are not a JSON object, and oversized ones', async () => {
    for (const body of ['not json', '[1, 2]', 'null', '"text"']) {
      const res = await call('POST', '/v1/signup', { body });
      expect(res.status).toBe(400);
      expect(res.body.error).toBe('invalid');
    }
    const huge = await call('POST', '/v1/signup', { body: signupBody({ padding: 'x'.repeat(70 * 1024) }) });
    expect(huge.status).toBe(413);
    expect(huge.body.error).toBe('too_large');
  });

  it('accepts the weakest and strongest allowed kdf', async () => {
    for (const kdf of [
      { alg: 'argon2id', m: 19456, t: 2, p: 1 },
      { alg: 'argon2id', m: 1048576, t: 10, p: 4 }
    ]) {
      const res = await call('POST', '/v1/signup', { body: signupBody({ kdf }) });
      expect(res.status).toBe(201);
      expect(res.body.account.kdf).toEqual(kdf);
    }
  });

  it('trims device names to 100 characters', async () => {
    const res = await call('POST', '/v1/signup', {
      body: signupBody({ device: { name: `   ${'é'.repeat(99)}😀😀😀   `, platform: '  windows  ' } })
    });
    expect(res.status).toBe(201);
    const [d] = res.body.account.devices;
    expect(Array.from(d.name)).toHaveLength(100);
    expect(d.name.endsWith('😀')).toBe(true);
    expect(d.platform).toBe('windows');
  });
});

describe('prelogin', () => {
  it('returns the real kdf and salt for an account', async () => {
    const kdf = { alg: 'argon2id', m: 131072, t: 4, p: 2 };
    const person = await signup({ kdf });
    const res = await call('POST', '/v1/prelogin', { body: { email: person.email.toUpperCase() } });
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ kdf, salt: person.salt });
  });

  it('returns the default kdf and a stable salt made from the pepper for an unknown address', async () => {
    const email = newEmail();
    const first = await call('POST', '/v1/prelogin', { body: { email } });
    const again = await call('POST', '/v1/prelogin', { body: { email: ` ${email.toUpperCase()} ` } });
    const other = await call('POST', '/v1/prelogin', { body: { email: newEmail() } });
    expect(first.status).toBe(200);
    expect(first.body.kdf).toEqual({ alg: 'argon2id', m: 65536, t: 3, p: 1 });
    expect(first.body.salt).toBe(toBase64url((await hmac(env.PEPPER, `salt\n${email}`)).slice(0, 16)));
    expect(again.body).toEqual(first.body);
    expect(other.body.salt).not.toBe(first.body.salt);
  });

  it('leaves nothing behind for an unknown address', async () => {
    const email = newEmail();
    await call('POST', '/v1/prelogin', { body: { email } });
    await call('POST', '/v1/login', { body: { email, authKey: key(), device: device() } });
    await call('POST', '/v1/recover/start', { body: { email, recoveryAuth: key() } });
    expect(await tables(email)).toEqual([]);
  });

  it('checks the address', async () => {
    const res = await call('POST', '/v1/prelogin', { body: { email: 'nobody' } });
    expect(res.status).toBe(400);
    expect(res.body.error).toBe('invalid');
  });
});

describe('login', () => {
  it('signs in with the right key and adds a device', async () => {
    const person = await signup();
    const res = await call('POST', '/v1/login', {
      body: { email: ` ${person.email.toUpperCase()}`, authKey: person.authKey, device: device('Fold', 'android') }
    });
    expect(res.status).toBe(200);
    expect(res.body.wrappedKey).toBe(person.wrappedKey);
    expect(res.body.token).not.toBe(person.token);
    const { devices } = res.body.account;
    expect(devices.map((d) => [d.name, d.platform, d.current])).toEqual([
      ['Test desktop', 'linux', false],
      ['Fold', 'android', true]
    ]);
  });

  it('answers 401 for a wrong key and for an unknown address alike', async () => {
    const person = await signup();
    const wrong = await wrongLogin(person);
    const unknown = await call('POST', '/v1/login', { body: { email: newEmail(), authKey: key(), device: device() } });
    expect(wrong.status).toBe(401);
    expect(unknown.status).toBe(401);
    expect(wrong.body).toEqual(unknown.body);
    expect(wrong.body.error).toBe('unauthorized');
  });

  it('checks its input', async () => {
    const person = await signup();
    for (const body of [
      { email: person.email, authKey: person.authKey },
      { email: person.email, authKey: 'short', device: device() },
      { email: 'nope', authKey: person.authKey, device: device() }
    ]) {
      const res = await call('POST', '/v1/login', { body });
      expect(res.status).toBe(400);
    }
  });
});

describe('the lock after failed attempts', () => {
  it('locks the account on the tenth failure within 15 minutes, for 15 minutes', async () => {
    const person = await signup();
    for (let i = 0; i < 9; i++) expect((await wrongLogin(person)).status).toBe(401);
    const tenth = await wrongLogin(person);
    expect(tenth.status).toBe(429);
    expect(tenth.body.error).toBe('locked');
    expect(Number(tenth.headers.get('retry-after'))).toBe(15 * 60);

    // Even the right key is refused while locked.
    expect((await login(person)).status).toBe(429);
    expect(
      (await call('POST', '/v1/recover/start', { body: { email: person.email, recoveryAuth: person.recoveryAuth } }))
        .status
    ).toBe(429);

    later(14 * MINUTE);
    const stillLocked = await login(person);
    expect(stillLocked.status).toBe(429);
    expect(Number(stillLocked.headers.get('retry-after'))).toBeLessThanOrEqual(60);

    later(2 * MINUTE);
    expect((await login(person)).status).toBe(200);
  });

  it('forgets failures older than 15 minutes', async () => {
    const person = await signup();
    for (let i = 0; i < 9; i++) await wrongLogin(person);
    later(16 * MINUTE);
    for (let i = 0; i < 9; i++) expect((await wrongLogin(person)).status).toBe(401);
    expect((await login(person)).status).toBe(200);
  });

  it('starts counting again after a success', async () => {
    const person = await signup();
    for (let i = 0; i < 9; i++) await wrongLogin(person);
    expect((await login(person)).status).toBe(200);
    for (let i = 0; i < 9; i++) expect((await wrongLogin(person)).status).toBe(401);
    expect((await login(person)).status).toBe(200);
  });

  it('counts sign-ins and recovery attempts together', async () => {
    const person = await signup();
    for (let i = 0; i < 5; i++) await wrongLogin(person);
    for (let i = 0; i < 4; i++) {
      const res = await call('POST', '/v1/recover/start', { body: { email: person.email, recoveryAuth: key() } });
      expect(res.status).toBe(401);
    }
    const tenth = await call('POST', '/v1/recover/finish', {
      body: {
        email: person.email,
        recoveryAuth: key(),
        kdf: KDF,
        salt: salt(),
        authKey: key(),
        wrappedKey: sealed(),
        device: device()
      }
    });
    expect(tenth.status).toBe(429);
    expect((await login(person)).status).toBe(429);
  });

  it("doesn't stop a signed-in device from changing the password, the recovery key or deleting the account", async () => {
    // Someone who only knows the address locks sign-ins.
    const person = await signup();
    for (let i = 0; i < 10; i++) await wrongLogin(person);
    expect((await login(person)).status).toBe(429);

    // A device that is signed in checks the password as usual: a wrong one is 403, the right one goes through.
    expect((await recovery(person.token, key())).status).toBe(403);
    expect((await check(person.token, person.authKey)).status).toBe(204);
    const next = { recoveryAuth: key(), recoveryWrappedKey: sealed() };
    const replaced = await call('POST', '/v1/recovery', { token: person.token, body: { authKey: person.authKey, ...next } });
    expect(replaced.status).toBe(204);
    const newAuthKey = key();
    const changed = await call('POST', '/v1/password', {
      token: person.token,
      body: { authKey: person.authKey, kdf: KDF, salt: salt(), newAuthKey, wrappedKey: sealed() }
    });
    expect(changed.status).toBe(204);

    // Neither moved the account's lock: sign-ins wait for it to end, also with the new password.
    expect((await login({ email: person.email, authKey: newAuthKey })).status).toBe(429);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: { authKey: newAuthKey } })).status).toBe(204);
    expect(await tables(person.email)).toEqual([]);
  });

  it("locks one session's password checks after ten wrong passwords, for 15 minutes", async () => {
    const person = await signup();
    const phone = (await login(person, 'Phone')).body.token;
    for (let i = 0; i < 9; i++) {
      const res = await call('DELETE', '/v1/account', { token: person.token, body: { authKey: key() } });
      expect(res.status).toBe(403);
    }
    const tenth = await recovery(person.token, key());
    expect(tenth.status).toBe(429);
    expect(tenth.body.error).toBe('locked');
    expect(Number(tenth.headers.get('retry-after'))).toBe(15 * 60);

    // Even the right password is refused on this session while it is locked.
    expect((await recovery(person.token, person.authKey)).status).toBe(429);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } })).status).toBe(
      429
    );
    // Nothing else is locked: the session works for calls without the password, and other devices aren't affected.
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(200);
    expect((await recovery(phone, person.authKey)).status).toBe(204);

    later(14 * MINUTE);
    const stillLocked = await recovery(person.token, person.authKey);
    expect(stillLocked.status).toBe(429);
    expect(Number(stillLocked.headers.get('retry-after'))).toBeLessThanOrEqual(60);

    later(2 * MINUTE);
    expect((await recovery(person.token, person.authKey)).status).toBe(204);
  });

  it("starts a session's count again after the right password, and forgets its failures after 15 minutes", async () => {
    const person = await signup();
    for (let i = 0; i < 9; i++) await recovery(person.token, key());
    expect((await recovery(person.token, person.authKey)).status).toBe(204);
    for (let i = 0; i < 9; i++) expect((await recovery(person.token, key())).status).toBe(403);
    later(16 * MINUTE);
    for (let i = 0; i < 9; i++) expect((await recovery(person.token, key())).status).toBe(403);
    expect((await recovery(person.token, person.authKey)).status).toBe(204);
  });

  it("doesn't count a session's wrong passwords toward the account's lock", async () => {
    const person = await signup();
    for (let i = 0; i < 9; i++) await wrongLogin(person);
    expect((await recovery(person.token, key())).status).toBe(403);
    expect((await login(person)).status).toBe(200);
  });

  it("keeps a session's count when the account's owner signs in on another device", async () => {
    // A stolen session guesses nine times; the owner signs in elsewhere meanwhile. The thief's next guess is the tenth.
    const person = await signup();
    for (let i = 0; i < 9; i++) expect((await recovery(person.token, key())).status).toBe(403);
    expect((await login(person, 'Phone')).status).toBe(200);
    expect((await wrongLogin(person)).status).toBe(401);
    expect((await login(person, 'Tablet')).status).toBe(200);
    const tenth = await recovery(person.token, key());
    expect(tenth.status).toBe(429);
    expect(tenth.body.error).toBe('locked');
  });

  it("keeps a session's lock when the account's owner signs in on another device", async () => {
    const person = await signup();
    const phone = (await login(person, 'Phone')).body.token;
    for (let i = 0; i < 10; i++) await recovery(person.token, key());
    expect((await recovery(person.token, person.authKey)).status).toBe(429);

    later(5 * MINUTE);
    const tablet = await login(person, 'Tablet');
    expect(tablet.status).toBe(200);
    const after = await recovery(person.token, person.authKey);
    expect(after.status).toBe(429);
    expect(Number(after.headers.get('retry-after'))).toBeGreaterThan(9 * 60);
    expect(Number(after.headers.get('retry-after'))).toBeLessThanOrEqual(10 * 60);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } })).status).toBe(
      429
    );
    // The device that signed in, and the one that was signed in already, have their own counts.
    expect((await recovery(phone, person.authKey)).status).toBe(204);
    expect((await recovery(tablet.body.token, person.authKey)).status).toBe(204);
    expect(await tables(person.email)).toContain('account');

    later(11 * MINUTE);
    expect((await recovery(person.token, person.authKey)).status).toBe(204);
  });

  it('adds a count to the sessions of an account made before sessions had one', async () => {
    const person = await signup();
    // The sessions table as servers before this one made it.
    await runInDurableObject(await accountStub(person.email), (instance, state) => {
      for (const column of ['failures', 'failedAt', 'lockedUntil']) {
        state.storage.sql.exec(`ALTER TABLE sessions DROP COLUMN ${column}`);
      }
      instance.migrate();
      instance.migrate();
      expect(state.storage.sql.exec('SELECT failures, failedAt, lockedUntil FROM sessions').one()).toEqual({
        failures: 0,
        failedAt: 0,
        lockedUntil: 0
      });
    });
    for (let i = 0; i < 9; i++) expect((await recovery(person.token, key())).status).toBe(403);
    expect((await recovery(person.token, key())).status).toBe(429);
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(200);
  });
});

describe('checking the password on a session', () => {
  // Everything about the account that a check must leave alone, with the bytes made comparable.
  async function snapshot(email) {
    return runInDurableObject(await accountStub(email), (instance, state) => {
      const plain = (row) =>
        Object.fromEntries(
          Object.entries(row).map(([name, value]) => [name, value instanceof ArrayBuffer ? toBase64url(new Uint8Array(value)) : value])
        );
      return {
        account: plain(state.storage.sql.exec('SELECT * FROM account').one()),
        sessions: state.storage.sql.exec('SELECT * FROM sessions ORDER BY id').toArray().map(plain)
      };
    });
  }

  it('answers 204 for the right password and changes nothing', async () => {
    const person = await signup();
    const before = await snapshot(person.email);
    const res = await check(person.token, person.authKey);
    expect(res.status).toBe(204);
    expect(res.text).toBe('');
    expect(await snapshot(person.email)).toEqual(before);
  });

  it('counts a wrong password toward the same lock as deleting the account', async () => {
    const person = await signup();
    const wrong = await check(person.token, key());
    expect(wrong.status).toBe(403);
    expect(wrong.body.error).toBe('wrong_password');
    // Eight more on other password calls, and the tenth on the check locks the session.
    for (let i = 0; i < 8; i++) expect((await recovery(person.token, key())).status).toBe(403);
    const tenth = await check(person.token, key());
    expect(tenth.status).toBe(429);
    expect(tenth.body.error).toBe('locked');

    // Ten wrong passwords for new recovery keys lock the check the same way. The check and the deletion refuse the
    // right password alike, so a device that stops at the check deletes nothing.
    const other = await signup();
    for (let i = 0; i < 10; i++) await recovery(other.token, key());
    const refused = await check(other.token, other.authKey);
    expect(refused.status).toBe(429);
    expect(Number(refused.headers.get('retry-after'))).toBe(15 * 60);
    expect((await call('DELETE', '/v1/account', { token: other.token, body: { authKey: other.authKey } })).status).toBe(429);
    expect(await tables(other.email)).toContain('account');

    later(16 * MINUTE);
    expect((await check(other.token, other.authKey)).status).toBe(204);
    expect((await call('DELETE', '/v1/account', { token: other.token, body: { authKey: other.authKey } })).status).toBe(204);
    expect(await tables(other.email)).toEqual([]);
  });

  it('answers 401 without a live session and 400 without a key', async () => {
    const person = await signup();
    expect((await call('POST', '/v1/password/check', { body: { authKey: person.authKey } })).status).toBe(401);
    expect((await call('POST', '/v1/password/check', { token: person.token, body: { authKey: 'short' } })).status).toBe(400);
    expect((await call('POST', '/v1/password/check', { token: person.token, body: {} })).status).toBe(400);
    expect((await call('POST', '/v1/logout', { token: person.token })).status).toBe(204);
    expect((await check(person.token, person.authKey)).status).toBe(401);
  });
});

describe('sessions', () => {
  it('signs out with logout, leaving other devices signed in', async () => {
    const person = await signup();
    const other = (await login(person)).body.token;
    const res = await call('POST', '/v1/logout', { token: person.token });
    expect(res.status).toBe(204);
    expect(res.text).toBe('');
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(401);
    expect((await call('POST', '/v1/logout', { token: person.token })).status).toBe(401);
    const account = await call('GET', '/v1/account', { token: other });
    expect(account.status).toBe(200);
    expect(account.body.devices).toHaveLength(1);
  });

  it('answers 401 without a valid token', async () => {
    const person = await signup();
    const [name] = person.token.split('.');
    for (const headers of [
      {},
      { authorization: person.token },
      { authorization: `Bearer ${person.token}x` },
      { authorization: `Bearer ${name}.${toBase64url(new Uint8Array(32))}` },
      { authorization: `Bearer ${toBase64url(new Uint8Array(32))}.${person.token.split('.')[1]}` },
      { authorization: 'Bearer nonsense' }
    ]) {
      const res = await call('GET', '/v1/account', { headers });
      expect(res.status).toBe(401);
      expect(res.body.error).toBe('unauthorized');
    }
  });

  it('accepts the bearer scheme in any case', async () => {
    const person = await signup();
    const res = await call('GET', '/v1/account', { headers: { authorization: `bearer ${person.token}` } });
    expect(res.status).toBe(200);
  });

  it('keeps at most 100 sessions, dropping the one seen longest ago', async () => {
    const person = await signup();
    for (let i = 0; i < 100; i++) {
      later(MINUTE);
      expect((await login(person, `Device ${i}`)).status).toBe(200);
    }
    const res = await call('GET', '/v1/account', { token: (await login(person, 'Last')).body.token });
    expect(res.body.devices).toHaveLength(100);
    expect(res.body.devices.map((d) => d.name)).not.toContain('Test desktop');
    expect(res.body.devices.map((d) => d.name)).not.toContain('Device 0');
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(401);
  });

  it('keeps only a hash of each token', async () => {
    const person = await signup();
    const stored = await runInDurableObject(await accountStub(person.email), (instance, state) =>
      state.storage.sql.exec('SELECT * FROM sessions').toArray()
    );
    expect(stored).toHaveLength(1);
    expect(Object.keys(stored[0]).sort()).toEqual([
      'createdAt',
      'failedAt',
      'failures',
      'id',
      'lastSeenAt',
      'lockedUntil',
      'name',
      'platform',
      'tokenHash'
    ]);
    const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(person.token)));
    expect(new Uint8Array(stored[0].tokenHash)).toEqual(hash);
  });

  it('updates when a device was last seen at most once an hour', async () => {
    const person = await signup();
    const seen = async () => (await call('GET', '/v1/account', { token: person.token })).body.devices[0].lastSeenAt;
    const first = await seen();
    later(30 * MINUTE);
    expect(await seen()).toBe(first);
    later(31 * MINUTE);
    const second = await seen();
    expect(second).toBeGreaterThanOrEqual(first + 60 * MINUTE);
    later(10 * MINUTE);
    expect(await seen()).toBe(second);
  });
});

describe('GET /v1/account', () => {
  it('reports the account, its usage, limits and devices', async () => {
    const person = await signup();
    await login(person, 'Phone');
    const data = sealed(100);
    expect(
      (
        await push(person.token, [
          { id: recordId(), data },
          { id: recordId(), data: sealed(40) }
        ])
      ).status
    ).toBe(200);
    const res = await call('GET', '/v1/account', { token: person.token, vars: { MAX_RECORDS: 500 } });
    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({
      email: person.email,
      createdAt: person.account.createdAt,
      kdf: KDF,
      usage: { records: 2, bytes: 140 },
      limits: { records: 500, bytes: 104857600, recordBytes: 16384, batch: 500 }
    });
    expect(res.body.devices.map((d) => [d.name, d.current])).toEqual([
      ['Test desktop', true],
      ['Phone', false]
    ]);
    for (const d of res.body.devices)
      expect(Object.keys(d).sort()).toEqual(['createdAt', 'current', 'id', 'lastSeenAt', 'name', 'platform']);
  });
});

describe('DELETE /v1/devices/:id', () => {
  it('signs another device out', async () => {
    const person = await signup();
    const phone = (await login(person, 'Phone')).body;
    const phoneId = phone.account.devices.find((d) => d.current).id;
    const res = await call('DELETE', `/v1/devices/${phoneId}`, { token: person.token });
    expect(res.status).toBe(204);
    expect((await call('GET', '/v1/account', { token: phone.token })).status).toBe(401);
    const left = await call('GET', '/v1/account', { token: person.token });
    expect(left.body.devices.map((d) => d.name)).toEqual(['Test desktop']);
  });

  it('can sign out the device asking', async () => {
    const person = await signup();
    const id = person.account.devices[0].id;
    expect((await call('DELETE', `/v1/devices/${id}`, { token: person.token })).status).toBe(204);
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(401);
  });

  it('answers 404 for an unknown device and 400 for a malformed id', async () => {
    const person = await signup();
    const other = await signup();
    const unknown = await call('DELETE', `/v1/devices/${other.account.devices[0].id}`, { token: person.token });
    expect(unknown.status).toBe(404);
    expect(unknown.body.error).toBe('not_found');
    expect((await call('GET', '/v1/account', { token: other.token })).status).toBe(200);
    const malformed = await call('DELETE', '/v1/devices/not-an-id', { token: person.token });
    expect(malformed.status).toBe(400);
    expect((await call('DELETE', `/v1/devices/${person.account.devices[0].id}`)).status).toBe(401);
  });
});

describe('POST /v1/password', () => {
  function change(person, overrides = {}) {
    return call('POST', '/v1/password', {
      token: person.token,
      body: { authKey: person.authKey, kdf: KDF, salt: salt(), newAuthKey: key(), wrappedKey: sealed(), ...overrides }
    });
  }

  it('changes the password and signs out every other device', async () => {
    const person = await signup();
    const phone = (await login(person, 'Phone')).body.token;
    const kdf = { alg: 'argon2id', m: 262144, t: 3, p: 1 };
    const next = { salt: salt(), newAuthKey: key(), wrappedKey: sealed() };
    const res = await change(person, { kdf, ...next });
    expect(res.status).toBe(204);

    expect((await call('GET', '/v1/account', { token: phone })).status).toBe(401);
    const mine = await call('GET', '/v1/account', { token: person.token });
    expect(mine.status).toBe(200);
    expect(mine.body.devices).toHaveLength(1);
    expect(mine.body.kdf).toEqual(kdf);

    expect((await login(person)).status).toBe(401);
    const fresh = await login({ email: person.email, authKey: next.newAuthKey });
    expect(fresh.status).toBe(200);
    expect(fresh.body.wrappedKey).toBe(next.wrappedKey);
    const pre = await call('POST', '/v1/prelogin', { body: { email: person.email } });
    expect(pre.body).toEqual({ kdf, salt: next.salt });
  });

  it('answers 403 wrong_password for a wrong current password and changes nothing', async () => {
    const person = await signup();
    const phone = (await login(person, 'Phone')).body.token;
    const res = await change(person, { authKey: key() });
    expect(res.status).toBe(403);
    expect(res.body.error).toBe('wrong_password');
    expect((await call('GET', '/v1/account', { token: phone })).status).toBe(200);
    expect((await login(person)).status).toBe(200);
  });

  it('needs a session and valid fields', async () => {
    const person = await signup();
    expect((await call('POST', '/v1/password', { body: { authKey: person.authKey } })).status).toBe(401);
    expect((await change(person, { kdf: { ...KDF, m: 1024 } })).status).toBe(400);
    expect((await change(person, { newAuthKey: undefined })).status).toBe(400);
    expect((await change(person, { wrappedKey: notSealed() })).status).toBe(400);
  });
});

describe('POST /v1/recovery', () => {
  it('replaces the recovery key', async () => {
    const person = await signup();
    const next = { recoveryAuth: key(), recoveryWrappedKey: sealed() };
    const res = await call('POST', '/v1/recovery', { token: person.token, body: { authKey: person.authKey, ...next } });
    expect(res.status).toBe(204);
    const old = await call('POST', '/v1/recover/start', {
      body: { email: person.email, recoveryAuth: person.recoveryAuth }
    });
    expect(old.status).toBe(401);
    const fresh = await call('POST', '/v1/recover/start', {
      body: { email: person.email, recoveryAuth: next.recoveryAuth }
    });
    expect(fresh.status).toBe(200);
    expect(fresh.body.recoveryWrappedKey).toBe(next.recoveryWrappedKey);
    // Other devices stay signed in.
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(200);
  });

  it('answers 403 wrong_password for a wrong password and keeps the old key', async () => {
    const person = await signup();
    const res = await call('POST', '/v1/recovery', {
      token: person.token,
      body: { authKey: key(), recoveryAuth: key(), recoveryWrappedKey: sealed() }
    });
    expect(res.status).toBe(403);
    expect(res.body.error).toBe('wrong_password');
    const old = await call('POST', '/v1/recover/start', {
      body: { email: person.email, recoveryAuth: person.recoveryAuth }
    });
    expect(old.status).toBe(200);
  });

  it('needs a session and valid fields', async () => {
    const person = await signup();
    const body = { authKey: person.authKey, recoveryAuth: key(), recoveryWrappedKey: sealed() };
    expect((await call('POST', '/v1/recovery', { body })).status).toBe(401);
    expect(
      (await call('POST', '/v1/recovery', { token: person.token, body: { ...body, recoveryAuth: salt() } })).status
    ).toBe(400);
    expect(
      (await call('POST', '/v1/recovery', { token: person.token, body: { ...body, recoveryWrappedKey: sealed(20) } }))
        .status
    ).toBe(400);
  });
});

describe('recovering a forgotten password', () => {
  it('hands out the recovery-wrapped key and the kdf for the right recovery key', async () => {
    const kdf = { alg: 'argon2id', m: 65536, t: 4, p: 1 };
    const person = await signup({ kdf });
    const res = await call('POST', '/v1/recover/start', {
      body: { email: person.email, recoveryAuth: person.recoveryAuth }
    });
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ recoveryWrappedKey: person.recoveryWrappedKey, kdf });
  });

  it('answers 401 for a wrong recovery key or an unknown address', async () => {
    const person = await signup();
    const wrong = await call('POST', '/v1/recover/start', { body: { email: person.email, recoveryAuth: key() } });
    const unknown = await call('POST', '/v1/recover/start', { body: { email: newEmail(), recoveryAuth: key() } });
    expect(wrong.status).toBe(401);
    expect(unknown.status).toBe(401);
    expect(wrong.body.error).toBe('unauthorized');
  });

  it('sets a new password, signs out every device and starts a new session', async () => {
    const person = await signup();
    const phone = (await login(person, 'Phone')).body.token;
    const next = { kdf: { alg: 'argon2id', m: 65536, t: 3, p: 2 }, salt: salt(), authKey: key(), wrappedKey: sealed() };
    const res = await call('POST', '/v1/recover/finish', {
      body: {
        email: person.email,
        recoveryAuth: person.recoveryAuth,
        ...next,
        device: device('Recovered laptop', 'darwin')
      }
    });
    expect(res.status).toBe(200);
    expect(res.body.token).toMatch(/^[A-Za-z0-9_-]{43}\.[A-Za-z0-9_-]{43}$/);
    expect(res.body.account.kdf).toEqual(next.kdf);
    expect(res.body.account.devices.map((d) => [d.name, d.current])).toEqual([['Recovered laptop', true]]);

    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(401);
    expect((await call('GET', '/v1/account', { token: phone })).status).toBe(401);
    expect((await call('GET', '/v1/account', { token: res.body.token })).status).toBe(200);
    expect((await login(person)).status).toBe(401);
    const fresh = await login({ email: person.email, authKey: next.authKey });
    expect(fresh.status).toBe(200);
    expect(fresh.body.wrappedKey).toBe(next.wrappedKey);
    // The recovery key itself is unchanged and still works.
    const again = await call('POST', '/v1/recover/start', {
      body: { email: person.email, recoveryAuth: person.recoveryAuth }
    });
    expect(again.status).toBe(200);
  });

  it('changes nothing for a wrong recovery key', async () => {
    const person = await signup();
    const res = await call('POST', '/v1/recover/finish', {
      body: {
        email: person.email,
        recoveryAuth: key(),
        kdf: KDF,
        salt: salt(),
        authKey: key(),
        wrappedKey: sealed(),
        device: device()
      }
    });
    expect(res.status).toBe(401);
    expect(res.body.error).toBe('unauthorized');
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(200);
    expect((await login(person)).status).toBe(200);
  });

  it('checks its input', async () => {
    const person = await signup();
    const base = {
      email: person.email,
      recoveryAuth: person.recoveryAuth,
      kdf: KDF,
      salt: salt(),
      authKey: key(),
      wrappedKey: sealed(),
      device: device()
    };
    for (const change of [{ kdf: undefined }, { salt: key() }, { device: undefined }, { recoveryAuth: undefined }]) {
      const res = await call('POST', '/v1/recover/finish', { body: { ...base, ...change } });
      expect(res.status).toBe(400);
    }
    expect((await call('POST', '/v1/recover/start', { body: { email: person.email } })).status).toBe(400);
  });
});

describe('DELETE /v1/account', () => {
  it('answers 403 wrong_password for a wrong password and keeps everything', async () => {
    const person = await signup();
    await push(person.token, [{ id: recordId(), data: sealed() }]);
    const res = await call('DELETE', '/v1/account', { token: person.token, body: { authKey: key() } });
    expect(res.status).toBe(403);
    expect(res.body.error).toBe('wrong_password');
    const account = await call('GET', '/v1/account', { token: person.token });
    expect(account.body.usage.records).toBe(1);
  });

  it('deletes the account and every record at once', async () => {
    const person = await signup();
    const phone = (await login(person, 'Phone')).body.token;
    await push(person.token, [{ id: recordId(), data: sealed() }]);
    const res = await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } });
    expect(res.status).toBe(204);

    expect(await tables(person.email)).toEqual([]);
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(401);
    expect((await call('GET', '/v1/records', { token: phone })).status).toBe(401);
    expect((await login(person)).status).toBe(401);
    const pre = await call('POST', '/v1/prelogin', { body: { email: person.email } });
    expect(pre.body.salt).toBe(toBase64url((await hmac(env.PEPPER, `salt\n${person.email}`)).slice(0, 16)));

    // The address is free again, and a new account starts empty.
    const again = await signup({ email: person.email });
    expect(again.account.usage).toEqual({ records: 0, bytes: 0 });
    const records = await call('GET', '/v1/records', { token: again.token });
    expect(records.body).toEqual({ records: [], seq: 0, more: false });
  });

  it('needs a session and the password', async () => {
    const person = await signup();
    expect((await call('DELETE', '/v1/account', { body: { authKey: person.authKey } })).status).toBe(401);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: {} })).status).toBe(400);
    expect((await call('GET', '/v1/account', { token: person.token })).status).toBe(200);
  });
});

describe('the cap on accounts', () => {
  const registry = () => env.REGISTRY.get(env.REGISTRY.idFromName('registry'));
  const count = () => runInDurableObject(registry(), (instance) => instance.count());

  it('refuses a sign-up past MAX_ACCOUNTS, and a failed or deleted account frees its place', async () => {
    const vars = { SIGNUPS: 'open', MAX_ACCOUNTS: String((await count()) + 1) };
    const first = await signup({ vars });
    expect(first.token).toBeTruthy();
    const full = await call('POST', '/v1/signup', {
      body: { email: newEmail(), kdf: KDF, salt: salt(), authKey: key(), wrappedKey: sealed(), recoveryAuth: key(), recoveryWrappedKey: sealed(), device: device() },
      vars
    });
    expect(full.status).toBe(403);
    expect(full.body.error).toBe('signups_closed');

    // A sign-up for an address that has an account takes no place.
    const before = await count();
    const taken = await call('POST', '/v1/signup', {
      body: { email: first.email, kdf: KDF, salt: salt(), authKey: key(), wrappedKey: sealed(), recoveryAuth: key(), recoveryWrappedKey: sealed(), device: device() },
      vars: { SIGNUPS: 'open', MAX_ACCOUNTS: '0' }
    });
    expect(taken.status).toBe(409);
    expect(await count()).toBe(before);

    const deleted = await call('DELETE', '/v1/account', { token: first.token, body: { authKey: first.authKey }, vars });
    expect(deleted.status).toBe(204);
    expect(await count()).toBe(before - 1);
    expect((await signup({ vars })).token).toBeTruthy();
  });

  it('gives a place back once when the same account is deleted twice at once', async () => {
    const person = await signup();
    const before = await count();
    const remove = () => call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } });
    const results = await Promise.all([remove(), remove(), remove()]);
    expect(results.map((r) => r.status).sort()).toEqual([204, 401, 401]);
    expect(await count()).toBe(before - 1);
  });

  it('lets an invite code make one account, and gives the use back when the sign-up fails', async () => {
    const vars = { SIGNUPS: 'invite', INVITE_USES: '1', INVITE_CODES: `once-${crypto.randomUUID()}` };
    const body = (email) => ({ email, invite: vars.INVITE_CODES, kdf: KDF, salt: salt(), authKey: key(), wrappedKey: sealed(), recoveryAuth: key(), recoveryWrappedKey: sealed(), device: device() });
    const taken = (await signup()).email;
    // A sign-up that fails (the address has an account) doesn't use the code up.
    expect((await call('POST', '/v1/signup', { body: body(taken), vars })).status).toBe(409);
    const firstBody = body(newEmail());
    const first = await call('POST', '/v1/signup', { body: firstBody, vars });
    expect(first.status).toBe(201);
    const second = await call('POST', '/v1/signup', { body: body(newEmail()), vars });
    expect(second.status).toBe(403);
    expect(second.body.error).toBe('invite_required');
    // Deleting the account doesn't give the code back, or one code could make accounts one after another.
    const removed = await call('DELETE', '/v1/account', { token: first.body.token, body: { authKey: firstBody.authKey }, vars });
    expect(removed.status).toBe(204);
    expect((await call('POST', '/v1/signup', { body: body(newEmail()), vars })).status).toBe(403);
  });

  it('counts without a cap when MAX_ACCOUNTS is 0', async () => {
    const before = await count();
    await signup({ vars: { MAX_ACCOUNTS: '0' } });
    expect(await count()).toBe(before + 1);
  });
});
