'use strict';

// A Rukoo account server in memory, for tests of the desktop app. It follows server/PROTOCOL.md closely enough for
// the app's side; the real server in server/ has its own tests. Start one with startAccountServer().
//
// Booking pages (PROTOCOL.md, Booking pages): the owner routes under /v1/booking, plus helpers that play the guest's
// side: book() seals the guest's details to the page's key with the server's own sealing code (server/src/seal.js),
// guestCancel() and guestReschedule() change a booking as the public routes would.
const http = require('http');
const path = require('path');
const crypto = require('crypto');
const zlib = require('zlib');
const { pathToFileURL } = require('url');

const sha = (b64) => crypto.createHash('sha256').update(Buffer.from(b64, 'base64url')).digest('base64url');
const token = () => crypto.randomBytes(32).toString('base64url');
const DEFAULT_KDF = { alg: 'argon2id', m: 65536, t: 3, p: 1 };

const MINUTE = 60000;
const DAY = 86400000;
const QUARTER = 15 * MINUTE;
const HANDLE = /^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){2,31}$/;
const SLUG = /^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){0,39}$/;
const BOOKING_ID = /^[A-Za-z0-9_-]{22}$/;
// The desktop's list, which test/booking-rules.test.js holds to the server's (server/src/input.js).
const { RESERVED_HANDLES } = require('../../src/main/booking/rules');
const ACTIONS = ['handled', 'confirm', 'decline', 'cancel', 'reschedule'];
const REASONS = ['host_cancelled', 'host_deleted', 'guest_declined'];
const ACTIVE = ['pending', 'confirmed'];
const PENDING_HOLD = 48 * 60 * MINUTE;
const STALE = 24 * 60 * MINUTE;
const HANDLE_CHANGES = 3;

// The Worker's sealing module is ESM; it is loaded once, when the first booking is made.
let sealing = null;
const sealModule = () => (sealing ||= import(pathToFileURL(path.join(__dirname, '..', '..', 'server', 'src', 'seal.js')).href));

const isObject = (v) => Boolean(v && typeof v === 'object' && !Array.isArray(v));
const invalid = (field) => [400, { error: 'invalid', message: `${field} was refused.`, field }];
const refuse = (error, extra = {}) => [error === 'not_found' ? 404 : 409, { error, message: error, ...extra }];

// The host's day in the page's zone, for the daily counts.
function dayIn(ms, timeZone) {
  try {
    return new Intl.DateTimeFormat('en-CA', { timeZone, year: 'numeric', month: '2-digit', day: '2-digit' }).format(ms);
  } catch (_) {
    return new Date(ms).toISOString().slice(0, 10);
  }
}

// Sorted, non-overlapping [start, end] pairs inside [from, to]; with align, on that grid. The field name when not.
function badIntervals(list, field, { max, from, to, align = 0 }) {
  if (!Array.isArray(list) || list.length > max) return field;
  let last = -Infinity;
  for (let i = 0; i < list.length; i++) {
    const p = list[i];
    if (!Array.isArray(p) || p.length !== 2 || !p.every(Number.isSafeInteger) || p[0] >= p[1] || p[0] < last) return `${field}[${i}]`;
    if (p[0] < from || p[1] > to) return `${field}[${i}]`;
    if (align && (p[0] % align !== 0 || p[1] % align !== 0)) return `${field}[${i}]`;
    last = p[1];
  }
  return null;
}

// What a publish must look like, field by field, as far as the desktop's side matters. Returns the refused field.
function badPublish(body, now) {
  if (typeof body.enabled !== 'boolean') return 'enabled';
  if (body.takeover !== undefined && typeof body.takeover !== 'boolean') return 'takeover';
  const p = body.profile;
  if (!isObject(p)) return 'profile';
  if (typeof p.name !== 'string' || !p.name.trim() || [...p.name].length > 100) return 'profile.name';
  if (p.intro !== undefined && (typeof p.intro !== 'string' || [...p.intro].length > 1000)) return 'profile.intro';
  try {
    if (typeof p.timeZone !== 'string') return 'profile.timeZone';
    new Intl.DateTimeFormat('en', { timeZone: p.timeZone });
  } catch (_) {
    return 'profile.timeZone';
  }
  if (p.locale !== 'en' && p.locale !== 'nl') return 'profile.locale';
  const publicKey = typeof p.publicKey === 'string' ? Buffer.from(p.publicKey, 'base64url') : Buffer.alloc(0);
  if (publicKey.length !== 65 || publicKey[0] !== 4) return 'profile.publicKey';
  if (p.keyId !== crypto.createHash('sha256').update(publicKey).digest().subarray(0, 8).toString('base64url')) return 'profile.keyId';
  if (p.contact !== null && p.contact !== undefined && (typeof p.contact !== 'string' || [...p.contact].length > 200)) return 'profile.contact';
  if (!Number.isInteger(p.dailyCap) || p.dailyCap < 1 || p.dailyCap > 50) return 'profile.dailyCap';
  if (!Array.isArray(p.types) || p.types.length < 1 || p.types.length > 10) return 'profile.types';
  const slugs = new Set();
  for (let i = 0; i < p.types.length; i++) {
    const t = p.types[i];
    const f = `profile.types[${i}]`;
    if (!isObject(t) || typeof t.slug !== 'string' || !SLUG.test(t.slug) || slugs.has(t.slug)) return `${f}.slug`;
    slugs.add(t.slug);
    if (typeof t.title !== 'string' || !t.title.trim() || [...t.title].length > 100) return `${f}.title`;
    if (!Number.isInteger(t.duration) || t.duration < 5 || t.duration > 480 || t.duration % 5) return `${f}.duration`;
    // durations: optional, 1 to 10 different lengths with duration among them; slotInterval null follows the length.
    const lengths = t.durations === undefined || t.durations === null ? [t.duration] : t.durations;
    if (
      !Array.isArray(lengths) ||
      lengths.length < 1 ||
      lengths.length > 10 ||
      !lengths.every((m) => Number.isInteger(m) && m >= 5 && m <= 480 && m % 5 === 0) ||
      new Set(lengths).size !== lengths.length ||
      !lengths.includes(t.duration)
    ) {
      return `${f}.durations`;
    }
    if (!isObject(t.location) || !['googleMeet', 'phone', 'inPerson', 'link', 'none'].includes(t.location.kind)) return `${f}.location`;
    for (const [k, lo, hi] of [['bufferBefore', 0, 240], ['bufferAfter', 0, 240], ['minNotice', 0, 43200], ['horizon', 1, 90], ['slotInterval', 5, 480]]) {
      if (k === 'slotInterval' && t[k] === null) continue;
      if (!Number.isInteger(t[k]) || t[k] < lo || t[k] > hi) return `${f}.${k}`;
    }
    if (t.dailyLimit !== null && (!Number.isInteger(t.dailyLimit) || t.dailyLimit < 1 || t.dailyLimit > 50)) return `${f}.dailyLimit`;
    if (typeof t.confirm !== 'boolean' || typeof t.hidden !== 'boolean') return `${f}.confirm`;
    if (t.schedule !== 'default') return `${f}.schedule`;
  }
  if (!isObject(body.windows) || Object.keys(body.windows).join() !== 'default') return 'windows';
  return (
    badIntervals(body.windows.default, 'windows.default', { max: 2000, from: now - 2 * DAY, to: now + 92 * DAY }) ||
    badIntervals(body.busy, 'busy', { max: 5000, from: now - 3 * DAY, to: now + 93 * DAY, align: QUARTER })
  );
}

// passwordCheck: false plays a server from before POST /v1/password/check, which answers 404 there.
function startAccountServer({ signups = 'open', invites = [], limit = 1000, jurisdiction = 'eu', kdf = DEFAULT_KDF, features = ['booking'], passwordCheck = true } = {}) {
  const accounts = new Map();
  const sessions = new Map();
  const calls = [];
  // Booking pages by handle: { owner, handle, enabled, deviceId, profile, windows, busy, publishedAt, seq, retiredAt,
  // bookings: Map(id -> row), counts: { day, new, mail } }.
  const pages = new Map();
  // Set gzip on the started server to answer pulls compressed whatever the device asked for, as a proxy that
  // compresses everything would. mailMax is the booking pages' daily mail budget, now the server's clock and
  // bookingOrigin the server's BOOKING_ORIGIN: pages live at its root, and without it under /meet on the server's own
  // address. fail({ route, body }) may answer a request instead of the server: [status, json] or null.
  const control = { gzip: false, mailMax: 10, now: () => Date.now(), bookingOrigin: null, fail: null };

  const info = () => ({
    protocol: 1,
    version: '1.0.0-test',
    operator: { name: 'Test operator', url: 'https://example.test', contact: 'ops@example.test', privacy: 'https://example.test/privacy' },
    hosting: { provider: 'Cloudflare', company: 'Cloudflare, Inc.', country: 'US', jurisdiction },
    signups,
    limits: { records: 250000, bytes: 104857600, recordBytes: 16384, batch: 500 },
    features: [...features]
  });
  const accountView = (acc, current) => ({
    email: acc.email,
    createdAt: acc.createdAt,
    kdf: acc.kdf,
    usage: { records: [...acc.records.values()].filter((r) => r.data).length, bytes: [...acc.records.values()].reduce((n, r) => n + (r.data ? Buffer.from(r.data, 'base64url').length : 0), 0) },
    limits: info().limits,
    devices: [...sessions.entries()]
      .filter(([, s]) => s.email === acc.email)
      .map(([tok, s]) => ({ id: s.id, name: s.name, platform: s.platform, createdAt: s.createdAt, lastSeenAt: s.createdAt, current: tok === current }))
  });
  const newSession = (acc, device) => {
    const tok = token();
    sessions.set(tok, { email: acc.email, id: crypto.randomBytes(8).toString('hex'), name: device.name, platform: device.platform, createdAt: Date.now() });
    return tok;
  };
  const endSessions = (email, keep) => {
    for (const [tok, s] of sessions) if (s.email === email && tok !== keep) sessions.delete(tok);
  };
  // The signed-in calls that ask for the password again count wrong ones per session: the tenth within 15 minutes
  // locks that session's checks for 15 minutes, also for the right password (PROTOCOL.md, POST /v1/password). The
  // answer to send instead, or null when authKey is right.
  const wrongPassword = (acc, session, authKey) => {
    const now = control.now();
    if ((session.lockedUntil || 0) > now) return [429, { error: 'locked' }];
    if (acc.auth === sha(authKey)) {
      session.failures = 0;
      return null;
    }
    const fresh = now - (session.failedAt || 0) >= QUARTER;
    session.failures = fresh ? 1 : session.failures + 1;
    if (fresh) session.failedAt = now;
    if (session.failures < 10) return [403, { error: 'wrong_password' }];
    Object.assign(session, { failures: 0, failedAt: 0, lockedUntil: now + QUARTER });
    return [429, { error: 'locked' }];
  };

  // ---- booking pages ----

  const base = () => control.bookingOrigin || `${control.url}/meet`;
  const pageUrl = (handle) => `${base()}/${handle}`;
  const deviceName = (id) => {
    const s = [...sessions.values()].find((x) => x.id === id);
    return s ? s.name : null;
  };
  const counts = (page, now = control.now()) => {
    const day = dayIn(now, page.profile ? page.profile.timeZone : 'UTC');
    if (!page.counts || page.counts.day !== day) page.counts = { day, new: 0, mail: 0 };
    return page.counts;
  };
  // A booking row as GET /v1/booking/bookings sends it.
  const ownerRow = (r) => ({ ...r });
  // Every change that raises a booking's version takes the page's next seq. Sealed details go once it's over.
  const change = (page, row, fields) => {
    Object.assign(row, fields, { version: row.version + 1, seq: ++page.seq, updatedAt: control.now() });
    if (!ACTIVE.includes(row.status)) row.sealed = null;
  };
  // Requests nobody answered lapse at expiresAt, noticed by whatever reads next.
  const lapse = (page) => {
    const now = control.now();
    for (const row of page.bookings.values()) if (row.status === 'pending' && row.expiresAt !== null && row.expiresAt <= now) change(page, row, { status: 'expired', notify: false });
  };
  // The account's page: null without one, 'lost' once another account has its handle.
  const pageOf = (acc) => {
    if (!acc.booking.handle) return null;
    const page = pages.get(acc.booking.handle);
    if (!page) return null;
    return page.owner === acc.email ? page : 'lost';
  };
  const ownerJson = (acc, session, page) => {
    const handleChanges = { used: acc.booking.changes.filter((t) => t > control.now() - 30 * DAY).length, max: HANDLE_CHANGES };
    if (!page) {
      const handle = acc.booking.handle;
      return { handle, url: handle ? pageUrl(handle) : null, base: base(), enabled: false, device: null, publishedAt: null, keyId: null, seq: 0, pending: 0, upcoming: 0, today: null, handleChanges };
    }
    lapse(page);
    const now = control.now();
    const rows = [...page.bookings.values()];
    const c = counts(page);
    return {
      handle: page.handle,
      url: pageUrl(page.handle),
      base: base(),
      enabled: page.enabled && !page.retiredAt,
      device: page.deviceId ? { id: page.deviceId, name: deviceName(page.deviceId), current: page.deviceId === session.id } : null,
      publishedAt: page.publishedAt,
      keyId: page.profile ? page.profile.keyId : null,
      seq: page.seq,
      pending: rows.filter((r) => r.status === 'pending').length,
      upcoming: rows.filter((r) => r.status === 'confirmed' && r.end > now).length,
      today: page.profile ? { bookings: c.new, cap: page.profile.dailyCap, mail: c.mail, mailMax: control.mailMax } : null,
      handleChanges
    };
  };

  // A page turned off from another session than the booking device's keeps that device in offDevice: it doesn't know
  // yet and may still publish what it had. Its publishes and renames without takeover get other_device until its
  // session reads GET /v1/booking (PROTOCOL.md, One booking device).
  const unaware = (page, session, takeover) => !takeover && Boolean(page.offDevice) && page.offDevice === session.id;

  const booking = {
    'GET /v1/booking': ({ acc, session }) => {
      const page = pageOf(acc);
      if (page === 'lost') return refuse('handle_lost');
      if (page && page.offDevice === session.id) page.offDevice = null;
      return [200, ownerJson(acc, session, page)];
    },
    'PUT /v1/booking': ({ acc, session, body }) => {
      const now = control.now();
      if (typeof body.handle !== 'string' || !HANDLE.test(body.handle)) return invalid('handle');
      if (RESERVED_HANDLES.has(body.handle)) return refuse('handle_reserved');
      const bad = badPublish(body, now);
      if (bad) return invalid(bad);
      const takeover = body.takeover === true;
      const current = acc.booking.handle;
      const target = pages.get(body.handle);
      // A deleted account's handle goes back only with the key the deleting device sent (PROTOCOL.md, Handles).
      if (target && target.owner !== acc.email && typeof body.reclaim === 'string' && target.owner === `deleted:${sha(body.reclaim)}`) target.owner = acc.email;
      if (target && target.owner !== acc.email) return refuse(body.handle === current ? 'handle_lost' : target.retiredAt ? 'handle_reserved' : 'handle_taken');
      const renaming = Boolean(current) && current !== body.handle;
      const old = renaming ? pages.get(current) : null;
      if (renaming) {
        if (acc.booking.changes.filter((t) => t > now - 30 * DAY).length >= HANDLE_CHANGES) return refuse('handle_changes');
        if (old && old.owner === acc.email) {
          if (old.deviceId && old.deviceId !== session.id && !takeover && !old.retiredAt) return refuse('other_device');
          if (unaware(old, session, takeover)) return refuse('other_device');
          lapse(old);
          if ([...old.bookings.values()].some((r) => r.end > now && (ACTIVE.includes(r.status) || r.version > (r.handledVersion || 0)))) return refuse('has_bookings');
        }
      }
      const page = target || { owner: acc.email, handle: body.handle, enabled: false, deviceId: null, offDevice: null, profile: null, windows: null, busy: null, publishedAt: null, seq: 0, retiredAt: null, bookings: new Map(), counts: null };
      if (page.deviceId && page.deviceId !== session.id && !takeover && !page.retiredAt) return refuse('other_device');
      if (unaware(page, session, takeover)) return refuse('other_device');
      Object.assign(page, { profile: body.profile, windows: body.windows, busy: body.busy, publishedAt: now, enabled: body.enabled, deviceId: session.id, offDevice: null, retiredAt: null });
      pages.set(page.handle, page);
      if (old && old.owner === acc.email) Object.assign(old, { retiredAt: now, enabled: false, profile: null, windows: null, busy: null, deviceId: null, movedTo: body.handle });
      if (body.handle !== current) {
        if (current) acc.booking.changes.push(now);
        acc.booking.handle = body.handle;
      }
      return [200, ownerJson(acc, session, page)];
    },
    'DELETE /v1/booking': ({ acc, session }) => {
      // A session whose password checks are locked deletes nothing (PROTOCOL.md, DELETE /v1/booking).
      if ((session.lockedUntil || 0) > control.now()) return [429, { error: 'locked' }];
      const page = pageOf(acc);
      if (page && page !== 'lost') {
        // The booking device, or the one an earlier turn-off left unaware; never the session that turns it off now.
        const off = page.deviceId || page.offDevice || null;
        const offDevice = off !== session.id ? off : null;
        Object.assign(page, { profile: null, windows: null, busy: null, publishedAt: null, enabled: false, deviceId: null, offDevice, seq: 0, counts: null, retiredAt: page.retiredAt || control.now() });
        page.bookings.clear();
      }
      return [204];
    },
    'GET /v1/booking/handles': ({ acc, rest }) => {
      let handle = rest;
      try {
        handle = decodeURIComponent(rest);
      } catch (_) {
        // As sent.
      }
      handle = [...handle].slice(0, 64).join('');
      const answer = (available, reason) => [200, { handle, available, reason }];
      if (!HANDLE.test(handle)) return answer(false, 'invalid');
      if (RESERVED_HANDLES.has(handle)) return answer(false, 'reserved');
      if (handle === acc.booking.handle) return answer(false, 'yours');
      const page = pages.get(handle);
      if (page && page.owner !== acc.email) return answer(false, page.retiredAt ? 'reserved' : 'taken');
      return answer(true, null);
    },
    'GET /v1/booking/bookings': ({ acc, url }) => {
      const since = Number(url.searchParams.get('since') || 0);
      const max = Math.min(Number(url.searchParams.get('limit')) || 500, 500);
      if (!Number.isSafeInteger(since) || since < 0) return invalid('since');
      const page = pageOf(acc);
      if (page === 'lost') return refuse('handle_lost');
      if (!page) return [200, { bookings: [], seq: 0, more: false }];
      lapse(page);
      const rows = [...page.bookings.values()].filter((r) => r.seq > since).sort((a, b) => a.seq - b.seq);
      return [200, { bookings: rows.slice(0, max).map(ownerRow), seq: page.seq, more: rows.length > max }];
    },
    'POST /v1/booking/bookings': ({ acc, session, body, rest }) => {
      if (!BOOKING_ID.test(rest)) return refuse('not_found');
      if (!ACTIONS.includes(body.action)) return invalid('action');
      if (!Number.isInteger(body.version) || body.version < 1) return invalid('version');
      const page = pageOf(acc);
      if (!page || page === 'lost') return refuse('handle_lost');
      if (page.deviceId && page.deviceId !== session.id) return refuse('other_device');
      lapse(page);
      const row = page.bookings.get(rest);
      if (!row) return refuse('not_found');
      if (body.action === 'handled') {
        if (body.version > row.version) return invalid('version');
        // The uid the host's calendar gave the event, when it isn't the one the booking was made with, so the
        // guest's .ics and the host's invitation are one event in the guest's calendar.
        if (body.uid !== undefined && body.uid !== null) {
          if (typeof body.uid !== 'string' || !/^[\x21-\x7e]{1,255}$/.test(body.uid)) return invalid('uid');
          row.uid = body.uid;
        }
        row.handledVersion = Math.max(row.handledVersion, body.version);
        return [200, { booking: ownerRow(row) }];
      }
      const stale = () => refuse('stale_version', { booking: ownerRow(row) });
      if (body.version !== row.version) return stale();
      const message = typeof body.message === 'string' && body.message.trim() ? body.message.trim().slice(0, 500) : null;
      switch (body.action) {
        case 'confirm':
          if (row.status !== 'pending') return stale();
          // The host chose it, so the invitation goes out; it still counts toward the day's budget.
          counts(page).mail += 1;
          change(page, row, { status: 'confirmed', notify: true, expiresAt: null });
          break;
        case 'decline':
          if (row.status !== 'pending') return stale();
          change(page, row, { status: 'declined', notify: false, message });
          break;
        case 'cancel': {
          if (!ACTIVE.includes(row.status)) return stale();
          if (row.end <= control.now()) return refuse('not_active', { booking: ownerRow(row) });
          const reason = body.reason === undefined ? 'host_cancelled' : body.reason;
          if (!REASONS.includes(reason)) return invalid('reason');
          change(page, row, { status: 'cancelled', notify: false, cancelledBy: 'host', reason, message });
          break;
        }
        case 'reschedule': {
          if (!ACTIVE.includes(row.status)) return stale();
          if (!Number.isSafeInteger(body.start) || body.start <= 0) return invalid('start');
          // Slot rules aren't modelled here: only the host's own moves go through.
          if (body.host !== true) return refuse('slot_taken', { alternatives: [] });
          change(page, row, { start: body.start, end: body.start + (row.end - row.start), notify: false });
          break;
        }
      }
      return [200, { booking: ownerRow(row) }];
    }
  };

  // ---- the guest's side, for tests ----

  const livePage = (handle) => {
    const page = pages.get(handle);
    if (!page || !page.profile || page.retiredAt) throw new Error(`No published page ${handle}`);
    return page;
  };

  // Makes a booking as the public routes would: the guest's details sealed to the page's published key. guest:
  // { name, email, notes, timeZone, locale, via }. status: 'confirmed' or 'pending'; by default what the type, a stale
  // page and the mail budget give. plaintext(details) may change what is sealed and publicKey (base64url) seals to
  // another key, for tests of a device that must refuse what it opens. duration: one of the type's lengths, the
  // default when left out. Returns { id, manageKey, booking }.
  async function book({ handle, type, start, duration = null, guest = {}, status, channel = 'api', plaintext = (p) => p, publicKey = null }) {
    const page = livePage(handle);
    const t = page.profile.types.find((x) => x.slug === type);
    if (!t) throw new Error(`No type ${type} on ${handle}`);
    if (duration !== null && !(t.durations || [t.duration]).includes(duration)) throw new Error(`Type ${type} has no length ${duration}`);
    const now = control.now();
    const end = start + (duration ?? t.duration) * MINUTE;
    const c = counts(page, now);
    let made = status || (t.confirm || now - page.publishedAt > STALE ? 'pending' : 'confirmed');
    // Past the day's mail budget a new booking becomes a request (PROTOCOL.md, Limits).
    if (made === 'confirmed' && !status && c.mail >= control.mailMax) made = 'pending';
    const id = crypto.randomBytes(16).toString('base64url');
    const manageKey = crypto.randomBytes(32).toString('base64url');
    const details = plaintext({
      name: guest.name ?? 'Jane Doe',
      email: guest.email ?? 'jane@example.com',
      notes: guest.notes ?? '',
      timeZone: guest.timeZone ?? 'Europe/Amsterdam',
      locale: guest.locale ?? 'en',
      via: guest.via ?? '',
      manageKey,
      type,
      start,
      end
    });
    const { seal } = await sealModule();
    const { blob } = await seal({ publicRaw: new Uint8Array(Buffer.from(publicKey || page.profile.publicKey, 'base64url')), handle, bookingId: id, plaintext: details });
    c.new += 1;
    if (made === 'confirmed') c.mail += 1;
    const row = {
      id,
      type,
      start,
      end,
      status: made,
      version: 1,
      handledVersion: 0,
      seq: ++page.seq,
      sealed: Buffer.from(blob).toString('base64url'),
      channel,
      createdAt: now,
      updatedAt: now,
      expiresAt: made === 'pending' ? Math.min(now + PENDING_HOLD, start - t.minNotice * MINUTE) : null,
      reschedules: 0,
      uid: `${id}@${new URL(base()).host}`,
      notify: made === 'confirmed',
      cancelledBy: null,
      reason: null,
      message: null,
      bookedStart: start,
      bookedEnd: end
    };
    page.bookings.set(id, row);
    return { id, manageKey, booking: ownerRow(row) };
  }

  // A guest's cancellation: it always goes through, with mail while the day's budget lasts.
  function guestCancel({ handle, id }) {
    const page = livePage(handle);
    const row = page.bookings.get(id);
    if (!row || !ACTIVE.includes(row.status)) throw new Error(`Booking ${id} isn't active`);
    const c = counts(page);
    const notify = row.status === 'confirmed' && c.mail < control.mailMax;
    if (notify) c.mail += 1;
    change(page, row, { status: 'cancelled', notify, cancelledBy: 'guest', reason: null });
    return ownerRow(row);
  }

  // A guest's move. A confirmed booking past the day's mail budget can't move (limit_reached).
  function guestReschedule({ handle, id, start }) {
    const page = livePage(handle);
    const row = page.bookings.get(id);
    if (!row || !ACTIVE.includes(row.status)) throw new Error(`Booking ${id} isn't active`);
    const c = counts(page);
    if (row.status === 'confirmed') {
      if (c.mail >= control.mailMax) throw new Error('limit_reached');
      c.mail += 1;
    }
    change(page, row, { start, end: start + (row.end - row.start), notify: row.status === 'confirmed', reschedules: row.reschedules + 1 });
    return ownerRow(row);
  }

  const routes = {
    'GET /v1/info': () => [200, info()],
    'POST /v1/prelogin': ({ body }) => {
      const acc = accounts.get(body.email);
      const fake = crypto.createHash('sha256').update(`salt\n${body.email}`).digest().subarray(0, 16).toString('base64url');
      return [200, acc ? { kdf: acc.kdf, salt: acc.salt } : { kdf, salt: fake }];
    },
    'POST /v1/signup': ({ body }) => {
      if (signups === 'closed') return [403, { error: 'signups_closed' }];
      if (signups === 'invite' && !invites.includes(body.invite)) return [403, { error: 'invite_required' }];
      if (accounts.has(body.email)) return [409, { error: 'exists' }];
      const acc = {
        email: body.email,
        createdAt: Date.now(),
        kdf: body.kdf,
        salt: body.salt,
        auth: sha(body.authKey),
        wrappedKey: body.wrappedKey,
        recovery: sha(body.recoveryAuth),
        recoveryWrappedKey: body.recoveryWrappedKey,
        records: new Map(),
        seq: 0,
        booking: { handle: null, changes: [] }
      };
      accounts.set(acc.email, acc);
      const tok = newSession(acc, body.device);
      return [201, { token: tok, account: accountView(acc, tok) }];
    },
    'POST /v1/login': ({ body }) => {
      const acc = accounts.get(body.email);
      if (!acc || acc.auth !== sha(body.authKey)) return [401, { error: 'unauthorized' }];
      const tok = newSession(acc, body.device);
      return [200, { token: tok, wrappedKey: acc.wrappedKey, account: accountView(acc, tok) }];
    },
    'POST /v1/recover/start': ({ body }) => {
      const acc = accounts.get(body.email);
      if (!acc || acc.recovery !== sha(body.recoveryAuth)) return [401, { error: 'unauthorized' }];
      return [200, { recoveryWrappedKey: acc.recoveryWrappedKey, kdf: acc.kdf }];
    },
    'POST /v1/recover/finish': ({ body }) => {
      const acc = accounts.get(body.email);
      if (!acc || acc.recovery !== sha(body.recoveryAuth)) return [401, { error: 'unauthorized' }];
      Object.assign(acc, { kdf: body.kdf, salt: body.salt, auth: sha(body.authKey), wrappedKey: body.wrappedKey });
      endSessions(acc.email, null);
      const tok = newSession(acc, body.device);
      return [200, { token: tok, account: accountView(acc, tok) }];
    },
    'POST /v1/logout': ({ tok }) => {
      sessions.delete(tok);
      return [204];
    },
    'GET /v1/account': ({ acc, tok }) => [200, accountView(acc, tok)],
    'POST /v1/password': ({ acc, tok, session, body }) => {
      const wrong = wrongPassword(acc, session, body.authKey);
      if (wrong) return wrong;
      Object.assign(acc, { kdf: body.kdf, salt: body.salt, auth: sha(body.newAuthKey), wrappedKey: body.wrappedKey });
      endSessions(acc.email, tok);
      return [204];
    },
    'POST /v1/password/check': ({ acc, session, body }) => wrongPassword(acc, session, body.authKey) || [204],
    'POST /v1/recovery': ({ acc, session, body }) => {
      const wrong = wrongPassword(acc, session, body.authKey);
      if (wrong) return wrong;
      Object.assign(acc, { recovery: sha(body.recoveryAuth), recoveryWrappedKey: body.recoveryWrappedKey });
      return [204];
    },
    'DELETE /v1/account': ({ acc, session, body }) => {
      const wrong = wrongPassword(acc, session, body.authKey);
      if (wrong) return wrong;
      accounts.delete(acc.email);
      endSessions(acc.email, null);
      // The Worker wipes a deleted account's page as well; its handle stays reserved.
      const page = acc.booking.handle && pages.get(acc.booking.handle);
      const owner = typeof body.handleKey === 'string' ? `deleted:${sha(body.handleKey)}` : `deleted:${token()}`;
      if (page && page.owner === acc.email) Object.assign(page, { owner, profile: null, enabled: false, deviceId: null, offDevice: null, retiredAt: control.now(), bookings: new Map() });
      return [204];
    },
    'GET /v1/records': ({ acc, url }) => {
      const since = Number(url.searchParams.get('since')) || 0;
      const max = Math.min(Number(url.searchParams.get('limit')) || limit, limit);
      const all = [...acc.records.entries()].map(([id, r]) => ({ id, seq: r.seq, data: r.data })).filter((r) => r.seq > since).sort((a, b) => a.seq - b.seq);
      return [200, { records: all.slice(0, max), seq: acc.seq, more: all.length > max }];
    },
    'POST /v1/records': ({ acc, body }) => {
      // Like the real server: one record over 16 KiB fails the whole push.
      if (body.records.some((r) => r.data && Buffer.from(r.data, 'base64url').length > 16384)) return [413, { error: 'too_large' }];
      const results = body.records.map((r) => {
        const had = acc.records.get(r.id);
        if (r.ifAbsent && had && had.data) return { id: r.id, seq: had.seq, stored: false };
        acc.seq += 1;
        acc.records.set(r.id, { seq: acc.seq, data: r.data });
        return { id: r.id, seq: acc.seq, stored: true };
      });
      return [200, { results, seq: acc.seq }];
    }
  };

  if (!passwordCheck) delete routes['POST /v1/password/check'];

  // Routes with a value at the end of the path: the route without it, and the value.
  const PREFIXED = [
    ['DELETE', '/v1/devices/'],
    ['GET', '/v1/booking/handles/'],
    ['POST', '/v1/booking/bookings/']
  ];

  const server = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const url = new URL(req.url, 'http://localhost');
      let route = `${req.method} ${url.pathname}`;
      let rest = '';
      for (const [method, prefix] of PREFIXED) {
        if (req.method === method && url.pathname.startsWith(prefix)) {
          route = `${method} ${prefix.slice(0, -1)}`;
          rest = url.pathname.slice(prefix.length);
        }
      }
      let body = {};
      try {
        body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
      } catch (_) {
        body = {};
      }
      calls.push({ route, path: url.pathname + url.search, body, acceptEncoding: req.headers['accept-encoding'] });
      const tok = String(req.headers.authorization || '').replace(/^Bearer /, '');
      const session = sessions.get(tok);
      const acc = session && accounts.get(session.email);
      const forced = typeof control.fail === 'function' ? control.fail({ route, body, rest }) : null;
      const send = (status, json) => {
        const headers = json ? { 'content-type': 'application/json' } : {};
        let out = json ? JSON.stringify(json) : undefined;
        if (out && control.gzip && route === 'GET /v1/records') {
          out = zlib.gzipSync(out);
          headers['content-encoding'] = 'gzip';
        }
        res.writeHead(status, headers);
        res.end(out);
      };
      if (forced) return send(forced[0], forced[1]);
      if (route === 'DELETE /v1/devices') {
        if (!acc) return send(401, { error: 'unauthorized' });
        const id = decodeURIComponent(rest);
        const found = [...sessions.entries()].find(([, s]) => s.email === acc.email && s.id === id);
        if (!found) return send(404, { error: 'not_found' });
        sessions.delete(found[0]);
        return send(204);
      }
      // A server without booking pages doesn't have these routes (PROTOCOL.md, GET /v1/info).
      const handler = routes[route] || (features.includes('booking') ? booking[route] : null);
      if (!handler) return send(404, { error: 'not_found' });
      const open = ['GET /v1/info', 'POST /v1/prelogin', 'POST /v1/signup', 'POST /v1/login', 'POST /v1/recover/start', 'POST /v1/recover/finish'];
      if (!open.includes(route) && !acc) return send(401, { error: 'unauthorized' });
      const [status, json] = handler({ body, url, acc, tok, session, rest });
      send(status, json);
    });
  });

  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolve(
        Object.assign(control, {
          url: `http://127.0.0.1:${port}`,
          accounts,
          sessions,
          calls,
          pages,
          book,
          guestCancel,
          guestReschedule,
          // Another account has the handle now, as after the 12-month reservation: the owner's calls get handle_lost.
          loseHandle: (handle, owner = 'someone-else@example.com') => {
            const page = pages.get(handle);
            if (page) page.owner = owner;
          },
          close: () => new Promise((r) => server.close(() => r()))
        })
      );
    });
  });
}

module.exports = { startAccountServer };
