'use strict';

// The booking page across devices and accounts: one key pair whoever makes it first, one booking device that another
// computer can take over, signing in again on the booking device, signing out, another Rukoo account on the same
// computer, and deleting the Rukoo account with and without a booking page.

const test = require('node:test');
const assert = require('node:assert/strict');
const { startAccountServer } = require('./fixtures/account-server');
const { fakeCalendar, setup } = require('./fixtures/google-calendar');
const { AccountService, bookingAcct } = require('../src/main/account');
const { Booking } = require('../src/main/booking');
const { newKeyring } = require('../src/main/booking/keys');
const { calendarKey } = require('../src/main/calendar');

const secrets = { encrypt: (s) => `plain:${Buffer.from(s).toString('base64')}`, decrypt: (s) => Buffer.from(String(s).slice(6), 'base64').toString() };
const PASSWORD = 'a long enough password';
const HOUR = 3600000;
const DAY = 24 * HOUR;
const ENABLE = { handle: 'bart', name: 'Bart', timeZone: 'Europe/Amsterdam', destination: calendarKey('g1', 'me@example.com') };

function slot(days, hour) {
  const d = new Date(Date.now() + days * DAY);
  while ([0, 6].includes(d.getUTCDay())) d.setUTCDate(d.getUTCDate() + 1);
  d.setUTCHours(hour - 1, 0, 0, 0);
  return d.getTime();
}

async function server(t, options) {
  const s = await startAccountServer(options);
  t.after(() => s.close());
  return s;
}

// fake: the Google calendar, shared when two computers use the same calendar account.
async function computer(t, srv, { email = 'host@example.com', signUp = true, name = 'desk', fake = fakeCalendar() } = {}) {
  const { engine, calendars, dataDir } = setup(fake);
  const account = new AccountService({ dataDir, secrets, defaultServer: srv.url, deviceName: name, platform: 'linux' }).init();
  if (signUp) await account.signUp({ email, password: PASSWORD });
  else await account.signIn({ email, password: PASSWORD });
  const booking = new Booking({ account, calendars, engine, dataDir, notify: () => {}, timing: { auto: false, publishAfter: HOUR, reconcileAfter: HOUR } }).init();
  account.beforeDelete = () => booking.disableForDelete();
  t.after(() => {
    booking.close();
    account.close();
    calendars.flush();
  });
  return { engine, calendars, account, booking, fake, dataDir };
}

const hasKey = (account) => [...account.entries.values()].some((e) => e.c === 'bookingkey' && !e.deleted);

test('two devices that make a key at the same time end up with the one the server kept first', async (t) => {
  const srv = await server(t);
  const a = await computer(t, srv, { name: 'desk' });
  const b = await computer(t, srv, { signUp: false, name: 'laptop' });
  const [ka, kb] = await Promise.all([a.booking.ensureKey(), b.booking.ensureKey()]);
  assert.equal(ka.current, kb.current);
  assert.deepEqual(a.account.get('bookingkey', a.account.acct), b.account.get('bookingkey', b.account.acct));

  // The same race, ordered: laptop made its own key after a pull that came before desk's write reached the server.
  const c = await computer(t, srv, { signUp: false, name: 'tablet' });
  await c.account.sync();
  const acct = c.account.acct;
  // Its pull brought desk's key; take it away to play a pull from just before.
  c.account.entries.delete(`bookingkey\n${acct}`);
  c.account.put('bookingkey', acct, newKeyring(), { ifAbsent: true });
  const kc = await c.booking.ensureKey();
  assert.equal(kc.current, ka.current, "the server's first write wins over this device's own");
  assert.equal(c.account.get('bookingkey', acct).current, ka.current);
});

test('another computer shows the page read-only until it takes over; the first then stops publishing', async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv, { name: 'desk' });
  await desk.booking.enable(ENABLE);
  const laptop = await computer(t, srv, { signUp: false, name: 'laptop' });
  await laptop.account.sync();
  let state = await laptop.booking.load({ refresh: true });
  assert.equal(state.status, 'on');
  assert.equal(state.thisDevice, false);
  assert.deepEqual(state.device, { id: srv.pages.get('bart').deviceId, name: 'desk', current: false });
  assert.equal(state.key, 'ok', 'the key came with the account');
  const notice = state.notices.find((n) => n.kind === 'otherDevice');
  assert.equal(notice.title, 'Bookings are handled by desk');
  assert.deepEqual(notice.actions.map((a) => a.action), ['takeover', 'dismiss']);
  assert.equal(await laptop.booking.publish({ force: true }), false, 'only the booking device publishes');
  // Every computer but the booking device shows it; it can be put away until another computer takes over.
  state = await laptop.booking.action('otherDevice', 'dismiss');
  assert.equal(state.notices.find((n) => n.kind === 'otherDevice'), undefined);
  assert.equal(state.device.name, 'desk');

  // A booking made now shows on the laptop with the guest's details, which it can open with the shared key.
  const { id } = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10), guest: { name: 'Jane Doe' } });
  state = await laptop.booking.sync();
  assert.equal(state.bookings[0].guest.name, 'Jane Doe');
  assert.equal(srv.pages.get('bart').bookings.get(id).handledVersion, 0, "the laptop doesn't process it");
  await assert.rejects(laptop.booking.action(id, 'cancel'), (err) => err.code === 'notBookingDevice');

  state = await laptop.booking.takeover();
  assert.equal(state.thisDevice, true);
  assert.equal(srv.pages.get('bart').deviceId, [...srv.sessions.values()].find((s) => s.name === 'laptop').id);
  assert.equal(srv.pages.get('bart').bookings.get(id).handledVersion, 1, 'the new booking device processed it');
  assert.ok(laptop.fake.store.some((e) => e.extendedProperties && e.extendedProperties.private.rukooBooking === id));
  assert.equal(laptop.booking.settings().device, srv.pages.get('bart').deviceId);

  state = await desk.booking.sync();
  assert.equal(state.thisDevice, false);
  assert.equal(await desk.booking.publish({ force: true }), false);
  const puts = srv.calls.filter((c) => c.route === 'PUT /v1/booking');
  assert.equal(puts.at(-1).body.takeover, true);
});

test("a computer without the calendar for bookings can't take the page over, so no booking goes without its event", async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv, { name: 'desk' });
  await desk.booking.enable(ENABLE);
  const laptop = await computer(t, srv, { signUp: false, name: 'laptop' });
  // The laptop has another mail account only.
  laptop.engine.accounts[0].email = 'someone-else@example.com';
  await laptop.account.sync();
  await laptop.booking.load({ refresh: true });
  await assert.rejects(laptop.booking.takeover(), (err) => err.code === 'invalid' && err.field === 'destination' && /isn't on this computer/.test(err.message));
  assert.equal(laptop.booking.state().thisDevice, false);
  assert.equal(srv.pages.get('bart').deviceId, [...srv.sessions.values()].find((s) => s.name === 'desk').id);
});

test('signing in again on the booking device takes the page over for the new session by itself', async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv);
  await desk.booking.enable(ENABLE);
  const first = srv.pages.get('bart').deviceId;
  await desk.account.signOut();
  assert.equal(hasKey(desk.account), false, 'signing out removes the private key here');
  assert.equal(desk.booking.state().status, 'signed_out');
  await desk.account.signIn({ email: 'host@example.com', password: PASSWORD });
  await desk.account.sync();
  assert.equal(hasKey(desk.account), true, 'and signing in brings it back');

  const { id } = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10) });
  const state = await desk.booking.sync();
  assert.equal(state.thisDevice, true);
  assert.notEqual(srv.pages.get('bart').deviceId, first);
  assert.equal(srv.pages.get('bart').bookings.get(id).handledVersion, 1);
  const put = srv.calls.filter((c) => c.route === 'PUT /v1/booking').at(-1);
  assert.equal(put.body.takeover, true);
});

test('a page turned off on another computer stays off: the booking device asks before it publishes', async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv, { name: 'desk' });
  await desk.booking.enable(ENABLE);
  const laptop = await computer(t, srv, { signUp: false, name: 'laptop' });
  await laptop.account.sync();
  await laptop.booking.load({ refresh: true });
  await laptop.booking.disable();
  // The desk's half-hourly publish, or the one after a calendar change, before its next poll.
  assert.equal(await desk.booking.publish({ force: true }), false);
  assert.equal(srv.pages.get('bart').enabled, false);
  assert.equal(srv.calls.filter((c) => c.route === 'PUT /v1/booking').length, 1, 'only the one that turned it on');
  assert.equal(desk.booking.state().status, 'off');
});

test("the server refuses the former booking device's publish until it has read the page another computer turned off", async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv, { name: 'desk' });
  await desk.booking.enable(ENABLE);
  const { body } = srv.calls.filter((c) => c.route === 'PUT /v1/booking').at(-1);
  const laptop = await computer(t, srv, { signUp: false, name: 'laptop' });
  await laptop.account.sync();
  await laptop.booking.load({ refresh: true });
  await laptop.booking.disable();
  const deskSession = [...srv.sessions.values()].find((s) => s.name === 'desk').id;
  assert.equal(srv.pages.get('bart').offDevice, deskSession);

  // What the desk had, sent as an older Rukoo would, without reading first: refused, and the page stays off.
  await assert.rejects(desk.account.bookingCall('PUT', '/v1/booking', { ...body, takeover: false }), (err) => err.code === 'other_device');
  await assert.rejects(desk.account.bookingCall('PUT', '/v1/booking', { ...body, handle: 'bart-vdm', takeover: false }), (err) => err.code === 'other_device');
  assert.equal(srv.pages.get('bart').enabled, false);
  // The laptop, which turned it off, isn't held back.
  assert.equal(laptop.booking.state().status, 'off');

  // Rukoo reads before it publishes: that clears the block, and turning the page on again from setup needs no takeover.
  assert.equal(await desk.booking.publish({ force: true }), false);
  assert.equal(srv.pages.get('bart').offDevice, null);
  const state = await desk.booking.enable(ENABLE);
  assert.equal(state.status, 'on');
  assert.equal(state.thisDevice, true);
  assert.notEqual(srv.calls.filter((c) => c.route === 'PUT /v1/booking').at(-1).body.takeover, true);
});

test("a computer that took over with an older copy of the calendar applies the guest's move and cancellation", async (t) => {
  const srv = await server(t);
  const fake = fakeCalendar();
  const desk = await computer(t, srv, { fake, name: 'desk' });
  await desk.booking.enable(ENABLE);
  const laptop = await computer(t, srv, { fake, signUp: false, name: 'laptop' });
  const start = slot(3, 10);
  // The laptop's calendar view was filled before the desk made the bookings' events.
  await laptop.calendars.ensure('g1', start - 7 * DAY, start + 7 * DAY);
  const a = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  const b = await srv.book({ handle: 'bart', type: '30min', start: start + DAY, guest: { name: 'Max Power', email: 'max@example.org' } });
  const c = await srv.book({ handle: 'bart', type: '30min', start: start + 2 * DAY, guest: { name: 'Ann Lee', email: 'ann@example.org' } });
  await desk.booking.sync();
  await desk.account.sync();
  await laptop.account.sync();
  // c's mapping record hasn't come to the laptop yet.
  laptop.account.entries.delete(`booking\nmap:${laptop.account.acct}:${c.id}`);
  await laptop.booking.load({ refresh: true });
  await laptop.booking.takeover();
  const row = (id) => srv.pages.get('bart').bookings.get(id);
  const live = (id) => fake.store.filter((e) => e.extendedProperties && e.extendedProperties.private && e.extendedProperties.private.rukooBooking === id && e.status !== 'cancelled');
  assert.deepEqual([row(a.id).status, row(b.id).status, row(c.id).status], ['confirmed', 'confirmed', 'confirmed'], "events the laptop's copy lacks aren't taken as deleted");

  const moved = start + 2 * HOUR;
  srv.guestReschedule({ handle: 'bart', id: a.id, start: moved });
  await laptop.booking.sync();
  assert.equal(row(a.id).start, moved, "the guest's move stands");
  assert.equal(row(a.id).handledVersion, row(a.id).version);
  assert.deepEqual(live(a.id).map((e) => Date.parse(e.start.dateTime)), [moved], 'and the one event moved with it');

  // The copy is older than the events again.
  const cache = laptop.calendars.cache('g1');
  cache.events = cache.events.filter((e) => !e.bookingId);
  srv.guestCancel({ handle: 'bart', id: b.id });
  await laptop.booking.sync();
  assert.deepEqual(live(b.id), [], 'the cancellation removed the event');
  assert.equal(row(b.id).handledVersion, row(b.id).version);
  assert.match(laptop.booking.state().notices.find((n) => n.id === `guestCancelled:${b.id}`).text, /removed it from your calendar/);

  // Without a mapping record the event is found by its booking.
  srv.guestCancel({ handle: 'bart', id: c.id });
  await laptop.booking.sync();
  assert.deepEqual(live(c.id), []);
  assert.equal(row(c.id).handledVersion, row(c.id).version);
  assert.ok(laptop.booking.state().notices.some((n) => n.id === `guestCancelled:${c.id}`));
});

test("another Rukoo account on the same computer starts from nothing and gets none of the first account's records", async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv, { email: 'one@example.com' });
  await desk.booking.enable(ENABLE);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10) });
  await desk.booking.sync();
  const one = bookingAcct('one@example.com');
  assert.ok(desk.account.get('booking', `map:${one}:${id}`));
  await desk.account.signOut();

  await desk.account.signUp({ email: 'two@example.com', password: PASSWORD });
  await desk.account.sync();
  const state = await desk.booking.load({ refresh: true });
  assert.equal(state.status, 'off');
  assert.equal(state.settings, null);
  assert.equal(state.handle, null);
  assert.deepEqual([state.bookings, state.requests], [[], []]);
  assert.equal(desk.booking.file.acct, bookingAcct('two@example.com'));
  assert.deepEqual(desk.booking.file.rows, {});
  assert.equal(srv.accounts.get('two@example.com').records.size, 0, "the first account's booking records stay out of the second");
  // They are still here for when the first account signs in again.
  assert.equal(desk.account.list('booking').filter((r) => r.k.includes(one)).length >= 2, true);
  // The second account's export holds none of them: they name the first one's guests.
  assert.deepEqual(desk.account.exportData().records.filter((r) => r.collection === 'booking'), []);
  await desk.account.signOut();
  assert.deepEqual(desk.account.exportData().records.filter((r) => r.collection === 'booking'), []);
});

test('the export leaves out the booking key', async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv);
  await desk.booking.enable(ENABLE);
  const collections = desk.account.exportData().records.map((r) => r.collection);
  assert.ok(collections.includes('booking'));
  assert.ok(!collections.includes('bookingkey'));
});

test('deleting the Rukoo account turns the booking page off first and stops when that fails', async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv);
  await desk.booking.enable(ENABLE);
  await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10) });

  // A mistyped password stops before anything goes: the page and its bookings can't come back.
  await assert.rejects(desk.account.deleteAccount('a mistyped password!!'), (err) => err.code === 'wrong_password');
  assert.equal(srv.pages.get('bart').enabled, true);
  assert.equal(srv.pages.get('bart').bookings.size, 1);
  assert.equal(srv.calls.filter((c) => c.route === 'DELETE /v1/booking').length, 0);
  assert.equal(desk.account.signedIn, true);

  srv.fail = ({ route }) => (route === 'DELETE /v1/booking' ? [500, { error: 'server' }] : null);
  await assert.rejects(desk.account.deleteAccount(PASSWORD), (err) => err.code === 'deleteFirst' && /wasn't deleted/.test(err.message));
  assert.ok(srv.accounts.has('host@example.com'));
  assert.equal(srv.calls.filter((c) => c.route === 'DELETE /v1/account').length, 0);

  // A server that answers 404 has no page to delete: the account goes.
  srv.fail = ({ route }) => (route === 'DELETE /v1/booking' ? [404, { error: 'not_found' }] : null);
  await desk.account.deleteAccount(PASSWORD);
  assert.equal(srv.accounts.has('host@example.com'), false);

  // The password is checked on the session that deletes the account, under the lock the deletion uses, before the
  // page goes.
  const srv2 = await server(t);
  const laptop = await computer(t, srv2, { name: 'laptop' });
  await laptop.booking.enable(ENABLE);
  await srv2.book({ handle: 'bart', type: '30min', start: slot(3, 10) });
  srv2.calls.length = 0;
  await laptop.account.deleteAccount(PASSWORD);
  let routes = srv2.calls.map((c) => c.route);
  assert.ok(routes.indexOf('POST /v1/password/check') >= 0);
  assert.ok(routes.indexOf('POST /v1/password/check') < routes.indexOf('DELETE /v1/booking'));
  assert.ok(routes.indexOf('DELETE /v1/booking') < routes.indexOf('DELETE /v1/account'));
  assert.equal(routes.includes('POST /v1/login'), false, 'no sign-in of its own');
  assert.equal(srv2.calls.find((c) => c.route === 'POST /v1/password/check').body.authKey, srv2.calls.find((c) => c.route === 'DELETE /v1/account').body.authKey);

  // A server from before that call answers 404: the password is checked with a sign-in of its own, which has ended
  // by the time the page goes.
  const srv3 = await server(t, { passwordCheck: false });
  const tablet = await computer(t, srv3, { name: 'tablet' });
  await tablet.booking.enable(ENABLE);
  await srv3.book({ handle: 'bart', type: '30min', start: slot(3, 10) });
  const sessions = srv3.sessions.size;
  let during = null;
  srv3.fail = ({ route }) => {
    if (route === 'DELETE /v1/booking') during = srv3.sessions.size;
    return null;
  };
  await tablet.account.deleteAccount(PASSWORD);
  routes = srv3.calls.map((c) => c.route);
  assert.ok(routes.includes('DELETE /v1/booking'));
  const login = routes.lastIndexOf('POST /v1/login');
  assert.ok(routes.lastIndexOf('POST /v1/password/check') < login);
  assert.ok(login >= 0 && login < routes.lastIndexOf('POST /v1/logout'));
  assert.ok(routes.lastIndexOf('POST /v1/logout') < routes.indexOf('DELETE /v1/booking'));
  assert.ok(routes.indexOf('DELETE /v1/booking') < routes.indexOf('DELETE /v1/account'));
  assert.equal(during, sessions);
  assert.equal(srv3.accounts.has('host@example.com'), false);
});

test("a session locked after ten wrong passwords keeps the booking page and the account until the lock is over", async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv);
  await desk.booking.enable(ENABLE);
  await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10) });

  // Ten wrong passwords for a new recovery key lock this session's password checks. Sign-ins aren't locked.
  for (let i = 0; i < 9; i++) await assert.rejects(desk.account.newRecoveryKey('a mistyped password!!'), (err) => err.code === 'wrong_password');
  await assert.rejects(desk.account.newRecoveryKey('a mistyped password!!'), (err) => err.code === 'locked');

  // The right password now: the check refuses it as the deletion would, before the page goes.
  await assert.rejects(desk.account.deleteAccount(PASSWORD), (err) => err.code === 'locked');
  assert.equal(srv.pages.get('bart').enabled, true);
  assert.equal(srv.pages.get('bart').bookings.size, 1);
  assert.equal(srv.calls.filter((c) => c.route === 'DELETE /v1/booking').length, 0);
  assert.ok(srv.accounts.has('host@example.com'));
  assert.equal(desk.account.signedIn, true);
  assert.equal(desk.booking.status(), 'on');
  // Turning the page off from this session waits for the lock as well, with the sign-in lock's message.
  await assert.rejects(desk.booking.disable(), (err) => err.code === 'locked');
  assert.equal(srv.pages.get('bart').bookings.size, 1);
  assert.equal(desk.booking.status(), 'on');

  // After the lock, the same password deletes both.
  srv.now = () => Date.now() + 16 * 60000;
  await desk.account.deleteAccount(PASSWORD);
  assert.equal(srv.accounts.has('host@example.com'), false);
  assert.equal(srv.pages.get('bart').bookings.size, 0);
});

test('a locked session that checks the password with a sign-in, as Rukoo from 2.1.0 does, stops at the booking page', async (t) => {
  // Without POST /v1/password/check this desktop takes the steps of the released ones: a sign-in of its own, the
  // booking page, then the account. The sign-in leaves the session's lock alone and the server refuses the page.
  const srv = await server(t, { passwordCheck: false });
  const desk = await computer(t, srv);
  await desk.booking.enable(ENABLE);
  await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10) });
  for (let i = 0; i < 10; i++) await desk.account.newRecoveryKey('a mistyped password!!').catch(() => {});

  srv.calls.length = 0;
  await assert.rejects(desk.account.deleteAccount(PASSWORD), (err) => err.code === 'deleteFirst');
  const routes = srv.calls.map((c) => c.route);
  assert.ok(routes.indexOf('POST /v1/login') < routes.indexOf('DELETE /v1/booking'));
  assert.equal(routes.includes('DELETE /v1/account'), false);
  assert.equal(srv.pages.get('bart').enabled, true);
  assert.equal(srv.pages.get('bart').bookings.size, 1);
  assert.ok(srv.accounts.has('host@example.com'));
  assert.equal(desk.account.signedIn, true);

  srv.now = () => Date.now() + 16 * 60000;
  await desk.account.deleteAccount(PASSWORD);
  assert.equal(srv.accounts.has('host@example.com'), false);
  assert.equal(srv.pages.get('bart').bookings.size, 0);
});

test("an account deleted and made again on the same computer takes its handle back; a sign-up elsewhere with the email doesn't", async (t) => {
  const srv = await server(t);
  const desk = await computer(t, srv);
  await desk.booking.enable(ENABLE);
  await desk.account.deleteAccount(PASSWORD);
  assert.equal(srv.pages.get('bart').owner.startsWith('deleted:'), true);

  // Someone else signs up with the same address first: the address isn't proven, so that gets no handle.
  const other = await computer(t, srv, { name: 'elsewhere' });
  assert.deepEqual(await other.booking.checkHandle('bart'), { handle: 'bart', available: false, reason: 'reserved' });
  await assert.rejects(other.booking.enable(ENABLE), (err) => err.code === 'handle_reserved');
  await other.account.deleteAccount(PASSWORD);

  // The computer that deleted it kept the key: made again there, the account has the handle back.
  await desk.account.signUp({ email: 'host@example.com', password: PASSWORD });
  assert.deepEqual(await desk.booking.checkHandle('bart'), { handle: 'bart', available: true, reason: null });
  const state = await desk.booking.enable(ENABLE);
  assert.equal(state.handle, 'bart');
  assert.equal(srv.pages.get('bart').owner, 'host@example.com');
  assert.equal(desk.account.handleKeyFor('host@example.com'), null, 'used once');
});

test('a server without booking pages is left alone when the account is deleted', async (t) => {
  const srv = await server(t, { features: [] });
  const desk = await computer(t, srv);
  const state = await desk.booking.load({ refresh: true });
  assert.equal(state.status, 'unsupported');
  await assert.rejects(desk.booking.enable(ENABLE), (err) => err.code === 'unsupported');
  await desk.account.deleteAccount(PASSWORD);
  assert.equal(srv.accounts.has('host@example.com'), false);
  assert.equal(srv.calls.filter((c) => c.route.includes('/v1/booking')).length, 0);
});
