// The Rukoo account server. This Worker checks each request and hands it to the Durable Object of the account it
// belongs to; see PROTOCOL.md for the contract and README.md for running your own.
//
// There is no table of accounts or sessions anywhere. An account's object is found by a name derived from its email
// address and the PEPPER secret, and a session token starts with that name, so every request routes straight to one
// object and only that object knows whether the account or the session exists.
//
// Booking pages (/v1/booking for their owners; for everyone else the root of BOOKING_ORIGIN, or /meet without it)
// live in owner.js and meet.js, with one Booking object per handle.

import pkg from '../package.json' with { type: 'json' };
import { hmac, sameBytes, sha256, toBase64url, utf8 } from './bytes.js';
import {
  ACCOUNT_NAME,
  accountObject,
  bookingFormerRoots,
  bookingRoot,
  checkConfig,
  config,
  limitRate,
  positive,
  registryObject
} from './config.js';
import { READ_CORS, html, json, redirect, send } from './http.js';
import * as input from './input.js';
import { HttpError, invalid } from './input.js';
import { meet } from './meet.js';
import * as owner from './owner.js';
import { page } from './page.js';
import { privacy, privacyLanguage } from './privacy.js';

export { Account } from './account.js';
export { Booking } from './booking.js';
export { Registry } from './registry.js';

const TOKEN = /^Bearer ([A-Za-z0-9_-]{43})\.[A-Za-z0-9_-]{43}$/i;
const DEVICE_PATH = /^\/v1\/devices\/([^/]+)$/;
const HANDLE_PATH = /^\/v1\/booking\/handles\/([^/]+)$/;
const BOOKING_PATH = /^\/v1\/booking\/bookings\/([^/]+)$/;

// The optional parts of the protocol this server offers (PROTOCOL.md, GET /v1/info).
export const FEATURES = ['booking'];

export default {
  async fetch(request, env) {
    try {
      return await handle(request, env);
    } catch (err) {
      if (err instanceof HttpError) {
        return json(err.status, { error: err.code, message: err.message, ...err.extra }, err.headers);
      }
      // Only the error itself is logged: never a body, a token or an address.
      console.error('Request failed:', err?.stack || err);
      return json(500, { error: 'server', message: 'Something went wrong on the server.' });
    }
  }
};

async function handle(request, env) {
  const url = new URL(request.url);
  const { pathname: path } = url;
  const method = request.method;

  const reading = method === 'GET' || method === 'HEAD';
  if (path === '/' && reading) return html(page(config(env, url)), method);
  if (path === '/privacy' && reading) {
    return html(privacy(config(env, url), privacyLanguage(url, request.headers.get('accept-language'))), method);
  }
  if (path === '/v1/info' && method === 'GET') return json(200, info(env, url));
  const account = path === '/v1' || path.startsWith('/v1/');
  const root = bookingRoot(env);
  if (path === '/meet' || path.startsWith('/meet/')) return root ? toRoot(request, root, url) : meet(request, env, url);
  // At the root of BOOKING_ORIGIN every other path is a booking page's, /mcp, /booker.js, /llms.txt, an icon or a font.
  if (root === url.origin && !account) return meet(request, env, url);
  // Where the pages lived before: links shared from there keep working. Reads go to the same path under the new
  // address. Posts, keyed reads and the guest's manage page are answered here, with the new address in their answers.
  // A form's post and the manage page it leads to stay on the form's own origin (form-action 'self' covers the whole
  // navigation), and clients drop the Authorization header on a redirect to another origin. That page loads the
  // script, icons and fonts from here too, under its own CSP.
  if (root && !account && bookingFormerRoots(env).has(url.origin)) {
    const here = !reading || request.headers.has('authorization') || MANAGE_PATH.test(path) || PAGE_ASSET.test(path);
    return here ? meet(request, env, url) : toRoot(request, root, url, path);
  }
  if (!account) throw notFound();

  checkConfig(env);

  const route = `${method} ${path}`;
  switch (route) {
    case 'POST /v1/prelogin':
      return prelogin(request, env);
    case 'POST /v1/signup':
      return signup(request, env);
    case 'POST /v1/login':
      return login(request, env);
    case 'POST /v1/recover/start':
      return recoverStart(request, env);
    case 'POST /v1/recover/finish':
      return recoverFinish(request, env);
    case 'POST /v1/logout': {
      const { stub, token } = session(request, env);
      return send(await stub.logout(token));
    }
    case 'GET /v1/account': {
      const { stub, token } = session(request, env);
      return send(await stub.account(token, limits(env)));
    }
    case 'POST /v1/password':
      return changePassword(request, env);
    case 'POST /v1/password/check':
      return checkPassword(request, env);
    case 'POST /v1/recovery':
      return replaceRecovery(request, env);
    case 'DELETE /v1/account':
      return deleteAccount(request, env);
    case 'GET /v1/records':
      return pull(request, env, url);
    case 'POST /v1/records':
      return push(request, env);
    case 'GET /v1/booking':
      return owner.getBooking(request, env, url);
    case 'PUT /v1/booking':
      return owner.putBooking(request, env, url);
    case 'DELETE /v1/booking':
      return owner.deleteBooking(request, env);
    case 'GET /v1/booking/bookings':
      return owner.listBookings(request, env, url);
  }
  const device = path.match(DEVICE_PATH);
  if (device && method === 'DELETE') {
    const { stub, token } = session(request, env);
    if (!/^[A-Za-z0-9_-]{22}$/.test(device[1])) throw invalid('That is not a device id.');
    return send(await stub.revoke(token, device[1]));
  }
  const handleCheck = path.match(HANDLE_PATH);
  if (handleCheck && method === 'GET') return owner.checkHandle(request, env, handleCheck[1]);
  const action = path.match(BOOKING_PATH);
  if (action && method === 'POST') return owner.bookingAction(request, env, action[1]);
  throw notFound();
}

async function prelogin(request, env) {
  await limitRate(request, env.RATE_LIMITER);
  const body = await input.readJson(request);
  const email = input.email(body.email);
  const name = await accountName(env, email);
  const result = await stub(env, name).prelogin();
  if (result.body) return send(result);
  // An address without an account gets the default parameters and a salt that is the same on every call, so this
  // answer can't tell anyone whether an account exists.
  const salt = (await hmac(env.PEPPER, `salt\n${email}`)).slice(0, 16);
  return json(200, { kdf: input.DEFAULT_KDF, salt: toBase64url(salt) });
}

async function signup(request, env) {
  await limitRate(request, env.RATE_LIMITER);
  const body = await input.readJson(request);
  const mode = config(env, new URL(request.url)).signups;
  if (mode === 'closed') throw new HttpError(403, 'signups_closed', 'This server does not take new accounts.');
  const invite = mode === 'invite' ? await validInvite(env, body.invite) : null;
  if (mode === 'invite' && !invite) {
    throw new HttpError(
      403,
      'invite_required',
      'This server takes new accounts by invitation. The invite code is missing or wrong.'
    );
  }
  const fields = {
    email: input.email(body.email),
    kdf: input.kdf(body.kdf),
    salt: input.key(body.salt, 'salt', 16),
    authKey: input.key(body.authKey, 'authKey', 32),
    wrappedKey: input.sealed(body.wrappedKey, 'wrappedKey'),
    recoveryAuth: input.key(body.recoveryAuth, 'recoveryAuth', 32),
    recoveryWrappedKey: input.sealed(body.recoveryWrappedKey, 'recoveryWrappedKey'),
    device: input.device(body.device)
  };
  const name = await accountName(env, fields.email);
  // A place under MAX_ACCOUNTS first; given back when the sign-up fails, for one because the address has an account.
  const registry = registryObject(env);
  if (registry) {
    const place = await registry.reserve(maxAccounts(env), invite, inviteUses(env));
    if (place === 'full') throw new HttpError(403, 'signups_closed', 'This server has no room for new accounts.');
    if (place === 'used') throw new HttpError(403, 'invite_required', 'This invite code has been used.');
  }
  let result;
  try {
    result = await stub(env, name).signup(name, fields, limits(env));
  } finally {
    if (registry && (!result || result.status !== 201)) await registry.release(invite);
  }
  return send(result);
}

async function login(request, env) {
  await limitRate(request, env.RATE_LIMITER);
  const body = await input.readJson(request);
  const email = input.email(body.email);
  const fields = { authKey: input.key(body.authKey, 'authKey', 32), device: input.device(body.device) };
  const name = await accountName(env, email);
  return send(await stub(env, name).login(name, fields, limits(env)));
}

async function recoverStart(request, env) {
  await limitRate(request, env.RATE_LIMITER);
  const body = await input.readJson(request);
  const email = input.email(body.email);
  const fields = { recoveryAuth: input.key(body.recoveryAuth, 'recoveryAuth', 32) };
  return send(await stub(env, await accountName(env, email)).recoverStart(fields));
}

async function recoverFinish(request, env) {
  await limitRate(request, env.RATE_LIMITER);
  const body = await input.readJson(request);
  const email = input.email(body.email);
  const fields = {
    recoveryAuth: input.key(body.recoveryAuth, 'recoveryAuth', 32),
    kdf: input.kdf(body.kdf),
    salt: input.key(body.salt, 'salt', 16),
    authKey: input.key(body.authKey, 'authKey', 32),
    wrappedKey: input.sealed(body.wrappedKey, 'wrappedKey'),
    device: input.device(body.device)
  };
  const name = await accountName(env, email);
  return send(await stub(env, name).recoverFinish(name, fields, limits(env)));
}

async function changePassword(request, env) {
  const { stub, token } = session(request, env);
  const body = await input.readJson(request);
  const fields = {
    authKey: input.key(body.authKey, 'authKey', 32),
    kdf: input.kdf(body.kdf),
    salt: input.key(body.salt, 'salt', 16),
    newAuthKey: input.key(body.newAuthKey, 'newAuthKey', 32),
    wrappedKey: input.sealed(body.wrappedKey, 'wrappedKey')
  };
  return send(await stub.changePassword(token, fields));
}

// Only checks the password, on the session's own lock (PROTOCOL.md, POST /v1/password/check).
async function checkPassword(request, env) {
  const { stub, token } = session(request, env);
  const body = await input.readJson(request);
  return send(await stub.checkPassword(token, { authKey: input.key(body.authKey, 'authKey', 32) }));
}

async function replaceRecovery(request, env) {
  const { stub, token } = session(request, env);
  const body = await input.readJson(request);
  const fields = {
    authKey: input.key(body.authKey, 'authKey', 32),
    recoveryAuth: input.key(body.recoveryAuth, 'recoveryAuth', 32),
    recoveryWrappedKey: input.sealed(body.recoveryWrappedKey, 'recoveryWrappedKey')
  };
  return send(await stub.replaceRecovery(token, fields));
}

// The account's booking page goes with it: the object hands back the handle it had, and the page is wiped after.
// handleKey (optional): the deleting device's key to take the handle back with an account made again later; sign-ups
// don't prove the address, so the email alone doesn't (PROTOCOL.md, Handles).
async function deleteAccount(request, env) {
  const { stub, token } = session(request, env);
  const body = await input.readJson(request);
  const handleKey = body.handleKey === undefined || body.handleKey === null ? null : input.key(body.handleKey, 'handleKey', 32);
  const result = await stub.deleteAccount(token, { authKey: input.key(body.authKey, 'authKey', 32) });
  if (result.status === 204) {
    const registry = registryObject(env);
    if (registry) await registry.release();
    await owner.wipeAfterDelete(env, token, result.handle, handleKey ? toBase64url(await sha256(handleKey)) : null);
  }
  return send(result);
}

async function pull(request, env, url) {
  const { stub, token } = session(request, env);
  const { since, limit } = input.pullQuery(url);
  return send(await stub.pull(token, since, limit));
}

async function push(request, env) {
  const { stub, token } = session(request, env);
  await limitRate(request, env.PUSH_LIMITER);
  const body = await input.readPushJson(request);
  return send(await stub.push(token, input.records(body.records), limits(env)));
}

// The account object a bearer token points at. Only the token's shape is checked here; whether it is a live session
// is for the object to say.
function session(request, env) {
  const match = (request.headers.get('authorization') || '').match(TOKEN);
  if (!match) throw new HttpError(401, 'unauthorized', 'Sign in first.');
  return { stub: stub(env, match[1]), token: match[0].slice('Bearer '.length) };
}

async function accountName(env, email) {
  return toBase64url(await hmac(env.PEPPER, `account\n${email}`));
}

function maxAccounts(env) {
  const n = Number(env.MAX_ACCOUNTS);
  return Number.isSafeInteger(n) && n > 0 ? n : 0;
}

function stub(env, name) {
  if (!ACCOUNT_NAME.test(name)) throw new HttpError(401, 'unauthorized', 'Sign in first.');
  return accountObject(env, name);
}

// Every listed code works. Each is compared through its hash, so the comparison takes the same time whatever the
// codes' lengths, and every code is compared, so the time doesn't show which one matched.
// The hash of the invite code when it is one of INVITE_CODES, else null. The registry counts its uses by this hash.
async function validInvite(env, invite) {
  if (typeof invite !== 'string' || !invite.trim()) return null;
  const codes = String(env.INVITE_CODES || '')
    .split(',')
    .map((code) => code.trim())
    .filter(Boolean);
  const given = await sha256(utf8(invite.trim()));
  let found = false;
  for (const code of codes) {
    if (sameBytes(await sha256(utf8(code)), given)) found = true;
  }
  return found ? toBase64url(given) : null;
}

// How many accounts one invite code may make; 0 for any number. The default is one each.
function inviteUses(env) {
  if (env.INVITE_USES === undefined || env.INVITE_USES === '') return 1;
  const n = Number(env.INVITE_USES);
  return Number.isSafeInteger(n) && n >= 0 ? n : 1;
}

function limits(env) {
  return {
    records: positive(env.MAX_RECORDS, 250000),
    bytes: positive(env.MAX_BYTES, 104857600),
    recordBytes: input.RECORD_BYTES,
    batch: input.BATCH
  };
}

function info(env, url) {
  const { operator, jurisdiction: where, signups: mode } = config(env, url);
  return {
    protocol: 1,
    version: pkg.version,
    operator,
    hosting: { provider: 'Cloudflare', company: 'Cloudflare, Inc.', country: 'US', jurisdiction: where },
    signups: mode,
    limits: limits(env),
    features: FEATURES
  };
}

function notFound() {
  return new HttpError(404, 'not_found', 'There is nothing here.');
}

// /meet addresses, with BOOKING_ORIGIN set, and reads on a former root: the same path at its root, the query kept. 301 for reads; 308 for the rest, so a client sends its POST again there. Reads carry the CORS header, so a
// cross-site fetch can follow them. rest: the path under the pages, /meet taken off.
// A guest's manage page or calendar file, /<handle>/booking/<id>[.ics].
const MANAGE_PATH = /^\/[^/]+\/booking\/[^/]+$/;
// The pages' script and icons, which are at the root there and under /meet/assets/ without BOOKING_ORIGIN.
const ROOT_FILE = /^\/(?:booker\.js|favicon\.ico|favicon\.svg|apple-touch-icon\.png)$/;
// The pages' own script, icons and fonts at the root.
const PAGE_ASSET = /^\/(?:booker\.js|favicon\.ico|favicon\.svg|apple-touch-icon\.png|assets\/fonts\/[^/]+)$/;

function toRoot(request, root, url, rest = url.pathname.slice('/meet'.length)) {
  const file = url.pathname.slice('/meet/assets'.length);
  if (url.pathname.startsWith('/meet/assets/') && ROOT_FILE.test(file)) rest = file;
  const reading = request.method === 'GET' || request.method === 'HEAD';
  return redirect(reading ? 301 : 308, `${root}${rest || '/'}${url.search}`, reading ? READ_CORS : {});
}
