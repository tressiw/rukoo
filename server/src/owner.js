// The owner API for booking pages, /v1/booking (PROTOCOL.md, Booking pages). The account's object says who is
// asking; the page's object holds the handle and the bookings. The Worker does the steps between them, since objects
// don't call each other. Only a dead session answers 401: everything about the page itself is a 409 or a 404.

import { HANDLE_CHANGES } from './account.js';
import { fromBase64url, randomBytes, sha256, toBase64url } from './bytes.js';
import { accountObject, blockedHandles, bookingBase, bookingObject, bookingTimes, ACCOUNT_NAME } from './config.js';
import { json, send } from './http.js';
import * as input from './input.js';
import { HttpError } from './input.js';
import { importPublic, keyId } from './seal.js';

const TOKEN = /^Bearer ([A-Za-z0-9_-]{43})\.[A-Za-z0-9_-]{43}$/i;

const MESSAGES = {
  handle_taken: 'Another account has this handle.',
  handle_reserved: 'This handle is reserved.',
  handle_lost: "This account no longer owns the booking page's handle.",
  has_bookings: "The handle can't change while the page has bookings ahead.",
  other_device: 'Another device handles this booking page. Take over to publish from this one.',
  not_found: 'There is no such booking.',
  slot_taken: "That time isn't open.",
  stale_version: 'The booking changed meanwhile.',
  not_active: "The booking is over, so it can't be cancelled.",
  invalid: 'The request was refused.'
};

const STATUS = { not_found: 404, invalid: 400, server: 500 };

function refuse(result) {
  const { error, field, booking, alternatives } = result;
  const extra = {};
  if (field) extra.field = field;
  if (booking) extra.booking = booking;
  if (alternatives) extra.alternatives = alternatives;
  return new HttpError(STATUS[error] ?? 409, error, MESSAGES[error] ?? 'The request was refused.', {}, extra);
}

// The session's account and what it says about its booking page, or a 401.
async function signedIn(request, env) {
  const match = (request.headers.get('authorization') || '').match(TOKEN);
  if (!match || !ACCOUNT_NAME.test(match[1])) throw new HttpError(401, 'unauthorized', 'Sign in first.');
  const token = match[0].slice('Bearer '.length);
  const account = accountObject(env, match[1]);
  const auth = await account.bookingAuth(token);
  if (auth.status !== 200) throw new HttpError(auth.status, auth.body.error, auth.body.message);
  return { account, token, auth };
}

// What GET /v1/booking returns, from the page's view and the account's sessions. base is what every page address on
// this server starts with, so a device can show it before the account has a handle.
function ownerJson(env, url, auth, view, used) {
  const handleChanges = { used, max: HANDLE_CHANGES };
  const base = bookingBase(env, url);
  if (!view) {
    return {
      handle: auth.handle,
      url: auth.handle ? `${base}/${auth.handle}` : null,
      base,
      enabled: false,
      device: null,
      publishedAt: null,
      keyId: null,
      seq: 0,
      pending: 0,
      upcoming: 0,
      today: null,
      handleChanges
    };
  }
  return {
    handle: view.handle,
    url: `${base}/${view.handle}`,
    base,
    enabled: view.enabled,
    device: view.deviceId ? { id: view.deviceId, name: auth.devices[view.deviceId] ?? null, current: view.deviceId === auth.deviceId } : null,
    publishedAt: view.publishedAt,
    keyId: view.keyId,
    seq: view.seq,
    pending: view.pending,
    upcoming: view.upcoming,
    today: view.today,
    handleChanges
  };
}

export async function getBooking(request, env, url) {
  const { auth } = await signedIn(request, env);
  if (!auth.handle) return json(200, ownerJson(env, url, auth, null, auth.changes));
  const result = await bookingObject(env, auth.handle).ownerView(auth.name, bookingTimes(env), auth.deviceId);
  if (!result.ok) throw refuse(result);
  return json(200, ownerJson(env, url, auth, result.empty ? null : result.view, auth.changes));
}

// keyId must be SHA-256(publicKey)[0..8], and the key a point on the curve.
async function checkKey(profile) {
  const raw = fromBase64url(profile.publicKey);
  if (toBase64url(await keyId(raw)) !== profile.keyId) {
    throw input.invalid('profile.keyId must be the first 8 bytes of SHA-256(publicKey).', 'profile.keyId');
  }
  try {
    await importPublic(raw);
  } catch {
    throw input.invalid('profile.publicKey is not a P-256 public key.', 'profile.publicKey');
  }
}

// Publishes, claiming the handle if needed. A new handle for an account that has one is a rename: the old page stops
// first (refused while it has bookings ahead), the new one is claimed, then the account records it. When a later step
// fails, the earlier ones are undone.
export async function putBooking(request, env, url) {
  const { account, token, auth } = await signedIn(request, env);
  const body = await input.readJson(request, input.PUBLISH_BODY_MAX);
  const data = input.bookingPut(body, Date.now(), blockedHandles(env));
  await checkKey(data.profile);
  if (data.reclaim) data.reclaim = toBase64url(await sha256(data.reclaim));
  const times = bookingTimes(env);
  const owner = auth.name;
  const current = auth.handle;
  const renaming = Boolean(current) && current !== data.handle;
  const old = renaming ? bookingObject(env, current) : null;

  let before = null;
  if (renaming) {
    if (auth.changes >= HANDLE_CHANGES) {
      throw new HttpError(409, 'handle_changes', 'This account changed its handle 3 times in the last 30 days.');
    }
    const retired = await old.retire(owner, data.handle, auth.deviceId, data.takeover);
    // A lost old handle has nothing of this account's left to retire.
    if (!retired.ok && retired.error !== 'handle_lost') throw refuse(retired);
    before = retired.ok ? retired.previous : null;
  }

  const target = bookingObject(env, data.handle);
  const published = await target.publish(owner, auth.deviceId, data.takeover, data, times);
  if (!published.ok) {
    if (before) await old.restore(owner, before);
    if (published.error === 'other_owner') {
      if (data.handle === current) throw refuse({ error: 'handle_lost' });
      throw refuse({ error: published.retired ? 'handle_reserved' : 'handle_taken' });
    }
    throw refuse(published);
  }

  let used = auth.changes;
  if (data.handle !== current) {
    const set = await account.bookingSetHandle(token, data.handle);
    if (set.status !== 200) {
      await target.unclaim(owner, published.previous);
      if (before) await old.restore(owner, before);
      return send(set);
    }
    used = set.body.changes;
    if (renaming) await old.finishRetire(owner);
  }
  return json(200, ownerJson(env, url, { ...auth, handle: data.handle }, published.view, used));
}

// Turns the page off and deletes what is on it; the handle stays this account's. Works from any session, and a page
// that isn't this account's (any more) has nothing of it to delete. Turned off from another session than the booking
// device's, the page refuses that device's publishes without takeover until it has read GET /v1/booking.
// A session whose password checks are locked gets 429 locked instead, before anything goes. Rukoo for desktop from
// 2.1.0 checks the password with a sign-in of its own, deletes the page and then deletes the account on this session;
// the lock would refuse that last step after the bookings were already gone.
export async function deleteBooking(request, env) {
  const { auth } = await signedIn(request, env);
  if (auth.passwordLock) return send(auth.passwordLock);
  if (auth.handle) await bookingObject(env, auth.handle).wipe(auth.name, auth.deviceId);
  return new Response(null, { status: 204, headers: { 'cache-control': 'no-store', 'x-content-type-options': 'nosniff' } });
}

// After an account is deleted: its page goes the same way. The page's own daily check catches a failure here.
// reclaim: the hash of the deleting device's handleKey, or null; the handle stays reserved for whoever holds that key,
// not for whoever signs up next with the same email.
export async function wipeAfterDelete(env, token, handle, reclaim = null) {
  if (!handle) return;
  try {
    await bookingObject(env, handle).wipe(token.slice(0, token.indexOf('.')), null, { reclaim: reclaim ?? toBase64url(randomBytes(32)) });
  } catch (err) {
    console.error('Wiping a booking page after account deletion failed:', err?.message || 'unknown error');
  }
}

export async function checkHandle(request, env, raw) {
  const { auth } = await signedIn(request, env);
  let handle;
  try {
    handle = decodeURIComponent(raw);
  } catch {
    handle = raw;
  }
  handle = Array.from(handle).slice(0, 64).join('');
  const answer = (available, reason) => json(200, { handle, available, reason });
  const problem = input.handleCheck(handle, blockedHandles(env));
  if (problem) return answer(false, problem);
  if (handle === auth.handle) return answer(false, 'yours');
  const claim = await bookingObject(env, handle).claim(auth.name);
  if (claim === 'taken' || claim === 'reserved') return answer(false, claim);
  return answer(true, null);
}

export async function listBookings(request, env, url) {
  const { auth } = await signedIn(request, env);
  const { since, limit } = input.bookingsQuery(url);
  if (!auth.handle) return json(200, { bookings: [], seq: 0, more: false });
  const result = await bookingObject(env, auth.handle).list(auth.name, since, limit);
  if (!result.ok) throw refuse(result);
  return json(200, { bookings: result.bookings, seq: result.seq, more: result.more });
}

export async function bookingAction(request, env, rawId) {
  const { auth } = await signedIn(request, env);
  const id = input.bookingId(rawId);
  const body = await input.readJson(request);
  const action = input.ownerAction(body);
  if (!auth.handle) throw refuse({ error: 'not_found' });
  const result = await bookingObject(env, auth.handle).act(auth.name, auth.deviceId, id, action, bookingTimes(env));
  if (!result.ok) throw refuse(result);
  return json(200, { booking: result.booking });
}
