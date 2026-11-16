import { runInDurableObject } from 'cloudflare:test';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fromBase64url, toBase64url } from '../src/bytes.js';
import {
  accountName,
  accountStub,
  call,
  key as randomKey,
  login,
  sealed,
  signup
} from './helpers.js';
import { DAY, HOUR, bookingStub, host, hostKey, hours, meetingType, newHandle, page, publish, slotAhead } from './booking-helpers.js';

afterEach(() => vi.useRealTimers());

const TIMES = { staleMs: 24 * HOUR, darkMs: 7 * DAY };

function later(ms) {
  vi.setSystemTime(Date.now() + ms);
}

async function tables(stub) {
  return runInDurableObject(stub, (instance, state) =>
    state.storage.sql
      .exec("SELECT name FROM sqlite_master WHERE type = 'table'")
      .toArray()
      .map((r) => r.name)
  );
}

// A booking made straight on the object, as the public routes make them.
function bookDirect(handle, start, overrides = {}) {
  return bookingStub(handle).book(
    {
      type: '30min',
      start,
      name: 'Jane Doe',
      email: `jane-${Math.random().toString(36).slice(2)}@example.com`,
      notes: '',
      timeZone: 'Europe/Amsterdam',
      locale: 'en',
      via: '',
      channel: 'api',
      idempotencyKey: null,
      network: `10.${Math.floor(Math.random() * 250)}.${Math.floor(Math.random() * 250)}.0/24`,
      uidHost: 'accounts.example',
      ...overrides
    },
    TIMES
  );
}

const tenAm = (days) => slotAhead(days);

describe('the account side of booking pages', () => {
  it('says who is asking, with the handle, the session and the device names', async () => {
    const person = await signup();
    const stub = await accountStub(person.email);
    const auth = await stub.bookingAuth(person.token);
    expect(auth).toMatchObject({
      status: 200,
      name: await accountName(person.email),
      handle: null,
      deviceId: person.account.devices[0].id,
      changes: 0
    });
    expect(auth.devices).toEqual({ [person.account.devices[0].id]: 'Test desktop' });
    const dead = await stub.bookingAuth(`${await accountName(person.email)}.${'x'.repeat(43)}`);
    expect(dead.status).toBe(401);
  });

  it('allows one handle, changed at most 3 times in 30 days', async () => {
    const person = await signup();
    const stub = await accountStub(person.email);
    expect((await stub.bookingSetHandle(person.token, 'first')).body).toEqual({ handle: 'first', changes: 0 });
    expect((await stub.bookingSetHandle(person.token, 'first')).body).toEqual({ handle: 'first', changes: 0 });
    for (const [i, h] of ['second', 'third', 'fourth'].entries()) {
      expect((await stub.bookingSetHandle(person.token, h)).body).toEqual({ handle: h, changes: i + 1 });
    }
    const refused = await stub.bookingSetHandle(person.token, 'fifth');
    expect(refused.status).toBe(409);
    expect(refused.body.error).toBe('handle_changes');
    expect((await stub.bookingAuth(person.token)).handle).toBe('fourth');
    later(30 * DAY + 1000);
    expect((await stub.bookingSetHandle(person.token, 'fifth')).body).toEqual({ handle: 'fifth', changes: 1 });
    // Giving the handle up isn't a change.
    expect((await stub.bookingSetHandle(person.token, null)).body).toEqual({ handle: null, changes: 1 });
  });

  it('hands the handle to the Worker when the account is deleted, and nothing else', async () => {
    const person = await signup();
    const stub = await accountStub(person.email);
    await stub.bookingSetHandle(person.token, 'leaving');
    const result = await stub.deleteAccount(person.token, { authKey: fromBase64url(person.authKey) });
    expect(result).toMatchObject({ status: 204, handle: 'leaving' });
    expect(await stub.exists()).toBe(false);
  });

  it('adds the booking table to accounts made before it', async () => {
    const person = await signup();
    const stub = await accountStub(person.email);
    await runInDurableObject(stub, (instance, state) => {
      state.storage.sql.exec('DROP TABLE booking');
      instance.migrate();
    });
    expect((await stub.bookingAuth(person.token)).handle).toBe(null);
    expect((await stub.bookingSetHandle(person.token, 'later')).status).toBe(200);
  });

  it('answers exists() without leaving anything in an empty object', async () => {
    const person = await signup();
    expect(await (await accountStub(person.email)).exists()).toBe(true);
    const nobody = await accountStub('nobody-here@example.com');
    expect(await nobody.exists()).toBe(false);
    expect(await tables(nobody)).toEqual([]);
  });
});

describe('GET /v1/booking', () => {
  it('describes an account without a page', async () => {
    const person = await signup();
    const res = await call('GET', '/v1/booking', { token: person.token });
    expect(res.status).toBe(200);
    expect(res.body).toEqual({
      handle: null,
      url: null,
      base: 'https://accounts.example/meet',
      enabled: false,
      device: null,
      publishedAt: null,
      keyId: null,
      seq: 0,
      pending: 0,
      upcoming: 0,
      today: null,
      handleChanges: { used: 0, max: 3 }
    });
    expect((await call('GET', '/v1/booking/bookings', { token: person.token })).body).toEqual({ bookings: [], seq: 0, more: false });
    const action = await call('POST', `/v1/booking/bookings/${'A'.repeat(22)}`, { token: person.token, body: { action: 'confirm', version: 1 } });
    expect(action.status).toBe(404);
  });

  it('needs a session', async () => {
    expect((await call('GET', '/v1/booking')).status).toBe(401);
    const res = await call('GET', '/v1/booking', { token: `${'a'.repeat(43)}.${'b'.repeat(43)}` });
    expect(res.status).toBe(401);
    expect(res.body.error).toBe('unauthorized');
  });
});

describe('PUT /v1/booking', () => {
  it('claims the handle, publishes and reports the page', async () => {
    const before = Date.now();
    const { person, key, handle, view } = await host(signup);
    expect(view).toEqual({
      handle,
      url: `https://accounts.example/meet/${handle}`,
      base: 'https://accounts.example/meet',
      enabled: true,
      device: { id: person.account.devices[0].id, name: 'Test desktop', current: true },
      publishedAt: expect.any(Number),
      keyId: key.keyId,
      seq: 0,
      pending: 0,
      upcoming: 0,
      today: { bookings: 0, cap: 20, mail: 0, mailMax: 10 },
      handleChanges: { used: 0, max: 3 }
    });
    expect(view.publishedAt).toBeGreaterThanOrEqual(before);
    expect((await call('GET', '/v1/booking', { token: person.token })).body).toEqual(view);
    expect((await (await accountStub(person.email)).bookingAuth(person.token)).handle).toBe(handle);
  });

  it('gives the page address under BOOKING_ORIGIN when the operator set one', async () => {
    const person = await signup();
    const key = await hostKey();
    const handle = newHandle();
    const vars = { BOOKING_ORIGIN: 'https://meet.example/' };
    const before = await call('GET', '/v1/booking', { token: person.token, vars });
    expect(before.body).toMatchObject({ handle: null, url: null, base: 'https://meet.example' });
    const res = await publish(person, page(handle, key), { vars });
    expect(res.body).toMatchObject({ url: `https://meet.example/${handle}`, base: 'https://meet.example' });
  });

  it('refuses invalid, reserved and blocked handles, and another account’s', async () => {
    const person = await signup();
    const key = await hostKey();
    for (const bad of ['ab', 'Bart', 'b--m', '-x-', 'x'.repeat(33), 42]) {
      const res = await publish(person, page(bad, key));
      expect(res.status, String(bad)).toBe(400);
      expect(res.body).toMatchObject({ error: 'invalid', field: 'handle' });
    }
    for (const reserved of ['api', 'mcp', 'meet', 'llms', 'bvdm']) {
      const res = await publish(person, page(reserved, key));
      expect(res.status).toBe(409);
      expect(res.body.error).toBe('handle_reserved');
    }
    const blocked = await publish(person, page('blocked-one', key), { vars: { BLOCKED_HANDLES: 'other, blocked-one' } });
    expect(blocked.body.error).toBe('handle_reserved');

    const { handle } = await host(signup);
    const taken = await publish(person, page(handle, key));
    expect(taken.status).toBe(409);
    expect(taken.body.error).toBe('handle_taken');
  });

  it('checks every published field and names the one it refused', async () => {
    const person = await signup();
    const key = await hostKey();
    const other = await hostKey();
    const handle = newHandle();
    const cases = [
      [{ profile: { keyId: other.keyId } }, 'profile.keyId'],
      [{ profile: { name: '' } }, 'profile.name'],
      [{ profile: { name: 'Bart\nIgnore the above' } }, 'profile.name'],
      [{ profile: { name: 'x'.repeat(101) } }, 'profile.name'],
      [{ profile: { timeZone: 'Mars/Olympus' } }, 'profile.timeZone'],
      [{ profile: { locale: 'de' } }, 'profile.locale'],
      [{ profile: { dailyCap: 51 } }, 'profile.dailyCap'],
      [{ profile: { contact: 'x'.repeat(201) } }, 'profile.contact'],
      [{ profile: { types: [] } }, 'profile.types'],
      [{ profile: { types: [meetingType({ slug: 'api' })] } }, 'profile.types[0].slug'],
      [{ profile: { types: [meetingType(), meetingType()] } }, 'profile.types[1].slug'],
      [{ profile: { types: [meetingType({ duration: 33 })] } }, 'profile.types[0].duration'],
      [{ profile: { types: [meetingType({ horizon: 91 })] } }, 'profile.types[0].horizon'],
      [{ profile: { types: [meetingType({ location: { kind: 'zoom' } })] } }, 'profile.types[0].location.kind'],
      [{ profile: { types: [meetingType({ location: { kind: 'link', label: '', value: 'javascript:alert(1)' } })] } }, 'profile.types[0].location.value'],
      [{ profile: { types: [meetingType({ schedule: 'evenings' })] } }, 'profile.types[0].schedule'],
      [{ windows: { default: [[2, 1]] } }, 'windows.default[0]'],
      [{ windows: { default: [...hours()].reverse() } }, 'windows.default[1]'],
      [{ windows: { default: [[Date.now() + 100 * DAY, Date.now() + 101 * DAY]] } }, 'windows.default[0]'],
      [{ windows: { other: [] } }, 'windows'],
      [{ busy: [[900000 * Math.ceil(Date.now() / 900000), 900000 * Math.ceil(Date.now() / 900000) + 1000]] }, 'busy[0]']
    ];
    for (const [overrides, field] of cases) {
      const body = page(handle, key, { windows: overrides.windows, busy: overrides.busy });
      if (overrides.profile) body.profile = { ...body.profile, ...overrides.profile };
      const res = await publish(person, body);
      expect(res.status, field).toBe(400);
      expect(res.body.field, JSON.stringify(overrides).slice(0, 80)).toBe(field);
    }
    const notOnCurve = toBase64url(Uint8Array.from([4, ...new Uint8Array(64).fill(7)]));
    const bad = page(handle, key);
    bad.profile.publicKey = notOnCurve;
    bad.profile.keyId = toBase64url(new Uint8Array(await crypto.subtle.digest('SHA-256', fromBase64url(notOnCurve))).slice(0, 8));
    const curve = await publish(person, bad);
    expect(curve.status).toBe(400);
    expect(curve.body.field).toBe('profile.publicKey');

    const huge = page(handle, key);
    huge.profile.intro = 'x'.repeat(1000);
    huge.profile.types = Array.from({ length: 10 }, (_, i) => meetingType({ slug: `t${i}`, description: 'y'.repeat(500) }));
    huge.profile.padding = 'z'.repeat(64 * 1024);
    expect((await publish(person, huge)).status).toBe(413);
    // Nothing was claimed by any of these.
    expect(await tables(bookingStub(handle))).toEqual([]);
  });

  it('keeps one booking device until another takes over', async () => {
    const { person, key, handle, body } = await host(signup);
    const second = (await login(person)).body;
    const refused = await publish({ token: second.token }, body);
    expect(refused.status).toBe(409);
    expect(refused.body.error).toBe('other_device');
    // Any session can read the page.
    const read = await call('GET', '/v1/booking', { token: second.token });
    expect(read.body.device).toEqual({ id: person.account.devices[0].id, name: 'Test desktop', current: false });

    const taken = await publish({ token: second.token }, { ...body, takeover: true });
    expect(taken.status).toBe(200);
    expect(taken.body.device).toMatchObject({ name: 'Second device', current: true });
    const old = await publish(person, page(handle, key));
    expect(old.body.error).toBe('other_device');
  });

  it('turns a page off and on again without losing the handle', async () => {
    const { person, key, handle } = await host(signup);
    const off = await publish(person, page(handle, key, { enabled: false }));
    expect(off.body.enabled).toBe(false);
    const on = await publish(person, page(handle, key));
    expect(on.body.enabled).toBe(true);
  });
});

describe('renaming', () => {
  it('moves the page to a new handle and keeps the old one reserved for this account', async () => {
    const { person, key, handle } = await host(signup);
    const next = newHandle('renamed');
    const res = await publish(person, page(next, key));
    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({ handle: next, enabled: true, handleChanges: { used: 1, max: 3 } });
    expect((await bookingStub(handle).view(TIMES)).state).toBe('moved');

    // Nobody else can take the old one; its owner can.
    const stranger = await signup();
    const taken = await publish(stranger, page(handle, await hostKey()));
    expect(taken.body.error).toBe('handle_reserved');
    const check = await call('GET', `/v1/booking/handles/${handle}`, { token: stranger.token });
    expect(check.body).toEqual({ handle, available: false, reason: 'reserved' });
    const back = await call('GET', `/v1/booking/handles/${handle}`, { token: person.token });
    expect(back.body).toEqual({ handle, available: true, reason: null });
    const again = await publish(person, page(handle, key));
    expect(again.status).toBe(200);
    expect(again.body.handleChanges.used).toBe(2);
    expect((await bookingStub(next).view(TIMES)).state).toBe('moved');
  });

  it('is refused while the page has bookings ahead, and leaves both handles as they were', async () => {
    const { person, key, handle } = await host(signup);
    expect((await bookDirect(handle, tenAm(5))).outcome).toBe('created');
    const next = newHandle('blocked');
    const res = await publish(person, page(next, key));
    expect(res.status).toBe(409);
    expect(res.body.error).toBe('has_bookings');
    expect((await bookingStub(handle).view(TIMES)).accepting).toBe(true);
    expect(await tables(bookingStub(next))).toEqual([]);
  });

  it("is refused while a guest's cancellation of a future booking waits for the booking device", async () => {
    const { person, key, handle } = await host(signup);
    expect((await bookDirect(handle, tenAm(5))).outcome).toBe('created');
    // The guest cancelled; the booking device hasn't handled that version yet.
    await runInDurableObject(bookingStub(handle), (instance, state) => state.storage.sql.exec("UPDATE bookings SET status = 'cancelled', cancelledBy = 'guest', version = version + 1"));
    const next = newHandle('waiting');
    const res = await publish(person, page(next, key));
    expect(res.status).toBe(409);
    expect(res.body.error).toBe('has_bookings');
    await runInDurableObject(bookingStub(handle), (instance, state) => state.storage.sql.exec('UPDATE bookings SET handledVersion = version'));
    expect((await publish(person, page(next, key))).status).toBe(200);
  });

  it('undoes the old handle’s retirement when the new one is taken', async () => {
    const { person, key, handle } = await host(signup);
    const other = await host(signup);
    const res = await publish(person, page(other.handle, key));
    expect(res.body.error).toBe('handle_taken');
    const view = await bookingStub(handle).view(TIMES);
    expect(view).toMatchObject({ state: 'page', accepting: true });
  });

  it('allows 3 changes in 30 days', async () => {
    const { person, key } = await host(signup);
    for (let i = 0; i < 3; i++) expect((await publish(person, page(newHandle('r'), key))).status).toBe(200);
    const fourth = newHandle('r');
    const res = await publish(person, page(fourth, key));
    expect(res.status).toBe(409);
    expect(res.body.error).toBe('handle_changes');
    expect(await tables(bookingStub(fourth))).toEqual([]);
  });
});

describe('GET /v1/booking/handles/<handle>', () => {
  it('says whether a handle can be taken, without leaving storage behind', async () => {
    const person = await signup();
    const { handle: theirs } = await host(signup);
    const free = newHandle('free');
    const answers = {
      [free]: { available: true, reason: null },
      ab: { available: false, reason: 'invalid' },
      Bart: { available: false, reason: 'invalid' },
      'b%C3%A4rt': { available: false, reason: 'invalid' },
      calendar: { available: false, reason: 'reserved' },
      [theirs]: { available: false, reason: 'taken' }
    };
    for (const [h, answer] of Object.entries(answers)) {
      const res = await call('GET', `/v1/booking/handles/${h}`, { token: person.token });
      expect(res.status, h).toBe(200);
      expect(res.body, h).toEqual({ handle: decodeURIComponent(h), ...answer });
    }
    expect(await tables(bookingStub(free))).toEqual([]);
    const blocked = await call('GET', `/v1/booking/handles/${free}`, { token: person.token, vars: { BLOCKED_HANDLES: free } });
    expect(blocked.body.reason).toBe('reserved');
  });

  it('calls the account’s own handle yours', async () => {
    const { person, handle } = await host(signup);
    const res = await call('GET', `/v1/booking/handles/${handle}`, { token: person.token });
    expect(res.body).toEqual({ handle, available: false, reason: 'yours' });
  });
});

describe('DELETE /v1/booking', () => {
  it('deletes the page and its bookings, keeping the handle reserved for the account', async () => {
    const { person, key, handle } = await host(signup);
    await bookDirect(handle, tenAm(3));
    const second = (await login(person)).body;
    // Any session of the account may turn the page off.
    const res = await call('DELETE', '/v1/booking', { token: second.token });
    expect(res.status).toBe(204);
    const view = await call('GET', '/v1/booking', { token: person.token });
    expect(view.body).toMatchObject({ handle, enabled: false, publishedAt: null, keyId: null, seq: 0, pending: 0, upcoming: 0, today: null });
    expect((await call('GET', '/v1/booking/bookings', { token: person.token })).body).toEqual({ bookings: [], seq: 0, more: false });
    const rows = await runInDurableObject(bookingStub(handle), (i, state) => state.storage.sql.exec('SELECT COUNT(*) AS n FROM bookings').one().n);
    expect(rows).toBe(0);

    const stranger = await signup();
    expect((await publish(stranger, page(handle, await hostKey()))).body.error).toBe('handle_reserved');
    const again = await publish(second, page(handle, key));
    expect(again.status).toBe(200);
    expect(again.body.device.current).toBe(true);
  });

  it('answers 204 for an account without a page', async () => {
    const person = await signup();
    expect((await call('DELETE', '/v1/booking', { token: person.token })).status).toBe(204);
  });

  // The booking device may publish from what it knew before another computer turned the page off. That would turn
  // the page on again against the user's choice, so it needs a takeover until it has read the page.
  it('refuses the booking device’s publish after another session turned the page off, until it reads the page', async () => {
    const { person, key, handle } = await host(signup);
    const second = (await login(person)).body;
    expect((await call('DELETE', '/v1/booking', { token: second.token })).status).toBe(204);

    const stale = await publish(person, page(handle, key));
    expect(stale.status).toBe(409);
    expect(stale.body.error).toBe('other_device');
    const renamed = newHandle('stale');
    const rename = await publish(person, page(renamed, key));
    expect(rename.body.error).toBe('other_device');
    expect(await tables(bookingStub(renamed))).toEqual([]);
    // Another session reading the page changes nothing for the booking device.
    expect((await call('GET', '/v1/booking', { token: second.token })).body).toMatchObject({ enabled: false, device: null });
    expect((await publish(person, page(handle, key))).body.error).toBe('other_device');

    // Once it has read the page off, turning it on is the user's choice again.
    expect((await call('GET', '/v1/booking', { token: person.token })).body).toMatchObject({ handle, enabled: false, device: null });
    const on = await publish(person, page(handle, key));
    expect(on.status).toBe(200);
    expect(on.body).toMatchObject({ enabled: true, device: { current: true } });
  });

  it('lets the booking device take over at once after another session turned the page off', async () => {
    const { person, key, handle } = await host(signup);
    const second = (await login(person)).body;
    await call('DELETE', '/v1/booking', { token: second.token });
    const taken = await publish(person, page(handle, key, { takeover: true }));
    expect(taken.status).toBe(200);
    expect(taken.body.device.current).toBe(true);
    // The other session didn't handle the page, so it isn't held back.
    await call('DELETE', '/v1/booking', { token: person.token });
    expect((await publish(second, page(handle, key))).status).toBe(200);
  });

  it('lets a device turn its own page off and on again, and a new session turn it on', async () => {
    const { person, key, handle } = await host(signup);
    expect((await call('DELETE', '/v1/booking', { token: person.token })).status).toBe(204);
    const again = await publish(person, page(handle, key));
    expect(again.status).toBe(200);
    expect(again.body).toMatchObject({ enabled: true, device: { current: true } });

    // Signed in again on the same computer after another session turned it off: a new session, so nothing to catch up on.
    const second = (await login(person)).body;
    await call('DELETE', '/v1/booking', { token: second.token });
    const fresh = (await login(person, 'Same desktop, signed in again')).body;
    expect((await publish(fresh, page(handle, key))).status).toBe(200);
  });
});

describe('deleting the account', () => {
  it('wipes the page and keeps its handle from anyone else', async () => {
    const { person, handle } = await host(signup);
    await bookDirect(handle, tenAm(4));
    const res = await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } });
    expect(res.status).toBe(204);
    expect(res.text).toBe('');
    const view = await bookingStub(handle).view(TIMES);
    expect(view).toMatchObject({ state: 'page', accepting: false, profile: null });
    const stranger = await signup();
    expect((await publish(stranger, page(handle, await hostKey()))).body.error).toBe('handle_reserved');
  });

  it('keeps the handle from a new sign-up with the same email; only the deleting device’s key takes it back', async () => {
    const { person: alice, handle } = await host(signup);
    const handleKey = randomKey();
    const res = await call('DELETE', '/v1/account', { token: alice.token, body: { authKey: alice.authKey, handleKey } });
    expect(res.status).toBe(204);
    // Sign-ups don't prove the address: someone else can sign up with it, but not take the page.
    const again = await signup({ email: alice.email });
    const check = await call('GET', `/v1/booking/handles/${handle}`, { token: again.token });
    expect(check.body).toEqual({ handle, available: false, reason: 'reserved' });
    expect((await publish(again, page(handle, await hostKey()))).body.error).toBe('handle_reserved');
    expect((await publish(again, { ...page(handle, await hostKey()), reclaim: randomKey() })).body.error).toBe('handle_reserved');
    // The device that deleted it, with the account made again, has the key.
    const back = await publish(again, { ...page(handle, await hostKey()), reclaim: handleKey });
    expect(back.status).toBe(200);
    expect(back.body).toMatchObject({ handle, enabled: true });
  });

  it('without a key from the deleting device, nobody gets the handle back', async () => {
    const { person: alice, handle } = await host(signup);
    expect((await call('DELETE', '/v1/account', { token: alice.token, body: { authKey: alice.authKey } })).status).toBe(204);
    const again = await signup({ email: alice.email });
    expect((await publish(again, page(handle, await hostKey()))).body.error).toBe('handle_reserved');
  });

  // Ten wrong passwords for a new recovery key: the session's own password checks are locked for 15 minutes.
  async function lockSession(person) {
    for (let i = 0; i < 10; i++) {
      await call('POST', '/v1/recovery', {
        token: person.token,
        body: { authKey: randomKey(), recoveryAuth: randomKey(), recoveryWrappedKey: sealed() }
      });
    }
  }

  it("refuses a locked session's password check before the device deletes the page, as the deletion would", async () => {
    const { person, handle } = await host(signup);
    await bookDirect(handle, tenAm(4));
    await lockSession(person);
    const checked = await call('POST', '/v1/password/check', { token: person.token, body: { authKey: person.authKey } });
    expect(checked.status).toBe(429);
    // The device stops there: the page keeps its booking, and the deletion would have been refused the same way.
    expect((await call('GET', '/v1/booking/bookings', { token: person.token })).body.bookings).toHaveLength(1);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } })).status).toBe(429);
  });

  it("deletes the page, then the account, after the password check on the deleting session", async () => {
    // Rukoo for desktop from this version. Nine typos before don't stand in the way: the right password resets them.
    const { person, handle } = await host(signup);
    await bookDirect(handle, tenAm(4));
    for (let i = 0; i < 9; i++) {
      expect((await call('POST', '/v1/password/check', { token: person.token, body: { authKey: randomKey() } })).status).toBe(403);
    }
    expect((await call('POST', '/v1/password/check', { token: person.token, body: { authKey: person.authKey } })).status).toBe(204);
    expect((await call('DELETE', '/v1/booking', { token: person.token })).status).toBe(204);
    expect((await call('GET', '/v1/booking/bookings', { token: person.token })).body.bookings).toHaveLength(0);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } })).status).toBe(204);
    expect(await tables(await accountStub(person.email))).toEqual([]);
  });

  it("refuses to turn the page off from a locked session, so a desktop that checks the password with a sign-in keeps its bookings", async () => {
    // Rukoo for desktop from 2.1.0 checks the password with a sign-in of its own, deletes the page and then deletes
    // the account on its session, and stops when the page can't go. The sign-in leaves the session's lock alone.
    const { person, handle } = await host(signup);
    await bookDirect(handle, tenAm(4));
    await lockSession(person);
    const signedIn = await login(person, 'Same desktop');
    expect(signedIn.status).toBe(200);
    expect((await call('POST', '/v1/logout', { token: signedIn.body.token })).status).toBe(204);
    const refused = await call('DELETE', '/v1/booking', { token: person.token });
    expect(refused.status).toBe(429);
    expect(refused.body.error).toBe('locked');
    expect(Number(refused.headers.get('retry-after'))).toBeGreaterThan(14 * 60);
    expect(Number(refused.headers.get('retry-after'))).toBeLessThanOrEqual(15 * 60);

    // Nothing went: the page is on with its booking, and the account is there. Reading the page still works.
    const view = await call('GET', '/v1/booking', { token: person.token });
    expect(view.status).toBe(200);
    expect(view.body).toMatchObject({ handle, enabled: true });
    expect((await call('GET', '/v1/booking/bookings', { token: person.token })).body.bookings).toHaveLength(1);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } })).status).toBe(429);
    expect(await tables(await accountStub(person.email))).toContain('account');

    // Once the lock is over, the same steps delete both.
    later(16 * 60 * 1000);
    expect((await login(person, 'Same desktop')).status).toBe(200);
    expect((await call('DELETE', '/v1/booking', { token: person.token })).status).toBe(204);
    expect((await call('DELETE', '/v1/account', { token: person.token, body: { authKey: person.authKey } })).status).toBe(204);
    expect(await tables(await accountStub(person.email))).toEqual([]);
  });
});

describe('a handle this account lost', () => {
  it('answers 409 handle_lost, never 401', async () => {
    const { person, key, handle } = await host(signup);
    await runInDurableObject(bookingStub(handle), (i, state) => {
      state.storage.sql.exec("UPDATE page SET owner = 'someone-else'");
    });
    for (const [method, path, body] of [
      ['GET', '/v1/booking'],
      ['GET', '/v1/booking/bookings'],
      ['POST', `/v1/booking/bookings/${'A'.repeat(22)}`, { action: 'confirm', version: 1 }],
      ['PUT', '/v1/booking', page(handle, key)]
    ]) {
      const res = await call(method, path, { token: person.token, body });
      expect(res.status, `${method} ${path}`).toBe(409);
      expect(res.body.error).toBe('handle_lost');
    }
    // Turning the page off still works: nothing of this account is left there.
    expect((await call('DELETE', '/v1/booking', { token: person.token })).status).toBe(204);
  });
});

describe('a lost random key', () => {
  it('keeps a random session token from reaching anything', async () => {
    const res = await call('PUT', '/v1/booking', { token: `${randomKey().slice(0, 43)}.${'c'.repeat(43)}`, body: {} });
    expect(res.status).toBe(401);
  });
});
