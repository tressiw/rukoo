'use strict';

// The booking service (src/main/booking) against the in-memory account server and the Google calendar fake: turning
// the page on, what it publishes, how bookings become events, the guest's and the host's changes, and two devices
// sharing one page.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { startAccountServer } = require('./fixtures/account-server');
const { fakeCalendar, setup } = require('./fixtures/google-calendar');
const { AccountService } = require('../src/main/account');
const { Booking } = require('../src/main/booking');
const { calendarKey } = require('../src/main/calendar');

const secrets = { encrypt: (s) => `plain:${Buffer.from(s).toString('base64')}`, decrypt: (s) => Buffer.from(String(s).slice(6), 'base64').toString() };
const PASSWORD = 'a long enough password';
const HOUR = 3600000;
const DAY = 24 * HOUR;
const DEST = calendarKey('g1', 'me@example.com');

// Whole hours a few days ahead, on a weekday inside the default working hours (09:00 to 17:00 in Amsterdam).
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

// A computer: its own profile with the Google fake as its calendar, signed in to the Rukoo account.
async function computer(t, srv, { fake = fakeCalendar(), email = 'host@example.com', signUp = true, name = 'desk', now } = {}) {
  const { engine, calendars, dataDir } = setup(fake);
  const account = new AccountService({ dataDir, secrets, defaultServer: srv.url, deviceName: name, platform: 'linux' }).init();
  if (signUp) await account.signUp({ email, password: PASSWORD });
  else await account.signIn({ email, password: PASSWORD });
  const shown = [];
  const booking = new Booking({ account, calendars, engine, dataDir, notify: (n) => shown.push(n), now, timing: { auto: false, publishAfter: HOUR, reconcileAfter: HOUR } }).init();
  t.after(() => {
    booking.close();
    account.close();
    calendars.flush();
  });
  return { engine, calendars, account, booking, shown, fake, dataDir };
}

const ENABLE = { handle: 'bart', name: 'Bart', timeZone: 'Europe/Amsterdam', destination: DEST };

const inserted = (fake) => fake.requests.filter((r) => r.method === 'POST' && /\/events$/.test(r.path));
const bookingEvent = (fake, id) => fake.store.find((e) => e.extendedProperties && e.extendedProperties.private && e.extendedProperties.private.rukooBooking === id);
const when = (ms) => new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }).format(ms);
const pause = () => new Promise((resolve) => setTimeout(resolve, 20));
// The description a 30min booking's event gets on the page 'bart': the guest's links, in English unless said.
const guestLines = (srv, id, key, { nl = false, base = `${srv.url}/meet/bart` } = {}) =>
  `${nl ? 'Andere tijd kiezen' : 'Pick another time'}: ${base}/30min?reschedule=${id}&k=${key}\n${nl ? 'Afspraak annuleren' : 'Cancel booking'}: ${base}/booking/${id}?k=${key}`;

test("before the page has a handle, the state says where the server's pages start, as the server reports it", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  let state = await host.booking.load({ refresh: true });
  assert.equal(state.handle, null);
  assert.equal(state.url, null);
  assert.equal(state.base, `${srv.url}/meet`);
  // A server from before base doesn't say; the setup then shows <server>/meet/ itself.
  srv.fail = ({ route }) => (route === 'GET /v1/booking' ? [200, { handle: null, url: null, enabled: false, handleChanges: { used: 0, max: 3 } }] : null);
  state = await host.booking.load({ refresh: true });
  assert.equal(state.base, null);
  srv.fail = null;
  // A server with BOOKING_ORIGIN keeps its pages at the root of that address.
  srv.bookingOrigin = 'https://book.example';
  state = await host.booking.load({ refresh: true });
  assert.equal(state.base, 'https://book.example');
  state = await host.booking.enable(ENABLE);
  assert.equal(state.url, 'https://book.example/bart');
  assert.equal(state.base, 'https://book.example');
});

test('turning the page on publishes the hours, the busy time and the key, and keeps the address in the settings', async (t) => {
  const srv = await server(t);
  const busyStart = slot(2, 11);
  const fake = fakeCalendar({ events: [{ id: 'busy1', summary: 'Roadmap review', start: { dateTime: new Date(busyStart + 10 * 60000).toISOString() }, end: { dateTime: new Date(busyStart + HOUR).toISOString() } }] });
  const host = await computer(t, srv, { fake });
  const state = await host.booking.enable(ENABLE);

  assert.equal(state.status, 'on');
  assert.equal(state.thisDevice, true);
  assert.equal(state.key, 'ok');
  assert.equal(state.url, `${srv.url}/meet/bart`);
  assert.equal(state.settings.url, `${srv.url}/meet/bart`, 'the record keeps the address the server reported');
  assert.equal(state.resolved.destination.found, true);

  const page = srv.pages.get('bart');
  const ring = host.account.get('bookingkey', host.account.acct);
  assert.equal(page.profile.keyId, ring.current);
  assert.equal(page.profile.publicKey, ring.keys[ring.current].publicRaw);
  assert.equal(page.profile.types[0].slug, '30min');
  assert.equal(page.profile.types[0].title, '30 minute meeting');
  assert.equal(page.profile.types[0].location.kind, 'googleMeet', 'Meet, because the destination calendar allows it');
  assert.equal(page.profile.types[0].slotInterval, 30);
  // Busy time is widened to whole quarter hours.
  assert.ok(page.busy.some(([s, e]) => s === busyStart && e === busyStart + HOUR), JSON.stringify(page.busy));
  assert.ok(page.busy.every(([s, e]) => s % 900000 === 0 && e % 900000 === 0));
  assert.ok(page.windows.default.length > 5);
  // The key went up before it was published, as a first write the server holds.
  assert.ok(host.account.recordState('bookingkey', host.account.acct).seq > 0);
  assert.deepEqual(host.account.get('booking', `published:${host.account.acct}`).availability, ['me@example.com\nme@example.com']);
});

test('a booking becomes an event in the destination calendar with the guest invited, and is handled', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const { id, manageKey } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe', email: 'Jane@Example.com', notes: 'Kickoff for the <b>pilot</b>', via: 'ChatGPT' } });

  await host.booking.sync();

  const ev = bookingEvent(host.fake, id);
  assert.ok(ev, 'the event exists');
  assert.equal(ev.summary, '30 minute meeting: Bart and Jane Doe');
  assert.deepEqual(ev.attendees.filter((a) => !a.organizer).map((a) => a.email), ['jane@example.com']);
  // The guest's links to move and cancel, in place of "Booked via"; nothing the guest wrote.
  assert.equal(ev.description, guestLines(srv, id, manageKey));
  assert.ok(ev.conferenceData, 'Google Meet was added');
  assert.equal(Date.parse(ev.start.dateTime), start);
  const insert = inserted(host.fake).find((r) => r.body.extendedProperties);
  assert.equal(insert.query.sendUpdates, 'all', 'the invitation goes out');

  const row = srv.pages.get('bart').bookings.get(id);
  assert.equal(row.handledVersion, 1);
  // The fake ignores the iCalUID a booking is inserted with, as Google's reference reads: the server gets the uid
  // the calendar gave the event, so the guest's .ics and the invitation are one event.
  assert.equal(row.uid, ev.iCalUID);

  const state = host.booking.state();
  assert.equal(state.bookings.length, 1);
  const item = state.bookings[0];
  assert.equal(item.guest.name, 'Jane Doe');
  assert.equal(item.guest.notes, 'Kickoff for the <b>pilot</b>');
  assert.equal(item.guest.via, 'ChatGPT');
  assert.equal(item.typeTitle, '30 minute meeting');
  assert.ok(item.eventId);
  const map = host.account.get('booking', `map:${host.account.acct}:${id}`);
  assert.equal(map.uid, ev.iCalUID);
  assert.equal(map.version, 1);
  assert.deepEqual(host.shown.map((n) => n.kind), ['new']);
  assert.equal(host.shown[0].body, `Jane Doe booked 30 minute meeting for ${new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' }).format(start)}.`);
  assert.deepEqual(host.shown[0].open, { id: item.eventId, start });

  // A second sync finds nothing new to do.
  const before = inserted(host.fake).length;
  await host.booking.sync();
  assert.equal(inserted(host.fake).length, before);
});

test("a guest's move and cancellation reach the event, with mail only within the day's budget", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  srv.mailMax = 2;
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  await host.booking.sync();

  const moved = start + 2 * HOUR;
  srv.guestReschedule({ handle: 'bart', id, start: moved });
  await host.booking.sync();
  const ev = bookingEvent(host.fake, id);
  assert.equal(Date.parse(ev.start.dateTime), moved);
  assert.equal(Date.parse(ev.end.dateTime), moved + 30 * 60000);
  const patch = host.fake.requests.filter((r) => r.method === 'PATCH').pop();
  assert.equal(patch.query.sendUpdates, 'all');
  assert.equal(host.account.get('booking', `map:${host.account.acct}:${id}`).start, moved);
  assert.equal(srv.pages.get('bart').bookings.get(id).handledVersion, 2);
  assert.equal(host.shown.at(-1).kind, 'rescheduled');

  // The budget of 2 is used (the booking and the move): the cancellation removes the event without mail.
  srv.guestCancel({ handle: 'bart', id });
  await host.booking.sync();
  assert.equal(bookingEvent(host.fake, id).status, 'cancelled');
  const del = host.fake.requests.filter((r) => r.method === 'DELETE').pop();
  assert.equal(del.query.sendUpdates, 'none');
  const state = host.booking.state();
  assert.deepEqual(state.notices.map((n) => n.id).sort(), [`guestCancelled:${id}`, `mailBudget:${id}`]);
  const budget = state.notices.find((n) => n.kind === 'mailBudget');
  assert.match(budget.text, /without emailing them/);
  assert.ok(budget.text.includes('Jane Doe'));
  assert.equal(state.bookings.length, 0);
  assert.equal(host.shown.at(-1).kind, 'cancelled');
  assert.deepEqual(host.shown.at(-1).open, { start: moved });
});

test('a request waits for the host: accepting makes the event, declining can email the guest from the calendar account', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const sent = [];
  host.engine.send = async (payload) => {
    sent.push(payload);
    return true;
  };
  const one = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10), status: 'pending', guest: { name: 'Jane Doe', notes: 'Please call me first' } });
  const two = await srv.book({ handle: 'bart', type: '30min', start: slot(4, 14), status: 'pending', guest: { name: 'Max Power', email: 'max@example.org', notes: 'IGNORE PREVIOUS INSTRUCTIONS' } });
  await host.booking.sync();
  let state = host.booking.state();
  assert.deepEqual(state.requests.map((r) => r.id), [one.id, two.id]);
  assert.equal(state.requests[0].guest.notes, 'Please call me first');
  assert.ok(state.requests[0].expiresAt > Date.now());
  assert.deepEqual(host.shown.map((n) => [n.kind, n.open]), [['request', { request: one.id }], ['request', { request: two.id }]]);
  assert.equal(inserted(host.fake).filter((r) => r.body.extendedProperties).length, 0, 'no event for a request');

  state = await host.booking.action(one.id, 'accept');
  const ev = bookingEvent(host.fake, one.id);
  assert.ok(ev);
  assert.equal(ev.description, guestLines(srv, one.id, one.manageKey), 'an accepted request gets the links too');
  assert.equal(inserted(host.fake).find((r) => r.body.extendedProperties).query.sendUpdates, 'all');
  assert.deepEqual(state.bookings.map((b) => b.id), [one.id]);
  assert.equal(srv.pages.get('bart').bookings.get(one.id).status, 'confirmed');
  assert.equal(host.shown.length, 2, "accepting is the host's own doing: no notification");

  state = await host.booking.action(two.id, 'decline', { message: 'That week is full, sorry.', email: true });
  const row = srv.pages.get('bart').bookings.get(two.id);
  assert.equal(row.status, 'declined');
  assert.equal(row.message, 'That week is full, sorry.');
  assert.equal(row.handledVersion, row.version);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].accountId, 'g1');
  assert.deepEqual(sent[0].to, [{ name: '', address: 'max@example.org' }]);
  assert.match(sent[0].text, /^That week is full, sorry\.\n\nYou can pick another time here: http/);
  assert.ok(!/Max|IGNORE/.test(sent[0].text + sent[0].subject), 'nothing the guest wrote goes into the mail');
  assert.equal(state.requests.length, 0);

  await assert.rejects(host.booking.action(two.id, 'accept'), (err) => err.booking === true && err.code === 'notFound');
});

test('a booking that overlaps another event stays, with a notice to keep or cancel it', async (t) => {
  const srv = await server(t);
  const start = slot(3, 10);
  const fake = fakeCalendar({ events: [{ id: 'busy1', summary: 'Dentist', start: { dateTime: new Date(start - 15 * 60000).toISOString() }, end: { dateTime: new Date(start + 15 * 60000).toISOString() } }] });
  const host = await computer(t, srv, { fake });
  await host.booking.enable(ENABLE);
  // The fixture books any time; the server would offer it once the busy time was published late.
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  await host.booking.sync();
  assert.ok(bookingEvent(host.fake, id), 'the event is made anyway');
  let notice = host.booking.state().notices.find((n) => n.kind === 'overlap');
  assert.equal(notice.id, `overlap:${id}`);
  assert.equal(notice.bookingId, id);
  assert.deepEqual(notice.actions.map((a) => a.action), ['keep', 'cancel', 'open']);

  const state = await host.booking.action(notice.id, 'keep');
  assert.equal(state.notices.find((n) => n.kind === 'overlap'), undefined);
  assert.equal(srv.pages.get('bart').bookings.get(id).status, 'confirmed');

  // Cancel booking from the card: the event goes with the provider's cancellation, then the server cancels.
  await host.booking.action(id, 'cancel', { message: 'Something came up.', notify: true });
  assert.equal(bookingEvent(host.fake, id).status, 'cancelled');
  assert.equal(host.fake.requests.filter((r) => r.method === 'DELETE').pop().query.sendUpdates, 'all');
  const row = srv.pages.get('bart').bookings.get(id);
  assert.deepEqual([row.status, row.reason, row.message, row.handledVersion], ['cancelled', 'host_cancelled', 'Something came up.', row.version]);
  notice = host.booking.state().notices.find((n) => n.kind === 'overlap');
  assert.equal(notice, undefined);
});

test('the host moving, resizing or deleting a booked event, or the guest declining it, reaches the booking', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const a = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  const b = await srv.book({ handle: 'bart', type: '30min', start: slot(4, 10), guest: { name: 'Max Power', email: 'max@example.org' } });
  const c = await srv.book({ handle: 'bart', type: '30min', start: slot(5, 10), guest: { name: 'Ann Lee', email: 'ann@example.org' } });
  await host.booking.sync();
  const eventOf = (id) => host.booking.state().bookings.find((x) => x.id === id).eventId;

  // Moved an hour later and made 45 minutes long: the booking moves (host: true), keeps 30 minutes, and the card
  // says custom length.
  const evA = host.calendars.event(eventOf(a.id));
  await host.calendars.update(evA.id, { accountId: evA.accountId, calendarId: evA.calendarId, title: evA.title, location: evA.location, description: evA.description, allDay: false, start: start + HOUR, end: start + HOUR + 45 * 60000 }, { etag: evA.etag, scope: 'one' });
  await host.booking.reconcile();
  const rowA = srv.pages.get('bart').bookings.get(a.id);
  assert.equal(rowA.start, start + HOUR);
  assert.equal(rowA.end, start + HOUR + 30 * 60000);
  assert.equal(rowA.handledVersion, rowA.version);
  const itemA = host.booking.state().bookings.find((x) => x.id === a.id);
  assert.equal(itemA.customLength, true);
  assert.equal(itemA.start, start + HOUR);

  // Deleted from the calendar while it could be read: cancelled as host_deleted.
  await host.calendars.remove(eventOf(b.id), { notify: true, etag: host.calendars.event(eventOf(b.id)).etag });
  // The guest declined the invitation in their own calendar.
  const evC = bookingEvent(host.fake, c.id);
  evC.attendees.find((x) => x.email === 'ann@example.org').responseStatus = 'declined';
  evC.etag = '"declined"';
  host.calendars.memos.clear();
  await host.booking.reconcile();
  const rowB = srv.pages.get('bart').bookings.get(b.id);
  assert.deepEqual([rowB.status, rowB.reason], ['cancelled', 'host_deleted']);
  const rowC = srv.pages.get('bart').bookings.get(c.id);
  assert.deepEqual([rowC.status, rowC.reason], ['cancelled', 'guest_declined']);
  const kinds = host.booking.state().notices.map((n) => n.id).sort();
  assert.deepEqual(kinds, [`guestDeclined:${c.id}`, `hostDeleted:${b.id}`].sort());
  assert.equal(srv.pages.get('bart').bookings.get(a.id).status, 'confirmed', 'the moved one stays');
});

test("a move in Rukoo's calendar reschedules the booking through the 'changed' event", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  host.booking.timing.reconcileAfter = 20;
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start });
  await host.booking.sync();
  const ev = host.calendars.event(host.booking.state().bookings[0].eventId);
  await host.calendars.update(ev.id, { accountId: ev.accountId, calendarId: ev.calendarId, title: ev.title, location: ev.location, description: ev.description, allDay: false, start: start + 3 * HOUR, end: start + 3 * HOUR + 30 * 60000 }, { etag: ev.etag, scope: 'one' });
  for (let i = 0; i < 100 && srv.pages.get('bart').bookings.get(id).start === start; i++) await new Promise((r) => setTimeout(r, 20));
  assert.equal(srv.pages.get('bart').bookings.get(id).start, start + 3 * HOUR);
});

test('a booked event moved days away is rescheduled, in Rukoo or in Google, and only a fresh read can find it gone', async (t) => {
  const srv = await server(t);
  // A read of more than 60 days fails while wide is set: the fresh look reconciliation takes before cancelling.
  let wide = false;
  const fake = fakeCalendar({
    fail: ({ method, url }) => {
      if (!wide || method !== 'GET' || !/\/events$/.test(url.pathname)) return null;
      const span = Date.parse(url.searchParams.get('timeMax')) - Date.parse(url.searchParams.get('timeMin'));
      return span > 60 * DAY ? { status: 500, body: { error: { code: 500, message: 'Backend Error' } } } : null;
    }
  });
  const host = await computer(t, srv, { fake });
  host.booking.timing.reconcileAfter = 20;
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const a = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  const b = await srv.book({ handle: 'bart', type: '30min', start: start + DAY, guest: { name: 'Max Power', email: 'max@example.org' } });
  await host.booking.sync();
  const row = (id) => srv.pages.get('bart').bookings.get(id);

  // A week-view drag two days later: the 'changed' event says where the event went.
  const evA = host.calendars.event(host.booking.state().bookings.find((x) => x.id === a.id).eventId);
  await host.calendars.update(evA.id, { accountId: evA.accountId, calendarId: evA.calendarId, title: evA.title, location: evA.location, description: evA.description, allDay: false, start: start + 2 * DAY, end: start + 2 * DAY + 30 * 60000 }, { etag: evA.etag, scope: 'one' });
  // The reconciliation the move starts tells the server twice: reschedule raises the version, handled follows in a
  // request of its own. Wait for both, then for the job to end, so the reconcile() calls below don't run beside it.
  const moveHandled = () => row(a.id).version > 1 && row(a.id).handledVersion === row(a.id).version;
  for (let i = 0; i < 250 && !moveHandled(); i++) await pause();
  await host.booking.queue;
  assert.deepEqual([row(a.id).status, row(a.id).start, row(a.id).handledVersion], ['confirmed', start + 2 * DAY, row(a.id).version]);

  // Moved a week later in Google itself. While the wide read fails, nothing is concluded.
  const evB = bookingEvent(host.fake, b.id);
  evB.start = { dateTime: new Date(start + 8 * DAY).toISOString() };
  evB.end = { dateTime: new Date(start + 8 * DAY + 30 * 60000).toISOString() };
  evB.etag = '"moved"';
  host.calendars.memos.clear();
  wide = true;
  await host.booking.reconcile();
  assert.deepEqual([row(b.id).status, row(b.id).start], ['confirmed', start + DAY], "not cancelled while the calendar couldn't be read");
  wide = false;
  host.calendars.memos.clear();
  await host.booking.reconcile();
  assert.deepEqual([row(b.id).status, row(b.id).start, row(b.id).handledVersion], ['confirmed', start + 8 * DAY, row(b.id).version]);
  assert.deepEqual(host.booking.state().notices.filter((n) => n.kind === 'hostDeleted'), []);
});

test('a booking the guest moved before this computer saw it still brings a notification', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  srv.guestReschedule({ handle: 'bart', id, start: start + 2 * HOUR });
  await host.booking.sync();
  assert.equal(Date.parse(bookingEvent(host.fake, id).start.dateTime), start + 2 * HOUR);
  assert.deepEqual(host.shown.map((n) => [n.kind, n.open.start]), [['new', start + 2 * HOUR]]);
});

test('a request the guest withdraws brings a notification that says so, and leaves no notice', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, status: 'pending', guest: { name: 'Jane Doe' } });
  await host.booking.sync();
  srv.guestCancel({ handle: 'bart', id });
  await host.booking.sync();
  const last = host.shown.at(-1);
  assert.deepEqual([last.kind, last.title, last.body, last.open], ['cancelled', 'Request withdrawn', `Jane Doe withdrew the request for 30 minute meeting on ${when(start)}.`, { start }]);
  const state = host.booking.state();
  assert.deepEqual([state.requests, state.notices], [[], []]);
  const row = srv.pages.get('bart').bookings.get(id);
  assert.equal(row.handledVersion, row.version);
});

test('a minute with nothing new writes nothing and sends no state; news sends it', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const file = path.join(host.dataDir, 'booking.json');
  const sent = [];
  host.booking.on('state', (s) => sent.push(s));
  await host.booking.tick();
  const written = fs.statSync(file).mtimeMs;
  host.booking.last.poll = 0;
  await host.booking.tick();
  host.booking.last.poll = 0;
  await host.booking.tick();
  assert.equal(sent.length, 0, 'every window would redraw its calendar and Settings for it');
  assert.equal(fs.statSync(file).mtimeMs, written);

  await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10), guest: { name: 'Jane Doe' } });
  host.booking.last.poll = 0;
  await host.booking.tick();
  assert.ok(sent.length > 0);
  assert.equal(sent.at(-1).bookings.length, 1);
  const count = sent.length;
  host.booking.last.poll = 0;
  await host.booking.tick();
  assert.equal(sent.length, count);
});

test("guest details are checked after opening: a mismatch or another key leaves the booking alone, and a notice counts them", async (t) => {
  const srv = await server(t);
  let clock = Date.now();
  const host = await computer(t, srv, { now: () => clock });
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  // Sealed for another time than the booking says.
  const moved = await srv.book({ handle: 'bart', type: '30min', start, plaintext: (p) => ({ ...p, start: p.start + HOUR, end: p.end + HOUR }) });
  // A name with a line break, which the server would never have taken.
  const broken = await srv.book({ handle: 'bart', type: '30min', start: start + DAY, guest: { name: 'Jane\nDoe' } });
  // Sealed to a key this account doesn't have.
  const { newKeyring } = require('../src/main/booking/keys');
  const other = newKeyring();
  const foreign = await srv.book({ handle: 'bart', type: '30min', start: start + 2 * DAY, publicKey: other.keys[other.current].publicRaw });
  await host.booking.sync();
  for (const { id } of [moved, broken, foreign]) {
    assert.equal(bookingEvent(host.fake, id), undefined);
    assert.equal(srv.pages.get('bart').bookings.get(id).handledVersion, 0, 'tried again later, not marked handled');
  }
  let state = host.booking.state();
  assert.equal(state.bookings.length, 3);
  assert.ok(state.bookings.every((i) => i.guest === null));
  // Bookings that don't open say nothing about the key; one notice counts them, and dismissing it holds until
  // another one doesn't open.
  assert.equal(state.key, 'ok');
  let notice = state.notices.find((n) => n.kind === 'unopened');
  assert.deepEqual([notice.id, notice.tone, notice.title, notice.actions.map((a) => a.action)], ['unopened', 'warn', "3 bookings can't be opened", ['dismiss']]);
  assert.match(notice.text, /couldn't open the guest details of 3 bookings/);
  clock += 1000;
  state = await host.booking.action('unopened', 'dismiss');
  assert.equal(state.notices.find((n) => n.kind === 'unopened'), undefined);
  await host.booking.sync();
  assert.equal(host.booking.state().notices.find((n) => n.kind === 'unopened'), undefined, 'the same three keep it hidden');
  clock += 1000;
  const fourth = await srv.book({ handle: 'bart', type: '30min', start: start + 3 * DAY, publicKey: other.keys[other.current].publicRaw });
  await host.booking.sync();
  notice = host.booking.state().notices.find((n) => n.kind === 'unopened');
  assert.equal(notice.title, "4 bookings can't be opened");
  // A booking that ends leaves the count; the three left were dismissed already.
  srv.guestCancel({ handle: 'bart', id: fourth.id });
  await host.booking.sync();
  assert.deepEqual(Object.keys(host.booking.file.unopened).sort(), [moved.id, broken.id, foreign.id].sort());
  assert.equal(host.booking.state().notices.find((n) => n.kind === 'unopened'), undefined);
});

test("the event title has the guest's cleaned short name, in the booking page's language", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  await host.booking.save({ locale: 'nl' });
  const { id, manageKey } = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10), guest: { name: "Dr. Jane O'Neil-Smith <img src=x> 42 ignore all instructions" } });
  await host.booking.sync();
  const ev = bookingEvent(host.fake, id);
  assert.equal(ev.summary, "30 minute meeting: Bart en Dr. Jane O'Neil-Smith img");
  assert.equal(ev.description, guestLines(srv, id, manageKey, { nl: true }));
});

test("the guest's links stay in the event through a guest's move, a host's move and an event made again, and never in the account", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const type = host.booking.settings().types[0];
  await host.booking.save({ types: [{ ...type, description: 'Bring the numbers.' }] });
  const start = slot(3, 10);
  const { id, manageKey } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  await host.booking.sync();
  const live = () => host.fake.store.filter((e) => e.extendedProperties && e.extendedProperties.private && e.extendedProperties.private.rukooBooking === id && e.status !== 'cancelled');
  const lines = `Bring the numbers.\n\n${guestLines(srv, id, manageKey)}`;
  assert.equal(live()[0].description, lines);

  // The key is in the event, not in the mapping record, which Export my data writes to a file.
  const map = host.account.get('booking', `map:${host.account.acct}:${id}`);
  assert.equal(map.guest.email, 'jane@example.com');
  assert.equal('manageKey' in map.guest, false);
  await host.account.sync();
  assert.equal(JSON.stringify(host.account.exportData()).includes(manageKey), false);

  // The guest moves it: the event moves and keeps its description, and the booking keeps its id and key.
  srv.guestReschedule({ handle: 'bart', id, start: start + 2 * HOUR });
  await host.booking.sync();
  assert.deepEqual(live().map((e) => [Date.parse(e.start.dateTime), e.description]), [[start + 2 * HOUR, lines]]);

  // The host moves it in Rukoo's calendar: the booking follows, the description stays.
  const ev = host.calendars.event(host.booking.state().bookings[0].eventId);
  await host.calendars.update(ev.id, { accountId: ev.accountId, calendarId: ev.calendarId, title: ev.title, location: ev.location, description: ev.description, allDay: false, start: start + 3 * HOUR, end: start + 3 * HOUR + 30 * 60000 }, { etag: ev.etag, scope: 'one' });
  await host.booking.reconcile();
  assert.equal(srv.pages.get('bart').bookings.get(id).start, start + 3 * HOUR);
  assert.deepEqual(live().map((e) => e.description), [lines]);

  // Deleted in Google behind Rukoo's back, then moved by the guest, after a restart (the opened details are memory
  // only): the event is made again from the mapping record, and the key is read again from the sealed details.
  live()[0].status = 'cancelled';
  const cache = host.calendars.cache('g1');
  cache.events = cache.events.filter((e) => !e.bookingId);
  host.calendars.memos.clear();
  host.booking.opened.clear();
  srv.guestReschedule({ handle: 'bart', id, start: start + 4 * HOUR });
  await host.booking.sync();
  assert.deepEqual(live().map((e) => [Date.parse(e.start.dateTime), e.description]), [[start + 4 * HOUR, lines]]);
  assert.equal('manageKey' in host.account.get('booking', `map:${host.account.acct}:${id}`).guest, false);
});

test("a manage key or page address that fails its check leaves the guest's links out, and the booking goes through", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  for (const manageKey of ['not-a-key', `${'A'.repeat(42)}=`, 'A'.repeat(42) + '\n']) {
    const { id } = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10), guest: { name: 'Jane Doe' }, plaintext: (p) => ({ ...p, manageKey }) });
    await host.booking.sync();
    const ev = bookingEvent(host.fake, id);
    assert.ok(ev, JSON.stringify(manageKey));
    assert.equal(ev.description, '', 'no links and no "Booked via"');
    const row = srv.pages.get('bart').bookings.get(id);
    assert.deepEqual([row.status, row.handledVersion], ['confirmed', row.version]);
    srv.guestCancel({ handle: 'bart', id });
    await host.booking.sync();
  }

  // An address that isn't https, from a server that isn't on this computer.
  const other = await server(t);
  other.bookingOrigin = 'http://meet.example';
  const desk = await computer(t, other);
  const state = await desk.booking.enable(ENABLE);
  assert.equal(state.url, 'http://meet.example/bart');
  const { id } = await other.book({ handle: 'bart', type: '30min', start: slot(4, 10), guest: { name: 'Jane Doe' } });
  await desk.booking.sync();
  assert.equal(bookingEvent(desk.fake, id).description, '');
  assert.equal(other.pages.get('bart').bookings.get(id).status, 'confirmed');
});

test('several lengths are saved, published and reach the other computer; a booking at another length gets its end and title', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const type = host.booking.settings().types[0];
  await assert.rejects(host.booking.save({ types: [{ ...type, durations: [15, 60] }] }), (err) => err.code === 'invalid' && err.field === 'types[0].durations');
  await host.booking.save({ types: [{ ...type, durations: [60, 15, 30] }] });
  assert.deepEqual(host.booking.settings().types[0].durations, [15, 30, 60]);
  const published = srv.pages.get('bart').profile.types[0];
  assert.deepEqual([published.duration, published.durations, published.slotInterval], [30, [15, 30, 60], null]);

  // Another computer of the same account has them from the account's settings record.
  const laptop = await computer(t, srv, { signUp: false, name: 'laptop' });
  await laptop.account.sync();
  const synced = laptop.booking.settings().types[0];
  assert.deepEqual([synced.duration, synced.durations], [30, [15, 30, 60]]);

  // A guest picks an hour: the event ends an hour later, and Rukoo's own title says so, in the event, the bookings
  // list and the notification.
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, duration: 60, guest: { name: 'Jane Doe' } });
  const state = await host.booking.sync();
  const ev = bookingEvent(host.fake, id);
  assert.equal(ev.summary, '60 minute meeting: Bart and Jane Doe');
  assert.equal(Date.parse(ev.end.dateTime) - Date.parse(ev.start.dateTime), HOUR);
  assert.equal(state.bookings.find((b) => b.id === id).typeTitle, '60 minute meeting');
  assert.match(host.shown.at(-1).body, /^Jane Doe booked 60 minute meeting for /);

  // A title the host wrote stays as it is at every length.
  await host.booking.save({ types: [{ ...host.booking.settings().types[0], title: 'Intro call' }] });
  const other = await srv.book({ handle: 'bart', type: '30min', start: slot(4, 11), duration: 15, guest: { name: 'Ann Lee' } });
  await host.booking.sync();
  assert.equal(bookingEvent(host.fake, other.id).summary, 'Intro call: Bart and Ann Lee');
});

test('a settings record an older app stored just under the size limit keeps the page publishing', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const current = host.booking.settings();
  // As an app from before several lengths wrote it: ten types without durations, 12,231 of the 12,288 bytes.
  const types = Array.from({ length: 10 }, (_, i) => {
    const { durations, ...type } = current.types[0];
    return { ...type, slug: `type-${i}`, description: 'é'.repeat(450) };
  });
  const record = { ...current, intro: '', types };
  record.intro = 'a'.repeat(12231 - Buffer.byteLength(JSON.stringify(record), 'utf8'));
  host.account.put('booking', `settings:${host.account.acct}`, record);
  assert.deepEqual(host.booking.settings(), record);
  await host.booking.publish({ force: true });
  assert.equal(srv.pages.get('bart').profile.types.length, 10);
  assert.equal(srv.pages.get('bart').profile.intro, record.intro);
});

test('settings are checked as a whole; a refused field keeps the record as it was', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const type = host.booking.settings().types[0];
  await assert.rejects(host.booking.save({ types: [{ ...type, slug: 'api' }] }), (err) => err.booking && err.code === 'invalid' && err.field === 'types[0].slug' && /reserved/.test(err.message));
  await assert.rejects(host.booking.save({ types: [type, { ...type, title: 'Again' }] }), (err) => err.code === 'invalid' && err.field === 'types[1].slug');
  await assert.rejects(host.booking.save({ hours: { custom: true, weekly: [[], [['17:00', '09:00']], [], [], [], [], []] } }), (err) => err.code === 'invalid' && err.field === 'hours.weekly');
  await assert.rejects(host.booking.save({ types: [{ ...type, location: { kind: 'link', label: '', value: 'http://example.com', public: true } }] }), (err) => err.field === 'types[0].location.value');
  const long = 'ü'.repeat(500);
  const many = Array.from({ length: 10 }, (_, i) => ({ ...type, slug: `type-${i}`, description: long }));
  await assert.rejects(host.booking.save({ intro: 'ü'.repeat(1000), types: many }), (err) => err.code === 'too_large');
  assert.equal(host.booking.settings().types.length, 1);

  // Date overrides merge per date; null takes one away.
  await host.booking.save({ overrides: { '2099-01-05': [], '2099-01-06': [['10:00', '12:00']] } });
  await host.booking.save({ overrides: { '2099-01-05': null } });
  assert.deepEqual(host.booking.settings().overrides, { '2099-01-06': [['10:00', '12:00']] });
  assert.equal(srv.pages.get('bart').profile.intro, '');
  await host.booking.save({ intro: 'Happy to talk.' });
  assert.equal(srv.pages.get('bart').profile.intro, 'Happy to talk.', 'saving republishes');
});

test('a new handle renames the page first; refused while bookings lie ahead', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10) });
  await assert.rejects(host.booking.save({ handle: 'bart-vdm' }), (err) => err.code === 'has_bookings');
  assert.equal(host.booking.settings().handle, 'bart');
  srv.guestCancel({ handle: 'bart', id });
  const state = await host.booking.save({ handle: 'bart-vdm' });
  assert.equal(state.handle, 'bart-vdm');
  assert.equal(state.settings.url, `${srv.url}/meet/bart-vdm`);
  assert.equal(srv.pages.get('bart-vdm').profile.name, 'Bart');
  assert.deepEqual(await host.booking.checkHandle('bart-vdm'), { handle: 'bart-vdm', available: false, reason: 'yours' });
  assert.deepEqual(await host.booking.checkHandle('Bart'), { handle: 'Bart', available: false, reason: 'invalid' });
  assert.deepEqual(await host.booking.checkHandle('meet'), { handle: 'meet', available: false, reason: 'reserved' });
  assert.deepEqual(await host.booking.checkHandle('someone'), { handle: 'someone', available: true, reason: null });
});

test("a calendar that can't be read is published from its last good busy time for 12 hours, then every type waits for the host", async (t) => {
  const srv = await server(t);
  let failing = false;
  const busyStart = slot(2, 11);
  const fake = fakeCalendar({
    events: [{ id: 'busy1', summary: 'Review', start: { dateTime: new Date(busyStart).toISOString() }, end: { dateTime: new Date(busyStart + HOUR).toISOString() } }],
    fail: ({ method, url }) => (failing && method === 'GET' && /\/events$/.test(url.pathname) ? { status: 500, body: { error: { code: 500, message: 'Backend Error' } } } : null)
  });
  let clock = Date.now();
  const host = await computer(t, srv, { fake, now: () => clock });
  await host.booking.enable(ENABLE);
  const busy = srv.pages.get('bart').busy;
  assert.ok(busy.some(([s]) => s === busyStart));

  failing = true;
  host.calendars.memos.clear();
  clock += 2 * HOUR;
  await host.booking.publish({ force: true });
  let page = srv.pages.get('bart');
  assert.ok(page.busy.some(([s]) => s === busyStart), 'the last good busy time is used');
  assert.equal(page.profile.types[0].confirm, false);
  let state = host.booking.state();
  assert.deepEqual(state.publishing.unchecked.map((u) => u.email), ['me@example.com']);
  assert.equal(state.notices.find((n) => n.kind === 'unchecked'), undefined);

  clock += 11 * HOUR;
  host.calendars.memos.clear();
  await host.booking.publish({ force: true });
  page = srv.pages.get('bart');
  assert.equal(page.profile.types[0].confirm, true, 'after 12 hours every type needs the host');
  state = host.booking.state();
  const notice = state.notices.find((n) => n.kind === 'unchecked');
  assert.deepEqual([notice.id, notice.accountId, notice.email, notice.actions.map((a) => a.action)], ['unchecked:g1', 'g1', 'me@example.com', ['reconnect']]);
  assert.match(notice.title, /me@example\.com/);

  failing = false;
  host.calendars.memos.clear();
  await host.booking.publish({ force: true });
  assert.equal(srv.pages.get('bart').profile.types[0].confirm, false);
  assert.equal(host.booking.state().notices.find((n) => n.kind === 'unchecked'), undefined);
});

test('turning the page off deletes it on the server and the mapping records, and keeps the settings for next time', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  await host.booking.save({ intro: 'Happy to talk.' });
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start });
  await host.booking.sync();
  const acct = host.account.acct;
  assert.ok(host.account.get('booking', `map:${acct}:${id}`));
  // A booking's own event isn't published as busy: the server keeps that time itself.
  const meeting = ([s, e]) => s <= start && e >= start + 30 * 60000;
  await host.booking.publish({ force: true });
  assert.ok(!srv.pages.get('bart').busy.some(meeting));

  let state = await host.booking.disable();
  assert.equal(state.status, 'off');
  assert.equal(state.bookings.length, 0);
  assert.equal(host.booking.url(), null);
  assert.equal(host.booking.link(), null);
  assert.equal(host.booking.hours(), null);
  assert.equal(host.account.get('booking', `map:${acct}:${id}`), undefined);
  assert.equal(host.account.get('booking', `settings:${acct}`).intro, 'Happy to talk.');
  assert.equal(srv.pages.get('bart').bookings.size, 0);
  assert.ok(bookingEvent(host.fake, id), 'the event stays in the calendar as a plain event');

  state = await host.booking.enable(ENABLE);
  assert.equal(state.status, 'on');
  assert.equal(state.settings.intro, 'Happy to talk.');
  assert.equal(srv.pages.get('bart').profile.intro, 'Happy to talk.');
  assert.ok(srv.pages.get('bart').busy.some(meeting), 'the meeting stayed in the calendar without its booking, so now it is busy');
  assert.deepEqual(host.booking.link(), { url: `${srv.url}/meet/bart`, types: [{ slug: '30min', title: '30 minute meeting', duration: 30, url: `${srv.url}/meet/bart/30min` }] });
  assert.equal(host.booking.hours().timeZone, 'Europe/Amsterdam');
  assert.equal(host.booking.locale(), 'en');
});

test('an event someone else made that carries a booking id leaves the booking alone', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe', email: 'jane@example.com' } });
  await host.booking.sync();
  // Two hours later, with the guest declining: as a calendar that kept the id on a copied or foreign event shows it.
  host.fake.store.push({
    id: 'other',
    calendarId: 'me@example.com',
    etag: '"1"',
    status: 'confirmed',
    summary: 'Something',
    organizer: { email: 'mallory@example.net' },
    attendees: [
      { email: 'mallory@example.net', organizer: true, responseStatus: 'accepted' },
      { email: 'jane@example.com', responseStatus: 'declined' },
      { email: 'me@example.com', self: true, responseStatus: 'needsAction' }
    ],
    extendedProperties: { private: { rukooBooking: id } },
    start: { dateTime: new Date(start + 2 * HOUR).toISOString() },
    end: { dateTime: new Date(start + 2 * HOUR + 30 * 60000).toISOString() }
  });
  await host.calendars.syncAccount('g1');
  host.calendars.memos.clear();
  await host.booking.reconcile();
  const row = srv.pages.get('bart').bookings.get(id);
  assert.deepEqual([row.status, row.start], ['confirmed', start], "the mapped event is the booking's");
  assert.deepEqual(host.booking.state().notices, []);
});

test("an invitation from someone else with the booking's uid doesn't stop the booking getting its own event", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv, { fake: fakeCalendar({ insertUid: 'keep' }) });
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe', email: 'jane@example.com' } });
  const row = srv.pages.get('bart').bookings.get(id);
  host.fake.store.push({
    id: 'theirs',
    calendarId: 'me@example.com',
    etag: '"1"',
    iCalUID: row.uid,
    status: 'confirmed',
    summary: 'Confirm at https://evil.example',
    organizer: { email: 'jane@example.com' },
    attendees: [{ email: 'jane@example.com', organizer: true, responseStatus: 'accepted' }, { email: 'me@example.com', self: true, responseStatus: 'needsAction' }],
    start: { dateTime: new Date(start).toISOString() },
    end: { dateTime: new Date(start + 30 * 60000).toISOString() }
  });
  await host.booking.sync();
  const map = host.account.get('booking', `map:${host.account.acct}:${id}`);
  assert.ok(map && map.eventId !== 'theirs', 'an event of its own');
  assert.ok(bookingEvent(host.fake, id));
  assert.equal(srv.pages.get('bart').bookings.get(id).handledVersion, 1);
  assert.equal(host.fake.store.find((e) => e.id === 'theirs').summary, 'Confirm at https://evil.example', 'the invitation is left as it is');
});

test('a destination calendar that leaves the list cancels no bookings; a guest move waits, and the booking device says so', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable({ ...ENABLE, destination: calendarKey('g1', 'team@example.com') });
  const start = slot(3, 10);
  const a = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  const b = await srv.book({ handle: 'bart', type: '30min', start: slot(4, 11), guest: { name: 'Max Power', email: 'max@example.org' } });
  await host.booking.sync();
  await host.calendars.ensure('g1', Date.now(), Date.now() + 7 * DAY);
  // Hidden from the list in Google: calendarList leaves it out; its events stay.
  host.fake.calendars.splice(host.fake.calendars.findIndex((c) => c.id === 'team@example.com'), 1);
  await host.calendars.syncAccount('g1');
  assert.ok(!host.calendars.list()[0].calendars.some((c) => c.id === 'team@example.com'));
  host.calendars.memos.clear();
  await host.booking.reconcile();
  for (const { id } of [a, b]) assert.equal(srv.pages.get('bart').bookings.get(id).status, 'confirmed');
  assert.deepEqual(host.booking.state().notices.map((n) => n.kind), ['destination']);
  assert.match(host.booking.state().notices[0].text, /isn't on this computer/);

  srv.guestReschedule({ handle: 'bart', id: a.id, start: start + 2 * HOUR });
  await host.booking.sync();
  assert.ok(host.account.get('booking', `map:${host.account.acct}:${a.id}`), 'the mapping record stays');
  assert.equal(srv.pages.get('bart').bookings.get(a.id).handledVersion, 1, 'tried again later');
  assert.equal(Date.parse(bookingEvent(host.fake, a.id).start.dateTime), start);
});

test("cancelling while the guest's move isn't processed yet cancels the booking as it is now, for good", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const start = slot(3, 10);
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  await host.booking.sync();
  srv.guestReschedule({ handle: 'bart', id, start: start + 2 * HOUR });
  await host.booking.action(id, 'cancel', { notify: true });
  const row = srv.pages.get('bart').bookings.get(id);
  assert.deepEqual([row.status, row.cancelledBy], ['cancelled', 'host']);
  const made = inserted(host.fake).length;
  await host.booking.sync();
  assert.equal(inserted(host.fake).length, made, 'no event again');
  assert.equal(bookingEvent(host.fake, id).status, 'cancelled');
  assert.ok(!host.fake.requests.some((r) => r.method === 'PATCH' && r.body && r.body.status === 'confirmed'));
});

test('a booking whose meeting is over is not cancelled, and its guest is not told', async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const start = Math.floor((Date.now() - 2 * DAY) / HOUR) * HOUR;
  const { id } = await srv.book({ handle: 'bart', type: '30min', start, guest: { name: 'Jane Doe' } });
  await host.booking.sync();
  assert.ok(bookingEvent(host.fake, id));
  await assert.rejects(host.booking.action(id, 'cancel', { notify: true }), (err) => err.code === 'notFound');
  assert.equal(host.fake.requests.filter((r) => r.method === 'DELETE').length, 0);
  assert.equal(srv.pages.get('bart').bookings.get(id).status, 'confirmed');
});

test("a rename handles the old handle's waiting cancellation first, and is refused while it can't be", async (t) => {
  const srv = await server(t);
  let failing = false;
  const fake = fakeCalendar({ fail: ({ method }) => (failing && method === 'DELETE' ? { status: 500, body: { error: { code: 500, message: 'down' } } } : null) });
  const host = await computer(t, srv, { fake });
  await host.booking.enable(ENABLE);
  const first = await srv.book({ handle: 'bart', type: '30min', start: slot(3, 10), guest: { name: 'Jane Doe' } });
  const second = await srv.book({ handle: 'bart', type: '30min', start: slot(4, 10), guest: { name: 'Max Power', email: 'max@example.org' } });
  await host.booking.sync();
  srv.guestCancel({ handle: 'bart', id: first.id });
  srv.guestCancel({ handle: 'bart', id: second.id });
  // The calendar can't take the cancellations out: the rename waits.
  failing = true;
  await assert.rejects(host.booking.save({ handle: 'bart-vdm' }), (err) => err.code === 'has_bookings');
  assert.equal(host.booking.settings().handle, 'bart');
  failing = false;
  const state = await host.booking.save({ handle: 'bart-vdm' });
  assert.equal(state.handle, 'bart-vdm');
  for (const { id } of [first, second]) {
    assert.equal(bookingEvent(host.fake, id).status, 'cancelled');
    const row = srv.pages.get('bart').bookings.get(id);
    assert.equal(row.handledVersion, row.version);
  }
  assert.deepEqual(host.shown.map((n) => n.kind), ['new', 'new', 'cancelled', 'cancelled']);
});

test("the booking link leaves out hidden meeting types; only Google's no-mail notice is about mail", async (t) => {
  const srv = await server(t);
  const host = await computer(t, srv);
  await host.booking.enable(ENABLE);
  const visible = host.booking.settings().types[0];
  await host.booking.save({ types: [visible, { ...visible, slug: 'board', title: 'Board call', hidden: true }] });
  assert.deepEqual(host.booking.link().types.map((x) => x.slug), ['30min']);

  // A calendar other than Google's sends what its server sends: no "no email sent" notice for it.
  const row = { id: 'Kq0bAAAAAAAAAAAAAAAAAA', type: '30min', start: slot(3, 10) };
  for (const kind of ['caldav', 'demo']) {
    host.booking.resolve = () => ({ kind });
    host.booking.budgetNotice(row, { name: 'Jane Doe' }, host.booking.settings());
  }
  assert.deepEqual(host.booking.file.notices, []);
  host.booking.resolve = () => ({ kind: 'google' });
  host.booking.budgetNotice(row, { name: 'Jane Doe' }, host.booking.settings());
  assert.deepEqual(host.booking.file.notices.map((n) => n.kind), ['mailBudget']);
});
