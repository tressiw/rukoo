'use strict';

// This computer as a host for Rukoo on a phone. A phone pairs with a code shown in Settings, may fetch the accounts
// and agent settings once, and runs Claude Code and Codex chats here, because those agents only run on a computer.
// The phone owns the chat, its mail and its transcript. This computer runs each turn with the same adapters as its own
// chats and streams the adapter's events back as NDJSON, so the phone's hub reads them like its own adapters' events.
// Rukoo's tools in such a chat act on the phone: Claude Code and Codex call this Rukoo's loopback MCP server as always,
// and route() sends those calls on to the MCP server of Rukoo on the phone.
//
// Everything here sits under /phone/v1 on the Tailscale listener, and only while "Let my phone use this computer" is
// on. Anything else answers 404, so a scan of the tailnet can't tell the feature exists. docs/agents.md ("Rukoo on your
// phone") describes the routes.

const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { AgentError, EFFORTS, MAX_PAIRINGS, isTailscale, writeJsonAtomic } = require('./config');
const { json, bearer, MAX_ABSURD } = require('./mcp');
const { describe } = require('./http');
const { INLINE_IMAGE_MAX, MAX_FILES, MAX_EMAILS } = require('./chatfiles');
const context = require('./context');
const { safeName } = require('../files');

// The agents a phone can run here.
const PHONE_AGENTS = ['claude', 'codex'];
const AGENT_NAMES = { claude: 'Claude Code', codex: 'Codex' };

// Pairing: an 8-character code from Crockford's base32 (no I, L, O or U), 40 bits, shown as XXXX-XXXX. It works for
// five minutes and once; five wrong tries burn it. With the rate limits below, guessing it is hopeless.
const CODE_ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
const PAIR_TTL_MS = 5 * 60 * 1000;
const PAIR_FAILURES = 5;
const PAIR_LIMIT_ALL = 10;
const PAIR_LIMIT_SOURCE = 5;
// A token that fails this often from one address in a minute locks that address out for five minutes.
const AUTH_FAILURES = 20;
const AUTH_BLOCK_MS = 5 * 60 * 1000;
// Per phone: requests other than the two streams, per minute, and turns at once. A turn counts from the moment its
// upload starts, and all phones together get MAX_TURNS_ALL.
const REQUESTS_PER_MINUTE = 60;
const MAX_TURNS_PER_PHONE = 4;
const MAX_TURNS_ALL = 8;
const MINUTE = 60 * 1000;
// The Claude Code sessions and Codex threads this computer gave phone chats, next to agents.json (see bindProvider).
const PROVIDER_FILE = 'phone-sessions.json';
const PROVIDER_KINDS = ['sessionId', 'threadId'];
const MAX_PROVIDER_IDS = 2000;
// The phone may fetch the export this long after pairing, or after "Send accounts and settings again".
const EXPORT_WINDOW_MS = 10 * 60 * 1000;

// Streams. A ping keeps a quiet connection open through Doze and mobile networks; the phone reads with a timeout
// above it. A stream that drops leaves the turn running this long, so the phone can reattach (events?after=).
const HEARTBEAT_MS = 15000;
const REATTACH_GRACE_MS = 30000;
// A finished turn's events stay this long for a phone that reattaches after the end.
const KEEP_FINISHED_MS = 2 * 60 * 1000;
const MAX_EVENTS = 5000;
// An adapter that ignores stop gets this long before the turn ends anyway (hub.js STOP_GRACE_MS).
const STOP_GRACE_MS = 12000;
// The phone answers a start-over with the email and a recap of the chat.
const START_OVER_WAIT_MS = 60000;
// A chat's route to the phone's MCP server outlives its turn this long, as a Claude Code process does (claude.js
// IDLE_MS), so tool calls between turns still reach the phone.
const ROUTE_IDLE_MS = 15 * 60 * 1000;
// Claude Code and Codex give a Rukoo tool 120 s; the answer has to be back before they give up.
const FORWARD_TIMEOUT_MS = 115000;
// read_attachment can return a 10 MB file as a base64 resource.
const FORWARD_MAX = 32 * 1024 * 1024;

// Body limits: small requests, a start-over (the email and a recap), a turn (up to ten 3.75 MB images in base64).
const SMALL_BODY = 16 * 1024;
const START_OVER_BODY = 1024 * 1024;
const PHONE_TURN_MAX = 52 * 1024 * 1024;
const INPUT_MAX = 400000;
const TEXT_MAX = 20000;
const INSTRUCTIONS_MAX = 20000;
const MAX_SKILLS = 20;
const CONVERSATION_ID = /^[A-Za-z0-9_-]{1,100}$/;
const ID = '([A-Za-z0-9_-]{1,40})';

// Adapter events that go to the phone as they are (hub.js onAdapterEvent), with their text fields clipped.
const EVENT_TYPES = new Set(['text', 'commentary', 'thinking', 'thinking-done', 'tool-start', 'tool-end', 'error', 'notice', 'usage']);

const ROUTES = [
  ['POST', new RegExp('^/phone/v1/pair$'), 'pair'],
  ['GET', new RegExp('^/phone/v1/hello$'), 'hello'],
  ['GET', new RegExp('^/phone/v1/export$'), 'export'],
  ['GET', new RegExp('^/phone/v1/agents/(claude|codex)/(models|skills)$'), 'lists'],
  ['POST', new RegExp('^/phone/v1/turns$'), 'startTurn'],
  ['GET', new RegExp(`^/phone/v1/turns/${ID}/events$`), 'events'],
  ['POST', new RegExp(`^/phone/v1/turns/${ID}/approvals/${ID}$`), 'decide'],
  ['POST', new RegExp(`^/phone/v1/turns/${ID}/stop$`), 'stop'],
  ['POST', new RegExp(`^/phone/v1/turns/${ID}/start-over/${ID}$`), 'startOverReply'],
  ['POST', new RegExp('^/phone/v1/conversations/forget$'), 'forget'],
  ['DELETE', new RegExp('^/phone/v1/pairing$'), 'unpairSelf']
];
// The two streams: the phone keeps them open, so they don't count as requests per minute.
const STREAMS = new Set(['startTurn', 'events']);
// Per route: how large its body may be, and whether it is a JSON object. GET routes take none.
const BODIES = {
  hello: [0, false],
  export: [0, false],
  lists: [0, false],
  events: [0, false],
  startTurn: [PHONE_TURN_MAX, true],
  decide: [SMALL_BODY, true],
  stop: [SMALL_BODY, false],
  startOverReply: [START_OVER_BODY, true],
  forget: [SMALL_BODY, true],
  unpairSelf: [SMALL_BODY, false]
};

// A chat the phone's route was dropped for: its tool calls fail instead of running on this computer's mail.
const GONE = Object.freeze({ gone: true });
const PING = '{"type":"ping"}\n';
const TOO_BIG = Symbol('too-big');

const rand = (n) => crypto.randomBytes(n).toString('base64url').replace(/[-_]/g, '').slice(0, n).toLowerCase().padEnd(n, '0');
const sha256 = (s) => crypto.createHash('sha256').update(String(s)).digest();
const isObj = (v) => Boolean(v) && typeof v === 'object' && !Array.isArray(v);
const clip = (v, max) => String(v == null ? '' : v).slice(0, max);
const oneLine = (v) => String(v == null ? '' : v).replace(/\s+/g, ' ').trim();
const peerOf = (req) => String((req.socket && req.socket.remoteAddress) || '').replace(/^::ffff:/, '');
const line = (event) => `${JSON.stringify(event)}\n`;
const hubModule = () => require('./hub');
const providerId = (v) => typeof v === 'string' && v.length > 0 && v.length <= 200 && !/[\u0000-\u001f\u007f]/.test(v);

function fail(res, status, code, detail = '', extra = {}) {
  json(res, status, detail ? { error: code, detail } : { error: code }, extra);
}

function errorResult(text) {
  return { content: [{ type: 'text', text }], isError: true };
}

// What the user types, as the code was made: Crockford's base32 reads O as 0 and I and L as 1.
function normalizeCode(v) {
  return String(v || '')
    .toUpperCase()
    .replace(/[\s-]/g, '')
    .replace(/O/g, '0')
    .replace(/[IL]/g, '1');
}

function newCode() {
  let s = '';
  for (let i = 0; i < 8; i++) s += CODE_ALPHABET[crypto.randomInt(CODE_ALPHABET.length)];
  return s;
}

// The image type from the file's first bytes; the phone's word for it does not count.
function imageType(buf) {
  if (buf.length >= 8 && buf.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))) return 'image/png';
  if (buf.length >= 3 && buf[0] === 0xff && buf[1] === 0xd8 && buf[2] === 0xff) return 'image/jpeg';
  if (buf.length >= 6 && /^GIF8[79]a$/.test(buf.subarray(0, 6).toString('latin1'))) return 'image/gif';
  if (buf.length >= 12 && buf.subarray(0, 4).toString('latin1') === 'RIFF' && buf.subarray(8, 12).toString('latin1') === 'WEBP') return 'image/webp';
  return null;
}

// Fixed-window counters per key: a key is over its limit until its minute is up.
class Limiter {
  constructor(limit, ms, now) {
    this.limit = limit;
    this.ms = ms;
    this.now = now;
    this.windows = new Map();
  }

  window(key) {
    const t = this.now();
    let w = this.windows.get(key);
    if (!w || t - w.start >= this.ms) {
      w = { start: t, count: 0 };
      this.windows.set(key, w);
    }
    return w;
  }

  allows(key) {
    return this.window(key).count < this.limit;
  }

  take(key) {
    const w = this.window(key);
    if (w.count >= this.limit) return false;
    w.count++;
    return true;
  }

  // Whole seconds until key may try again.
  retryAfter(key) {
    const w = this.window(key);
    return String(Math.max(1, Math.ceil((w.start + this.ms - this.now()) / 1000)));
  }

  prune() {
    const t = this.now();
    for (const [key, w] of this.windows) if (t - w.start >= this.ms) this.windows.delete(key);
  }
}

class BadRequest extends Error {}
function bad(detail) {
  throw new BadRequest(detail);
}

class PhoneHost {
  // options (tests): now(), isPeer(ip) instead of the Tailscale check on the address a request comes from,
  // hostname(), fetch, and shorter waits (graceMs, stopGraceMs, startOverWaitMs, heartbeatMs).
  constructor(hub, options = {}) {
    this.hub = hub;
    this.now = options.now || Date.now;
    this.waits = {
      grace: options.graceMs || REATTACH_GRACE_MS,
      stop: options.stopGraceMs || STOP_GRACE_MS,
      startOver: options.startOverWaitMs || START_OVER_WAIT_MS,
      heartbeat: options.heartbeatMs || HEARTBEAT_MS
    };
    this.isPeer = options.isPeer || isTailscale;
    this.hostname = options.hostname || (() => os.hostname());
    this.fetch = options.fetch || ((...args) => fetch(...args));
    this.code = null;
    this.hashes = [];
    this.pairAll = new Limiter(PAIR_LIMIT_ALL, MINUTE, this.now);
    this.pairSource = new Limiter(PAIR_LIMIT_SOURCE, MINUTE, this.now);
    this.authFailures = new Limiter(AUTH_FAILURES, MINUTE, this.now);
    this.blocked = new Map();
    this.requests = new Limiter(REQUESTS_PER_MINUTE, MINUTE, this.now);
    // turnId → turn; a finished one stays KEEP_FINISHED_MS for a reattach.
    this.turns = new Map();
    // host conversation id (p_<pairing>_<phone chat>) → id of its running turn.
    this.byConversation = new Map();
    // host conversation id → { pairingId, phoneCid, agent, url, token, at }: where its tool calls go.
    this.routes = new Map();
    // Codex thread id → host conversation id, for tool calls Codex makes with its shared token. Kept after the chat's
    // route goes, like chats below: Codex's token outlives the route, and a call from a phone's thread must then fail
    // rather than run on this computer's mail.
    this.threads = new Map();
    // "<agent>\n<phone chat id>" → host conversation id, for every phone chat this run has seen.
    this.chats = new Map();
    // host conversation id → the token Claude Code signs in with on the loopback listener.
    this.tokens = new Map();
    // pairing id → its turns whose body is still being read. They hold a turn's place (see dispatch()).
    this.uploads = new Map();
    // "<kind>\n<id>" → { kind, id, pairingId, conversationId, agent }: which phone chat this computer gave a Claude
    // Code session or Codex thread to, oldest first. Kept in PROVIDER_FILE, so a restart keeps the check.
    this.providerFile = path.join(hub.dataDir, PROVIDER_FILE);
    this.providerIds = this.loadProviderIds();
    this.refresh();
    this.sweeper = setInterval(() => this.sweep(), MINUTE);
    if (this.sweeper.unref) this.sweeper.unref();
  }

  log(...args) {
    this.hub.log('phone', ...args);
  }

  // The hub's conversation ids start with c_; a phone's chats here are p_<pairing id>_<the phone's id>.
  owns(cid) {
    return String(cid || '').startsWith('p_');
  }

  enabled() {
    return Boolean(this.hub.cfg.data.phone && this.hub.cfg.data.phone.enabled);
  }

  // The pairings' token hashes, after any change to the pairings.
  refresh() {
    this.hashes = this.hub.cfg.pairingHashes ? this.hub.cfg.pairingHashes() : [];
  }

  emit(payload) {
    this.hub.emitEvent({ kind: 'phone', ...payload });
  }

  // ---------- pairing (Settings) ----------

  startPairing({ allowExport = false, address, port }) {
    this.dropCode(null);
    const code = newCode();
    const expiresAt = this.now() + PAIR_TTL_MS;
    const timer = setTimeout(() => this.code && this.code.timer === timer && this.dropCode('expired'), PAIR_TTL_MS);
    if (timer.unref) timer.unref();
    this.code = { hash: sha256(code), expiresAt, allowExport, failures: 0, timer };
    return { code: `${code.slice(0, 4)}-${code.slice(4)}`, expiresAt, allowExport, address, port, url: `http://${address}:${port}` };
  }

  cancelPairing() {
    this.dropCode(null);
  }

  // The code that works now, or null.
  pairCode() {
    return this.code && this.code.expiresAt > this.now() ? this.code : null;
  }

  // reason: what Settings is told ('expired', 'burned'), or null when it knows already.
  dropCode(reason) {
    if (!this.code) return;
    clearTimeout(this.code.timer);
    this.code = null;
    if (reason) this.emit({ event: reason });
  }

  allowExport(id) {
    if (!this.hub.cfg.pairing(id)) return false;
    this.hub.cfg.setExportWindow(id, this.now() + EXPORT_WINDOW_MS);
    this.emit({ event: 'changed' });
    return true;
  }

  // by: 'desktop' (Settings) or 'phone' (the phone unpaired itself). Its chats here stop and their routes go.
  unpair(id, by) {
    const p = this.hub.cfg.pairing(id);
    if (!p) return false;
    for (const turn of this.turns.values()) if (turn.pairingId === id) this.stopTurn(turn);
    const prefix = `p_${id}_`;
    for (const [hostCid, r] of [...this.routes]) if (r.pairingId === id) this.forgetChat(hostCid, r.agent);
    for (const hostCid of [...this.tokens.keys()]) if (hostCid.startsWith(prefix)) this.forgetChat(hostCid, null);
    this.dropProviderIds((r) => r.pairingId === id);
    this.hub.cfg.removePairing(id);
    this.refresh();
    this.emit({ event: 'removed', pairing: { id, name: p.name }, by });
    return true;
  }

  // "Let my phone use this computer" went off: the code stops working, turns stop, and calls of earlier chats fail.
  stopAll() {
    this.dropCode(null);
    for (const turn of this.turns.values()) this.stopTurn(turn);
    for (const [hostCid, r] of [...this.routes]) this.forgetChat(hostCid, r.agent);
    for (const hostCid of [...this.tokens.keys()]) this.forgetChat(hostCid, null);
  }

  dispose() {
    clearInterval(this.sweeper);
    this.dropCode(null);
    for (const turn of [...this.turns.values()]) {
      if (!turn.done) {
        turn.controller.abort();
        this.finish(turn, { status: 'stopped' });
      }
      clearTimeout(turn.dropTimer);
    }
    this.turns.clear();
  }

  // Once a minute: routes of chats that have been quiet for ROUTE_IDLE_MS go, and old rate-limit windows.
  sweep() {
    const t = this.now();
    for (const [hostCid, r] of [...this.routes]) {
      if (this.byConversation.has(hostCid) || t - r.at < ROUTE_IDLE_MS) continue;
      this.dropRoute(hostCid);
    }
    for (const limiter of [this.pairAll, this.pairSource, this.authFailures, this.requests]) limiter.prune();
    for (const [ip, until] of this.blocked) if (until <= t) this.blocked.delete(ip);
  }

  dropRoute(hostCid) {
    this.routes.delete(hostCid);
    const token = this.tokens.get(hostCid);
    if (token) this.hub.revokeToken(token);
    this.tokens.delete(hostCid);
  }

  // The phone deleted the chat, unpaired, or may no longer use this computer. agent: whose process to close.
  forgetChat(hostCid, agent) {
    const turnId = this.byConversation.get(hostCid);
    const turn = turnId && this.turns.get(turnId);
    if (turn) this.stopTurn(turn);
    this.dropRoute(hostCid);
    const adapter = agent && this.hub.adapters.get(agent);
    // Optional adapter hook, as for the hub's own chats: Claude closes the process that belongs to this chat.
    if (adapter && typeof adapter.forget === 'function') Promise.resolve().then(() => adapter.forget({ id: hostCid })).catch(() => {});
  }

  // ---------- requests ----------

  handle(entry, req, res, pathname) {
    this.dispatch(entry, req, res, pathname).catch((err) => {
      this.log('request failed:', err && err.message);
      if (!res.headersSent) fail(res, 500, 'unknown', 'Something went wrong on the computer');
      else res.end();
    });
  }

  async dispatch(entry, req, res, pathname) {
    const ip = peerOf(req);
    // Off, not on the Tailscale listener, or from an address outside the tailnet: as if nothing were here.
    if (!entry.remote || !this.enabled() || !this.isPeer(ip) || this.hub.disposed) return this.refuse(req, res, 404, 'not found');
    const matches = ROUTES.filter(([, re]) => re.test(pathname));
    if (!matches.length) return this.refuse(req, res, 404, 'not found');
    const hit = matches.find(([method]) => method === req.method);
    if (!hit) return this.refuse(req, res, 405, 'method not allowed', '', { Allow: matches.map(([m]) => m).join(', ') });
    const [, re, name] = hit;
    const params = re.exec(pathname).slice(1);
    const query = new URL(String(req.url || ''), 'http://phone').searchParams;
    if (name === 'pair') return this.pair(req, res, ip);
    const until = this.blocked.get(ip);
    if (until && until > this.now()) return this.refuse(req, res, 429, 'rate-limited', 'Too many tries with a wrong token', { 'Retry-After': String(Math.ceil((until - this.now()) / 1000)) });
    const pairing = this.identify(bearer(req));
    if (!pairing) {
      if (!this.authFailures.take(ip)) this.blocked.set(ip, this.now() + AUTH_BLOCK_MS);
      return this.refuse(req, res, 401, 'unauthorized');
    }
    if (!STREAMS.has(name) && !this.requests.take(pairing.id)) {
      return this.refuse(req, res, 429, 'rate-limited', `At most ${REQUESTS_PER_MINUTE} requests a minute`, { 'Retry-After': this.requests.retryAfter(pairing.id) });
    }
    if (this.hub.cfg.touchPairing(pairing.id, this.now())) this.emit({ event: 'changed' });
    // A turn's body can be 52 MB, so it takes its turn's place before it is read: uploads count as running turns.
    // The place comes free when the request ends, however it ends; a turn it started counts itself from then on.
    const upload = name === 'startTurn';
    if (upload) {
      const full = this.turnsFull(pairing.id);
      if (full) return this.refuse(req, res, 429, 'rate-limited', full);
      this.uploads.set(pairing.id, (this.uploads.get(pairing.id) || 0) + 1);
    }
    try {
      const [max, isJson] = BODIES[name];
      const body = isJson ? await this.readJson(req, res, max) : await this.readCapped(req, res, max);
      if (body === undefined) return;
      // A body can take a while to arrive. The phone may have been removed, or the switch turned off, meanwhile: then
      // nothing runs and nothing goes out.
      const current = this.enabled() && !this.hub.disposed ? this.hub.cfg.pairing(pairing.id) : null;
      if (!current) return fail(res, this.enabled() ? 401 : 404, this.enabled() ? 'unauthorized' : 'not found');
      return await this[name](req, res, { pairing: current, ip, params, query, body });
    } finally {
      if (upload) {
        const left = (this.uploads.get(pairing.id) || 1) - 1;
        if (left > 0) this.uploads.set(pairing.id, left);
        else this.uploads.delete(pairing.id);
      }
    }
  }

  // Why the phone can't start another turn now, or null. Running turns and turns still uploading both count.
  turnsFull(pairingId) {
    let mine = this.uploads.get(pairingId) || 0;
    let all = 0;
    for (const n of this.uploads.values()) all += n;
    for (const turn of this.turns.values()) {
      if (turn.done) continue;
      all++;
      if (turn.pairingId === pairingId) mine++;
    }
    if (mine >= MAX_TURNS_PER_PHONE) return `At most ${MAX_TURNS_PER_PHONE} turns from one phone run at once`;
    if (all >= MAX_TURNS_ALL) return `At most ${MAX_TURNS_ALL} turns from phones run at once`;
    return null;
  }

  // The pairing a bearer token belongs to. Every entry is compared, in constant time, as in hub.identify().
  identify(token) {
    if (typeof token !== 'string' || !token || token.length > 512) return null;
    const hash = sha256(token);
    let found = null;
    for (const p of this.hashes) if (crypto.timingSafeEqual(p.hash, hash)) found = p.id;
    return found ? this.hub.cfg.pairing(found) : null;
  }

  // Answers once the request is read, like mcp.js reject(): a reset could otherwise reach the phone first.
  async refuse(req, res, status, code, detail = '', extra = {}) {
    await this.readBody(req, 0);
    if (!res.destroyed) fail(res, status, code, detail, extra);
  }

  // The body as a Buffer, or TOO_BIG once it passes max (the rest is read and dropped, so the 413 arrives).
  readBody(req, max) {
    return new Promise((resolve) => {
      if (Number(req.headers['content-length'] || 0) > MAX_ABSURD) {
        req.destroy();
        return resolve(TOO_BIG);
      }
      const chunks = [];
      let size = 0;
      let tooBig = false;
      req.on('data', (chunk) => {
        if (tooBig) return;
        size += chunk.length;
        if (size > max) {
          tooBig = true;
          chunks.length = 0;
          return;
        }
        chunks.push(chunk);
      });
      req.on('error', () => resolve(TOO_BIG));
      req.on('end', () => resolve(tooBig ? TOO_BIG : Buffer.concat(chunks)));
      // Closed before the end (the phone hung up): settled anyway, so a turn's place comes free.
      req.on('close', () => resolve(TOO_BIG));
    });
  }

  // The body as a Buffer, or undefined after answering 413.
  async readCapped(req, res, max) {
    const raw = await this.readBody(req, max);
    if (raw !== TOO_BIG) return raw;
    if (!res.destroyed) fail(res, 413, 'too-large', `The request is larger than ${max} bytes`, { Connection: 'close' });
    return undefined;
  }

  // The body as JSON, or undefined after answering 413 or 422.
  async readJson(req, res, max) {
    const raw = await this.readBody(req, max);
    if (raw === TOO_BIG) {
      if (!res.destroyed) fail(res, 413, 'too-large', `The request is larger than ${max} bytes`, { Connection: 'close' });
      return undefined;
    }
    try {
      const body = JSON.parse(raw.toString('utf8') || 'null');
      if (isObj(body)) return body;
    } catch (_) {
      // Answered below.
    }
    fail(res, 422, 'invalid', 'The body must be a JSON object');
    return undefined;
  }

  desktop(req) {
    return {
      name: this.hostname(),
      version: this.hub.deps.appVersion || '0.0.0',
      platform: process.platform,
      // Where the phone reached this computer.
      address: String((req.socket && req.socket.localAddress) || this.hub.cfg.address()).replace(/^::ffff:/, ''),
      port: (req.socket && req.socket.localPort) || (this.hub.mcp && this.hub.mcp.ports().remote) || null
    };
  }

  // ---------- POST /phone/v1/pair ----------

  async pair(req, res, ip) {
    if (!this.pairAll.allows('all') || !this.pairSource.allows(ip)) {
      const wait = this.pairSource.allows(ip) ? this.pairAll.retryAfter('all') : this.pairSource.retryAfter(ip);
      return this.refuse(req, res, 429, 'rate-limited', 'Too many tries. Wait a minute.', { 'Retry-After': wait });
    }
    this.pairAll.take('all');
    this.pairSource.take(ip);
    const body = await this.readJson(req, res, SMALL_BODY);
    if (body === undefined) return;
    const device = isObj(body.device) ? body.device : null;
    if (typeof body.code !== 'string' || body.code.length > 40 || !device || typeof device.name !== 'string') {
      return fail(res, 422, 'invalid', 'Send {code, device: {name, platform, model, appVersion}}');
    }
    const code = this.pairCode();
    if (!code) {
      this.dropCode(null);
      return fail(res, 410, 'expired', 'Start pairing again in Settings on the computer');
    }
    if (!crypto.timingSafeEqual(sha256(normalizeCode(body.code)), code.hash)) {
      code.failures++;
      if (code.failures >= PAIR_FAILURES) this.dropCode('burned');
      return fail(res, 401, 'wrong-code', 'That is not the code shown on the computer');
    }
    this.dropCode(null);
    const token = crypto.randomBytes(32).toString('base64url');
    const now = this.now();
    // Only a hash of the token is stored: this computer only ever compares tokens, and unlike secrets.encrypt a hash
    // works on Linux without a keyring. 256 random bits make a plain SHA-256 as hard to reverse as the token is to guess.
    const pairing = this.hub.cfg.addPairing({
      id: `ph_${rand(10)}`,
      name: oneLine(device.name),
      platform: oneLine(device.platform),
      model: oneLine(device.model),
      tokenHash: sha256(token).toString('hex'),
      exportUntil: code.allowExport ? now + EXPORT_WINDOW_MS : null,
      now
    });
    if (!pairing) return fail(res, 409, 'full', `${MAX_PAIRINGS} phones are paired with this computer already. Remove one in Settings first.`);
    this.refresh();
    this.emit({ event: 'paired', pairing: { id: pairing.id, name: pairing.name } });
    return json(res, 200, { pairingId: pairing.id, token, desktop: this.desktop(req), exportUntil: pairing.exportUntil });
  }

  // ---------- GET /phone/v1/hello ----------

  async hello(req, res, { pairing }) {
    const agents = {};
    await Promise.all(
      PHONE_AGENTS.map(async (id) => {
        const s = await this.hub.agentStatus(id).catch((err) => ({ state: 'unknown', detail: clip(err && err.message, 300) }));
        const c = this.hub.cfg.data[id];
        // Phone turns always ask before commands and file changes, whatever Access says here (see run()).
        agents[id] = { enabled: c.enabled !== false, state: s.state, detail: s.detail, model: c.model || '', access: 'ask' };
      })
    );
    const open = pairing.exportUntil && pairing.exportUntil > this.now();
    json(res, 200, { desktop: this.desktop(req), pairingId: pairing.id, exportUntil: open ? pairing.exportUntil : null, agents });
  }

  // ---------- GET /phone/v1/export ----------

  async export(req, res, { pairing }) {
    const until = pairing.exportUntil;
    if (!until || until <= this.now()) {
      return fail(res, 403, 'export-closed', 'Open Settings > Agents > Phone on your computer and send your accounts again.');
    }
    const bundle = this.bundle();
    // One shot: closed before it goes out, so two requests at once can't both get it. A send that breaks off opens
    // it again for what is left of the window.
    const { cfg } = this.hub;
    cfg.setExportWindow(pairing.id, null);
    res.once('finish', () => {
      cfg.setExportWindow(pairing.id, null, { exported: true, now: this.now() });
      this.log('sent the accounts and settings to', pairing.id);
      this.emit({ event: 'exported', pairing: { id: pairing.id, name: pairing.name } });
    });
    res.once('close', () => {
      if (!res.writableFinished && until > this.now() && cfg.pairing(pairing.id)) cfg.setExportWindow(pairing.id, until);
    });
    json(res, 200, bundle);
  }

  // The accounts with their secrets, the mail settings and the agent settings. Not what describes this computer only:
  // program paths, Claude's config folder, the Tailscale switches and host.
  bundle() {
    const { engine, cfg } = this.hub;
    const s = (engine && engine.settings) || {};
    const keys = ['defaultAccountId', 'language', 'theme', 'signature', 'signatureHtml', 'vips', 'spam', 'syncInterval', 'hiddenViews', 'darkEmails', 'swipeActions', 'swipeRight', 'swipeLeft', 'notifications', 'sort', 'density'];
    const settings = {};
    for (const k of keys) if (k in s) settings[k] = JSON.parse(JSON.stringify(s[k]));
    let client = null;
    try {
      client = engine && engine.google && typeof engine.google.client === 'function' ? engine.google.client() : null;
    } catch (_) {
      client = null;
    }
    const clark = cfg.get('clark');
    const openclaw = cfg.get('openclaw');
    const d = cfg.data;
    return {
      version: 1,
      exportedAt: this.now(),
      desktop: { name: this.hostname(), version: this.hub.deps.appVersion || '0.0.0' },
      google: client ? { client_id: client.id, client_secret: client.secret } : null,
      accounts: engine && typeof engine.exportAccounts === 'function' ? engine.exportAccounts() : [],
      settings,
      agents: {
        defaultAgent: d.defaultAgent,
        // The phone has no permission levels yet, only "archive, move and mark mail without asking": Auto's mail
        // part. Auto and Full permission both give it that, so the phone never does more without asking than here.
        permission: d.permission,
        autoMailActions: d.permission === 'auto' || d.permission === 'full',
        shortcuts: d.shortcuts ? JSON.parse(JSON.stringify(d.shortcuts)) : null,
        clark: { enabled: clark.enabled !== false, name: clark.name, url: clark.url, key: clark.key || '', model: clark.model },
        openclaw: { enabled: openclaw.enabled !== false, url: openclaw.url, key: openclaw.key || '', agent: openclaw.agent, model: openclaw.model },
        claude: { enabled: d.claude.enabled !== false, model: d.claude.model, access: d.claude.access },
        codex: { enabled: d.codex.enabled !== false, model: d.codex.model, access: d.codex.access },
        // Clark's bridge looks for Rukoo on this port on every device.
        mcp: { port: d.mcp.port }
      }
    };
  }

  // ---------- GET /phone/v1/agents/:agent/models|skills ----------

  async lists(req, res, { params, query }) {
    const [agent, what] = params;
    if (this.hub.cfg.data[agent].enabled === false) return fail(res, 409, 'disabled', `${AGENT_NAMES[agent]} is turned off on this computer`);
    const refresh = query.get('refresh') === '1';
    const out = what === 'models' ? await this.hub.models(agent, { refresh }) : await this.hub.ownSkills(agent, { refresh });
    json(res, 200, out);
  }

  // ---------- POST /phone/v1/turns ----------

  async startTurn(req, res, { pairing, ip, body }) {
    let t;
    try {
      t = turnRequest(body, ip);
    } catch (err) {
      if (!(err instanceof BadRequest)) throw err;
      return fail(res, 422, 'invalid', err.message);
    }
    const hostCid = `p_${pairing.id}_${t.conversation.id}`;
    if (this.byConversation.has(hostCid)) return fail(res, 409, 'busy', 'A turn of this chat is still running');
    const turn = {
      id: `t_${rand(12)}`,
      pairingId: pairing.id,
      hostCid,
      phoneCid: t.conversation.id,
      agent: t.agent,
      controller: new AbortController(),
      approvals: new Map(),
      settled: new Set(),
      startOvers: new Map(),
      events: [],
      seq: 0,
      droppedMax: 0,
      res: null,
      heartbeat: null,
      graceTimer: null,
      stopTimer: null,
      dropTimer: null,
      tempDir: null,
      done: false,
      ended: false
    };
    turn.forced = new Promise((resolve) => {
      turn.force = resolve;
    });
    this.turns.set(turn.id, turn);
    this.byConversation.set(hostCid, turn.id);
    this.chats.set(`${t.agent}\n${turn.phoneCid}`, hostCid);
    // The route first, so a tool call the agent makes right away finds the phone.
    if (t.mcp) {
      this.routes.set(hostCid, { pairingId: pairing.id, phoneCid: t.conversation.id, agent: t.agent, url: t.mcp.url, token: t.mcp.token, at: this.now() });
    }
    this.push(turn, { type: 'started', turnId: turn.id, conversationId: turn.phoneCid, agent: turn.agent });
    this.attach(turn, req, res, 0);
    this.run(turn, t).catch((err) => this.finish(turn, { status: 'error', error: hubModule().toError(err) }));
  }

  async run(turn, t) {
    const hub = this.hub;
    const { toError, ERROR_TEXT } = hubModule();
    if (hub.cfg.data[t.agent].enabled === false) {
      return this.finish(turn, { status: 'error', error: { code: 'disabled', detail: `${AGENT_NAMES[t.agent]} is turned off on this computer` } });
    }
    const adapter = hub.adapters.get(t.agent);
    if (!adapter) return this.finish(turn, { status: 'error', error: { code: 'unknown', detail: hub.adapterErrors.get(t.agent) || `${t.agent} adapter not available` } });
    // Only skills this computer lists for the agent: a name goes to Claude Code as /name.
    let skills = [];
    if (t.skills.length) {
      const list = await hub.ownSkills(t.agent).catch(() => null);
      const names = new Set((list && list.skills ? list.skills : []).map((s) => s.name));
      skills = t.skills.filter((name) => names.has(name)).map((name) => ({ name }));
    }
    if (turn.controller.signal.aborted || turn.done) return this.finish(turn, { status: 'stopped' });
    const files = this.writeImages(turn, t.files);
    // An effort the agent's current list no longer offers for the model goes, as hub.turnEffort() does.
    const offered = t.effort ? hub.effortsFor(t.agent, t.model) : null;
    const effort = offered && !offered.includes(t.effort) ? null : t.effort;
    // Only a session or thread this computer gave this chat of this phone. Any other could be a desktop chat's or
    // another phone's: resuming it would hand that chat's history to this phone, and a thread's tool calls with it.
    // Without one the agent starts a new session, as for a lost one.
    const provider = this.ownProvider(turn, t.conversation.provider);
    if (provider.threadId) this.mapThread(provider.threadId, turn.hostCid);
    const kind = t.agent === 'codex' ? 'threadId' : 'sessionId';
    const dropped = !t.firstTurn && Boolean(t.conversation.provider[kind]) && !provider[kind];
    const conversation = { id: turn.hostCid, agent: t.agent, title: t.conversation.title, provider, message: null, items: [] };
    const port = hub.mcp ? hub.mcp.ports().local : null;
    // The agent calls this Rukoo's loopback listener, as in a chat of its own; route() sends the calls to the phone.
    // The phone's token never reaches a command line or a Codex rollout file. Claude gets a token for this chat alone;
    // Codex signs in with the token of its app-server.
    const mcp = t.mcp && port ? { url: `http://127.0.0.1:${port}/mcp`, token: t.agent === 'claude' ? this.tokenFor(turn.hostCid, t.agent) : '' } : { url: '', token: '' };
    const handle = {
      id: turn.id,
      conversation,
      input: t.input,
      instructions: t.instructions || context.instructions(),
      text: t.text,
      action: t.action,
      skills,
      firstTurn: t.firstTurn,
      model: t.model,
      effort,
      // A turn from the phone asks before commands and file changes, whatever Access says in Settings: a paired
      // phone's token should not be enough to run anything on this computer unseen.
      access: 'ask',
      files,
      emails: t.emails,
      mcp,
      emit: (event) => this.onAdapterEvent(turn, event, ERROR_TEXT),
      approve: (request) => this.approve(turn, request),
      setProvider: (patch) => {
        if (!isObj(patch) || turn.done) return;
        const clean = {};
        for (const k of ['sessionId', 'threadId']) if (typeof patch[k] === 'string' && patch[k]) clean[k] = clip(patch[k], 200);
        if (!Object.keys(clean).length) return;
        Object.assign(conversation.provider, clean);
        this.bindProvider(turn, clean);
        if (clean.threadId) this.mapThread(clean.threadId, turn.hostCid);
        this.push(turn, { type: 'provider', patch: clean });
      },
      accepted: () => {
        if (turn.accepted || turn.done) return;
        turn.accepted = true;
        this.push(turn, { type: 'accepted' });
      },
      startOver: () => this.startOver(turn, handle),
      signal: turn.controller.signal
    };
    // As the adapter does for a lost session: the new one gets the email and a recap from the phone.
    if (dropped) {
      const text = t.agent === 'codex' ? 'Codex started a new thread' : 'Claude started a new session';
      this.push(turn, { type: 'notice', tone: 'info', code: t.agent === 'codex' ? 'new-thread' : 'new-session', text });
      await this.startOver(turn, handle);
    }
    let result;
    try {
      result = turn.controller.signal.aborted ? { status: 'stopped' } : await Promise.race([adapter.runTurn(handle), turn.forced]);
    } catch (err) {
      result = { status: 'error', error: toError(err) };
    }
    this.finish(turn, result);
  }

  // The images go into a folder of their own under Claude's --add-dir, for the adapters to send along as images.
  writeImages(turn, list) {
    if (!list.length) return [];
    const dir = path.join(this.hub.tempBase(), `phone-${rand(10)}`);
    fs.mkdirSync(dir, { recursive: true });
    this.hub.tempDirs.add(dir);
    turn.tempDir = dir;
    return list.map((f, i) => {
      const file = path.join(dir, `${i + 1}-${safeName(f.name)}`);
      fs.writeFileSync(file, f.data);
      return { id: f.id, name: f.name, type: f.type, kind: 'image', size: f.data.length, path: file, inline: true };
    });
  }

  removeTemp(turn) {
    const dir = turn.tempDir;
    if (!dir) return;
    turn.tempDir = null;
    fs.rm(dir, { recursive: true, force: true }, () => this.hub.tempDirs.delete(dir));
  }

  tokenFor(hostCid, agent) {
    let token = this.tokens.get(hostCid);
    if (!token) {
      token = this.hub.issueToken({ agent, conversationId: hostCid });
      this.tokens.set(hostCid, token);
    }
    return token;
  }

  mapThread(threadId, hostCid) {
    const key = String(threadId || '');
    if (key && !this.hub.threads.has(key)) this.threads.set(key, hostCid);
  }

  // ---------- sessions and threads ----------

  // What PROVIDER_FILE holds, without ids of pairings that are gone.
  loadProviderIds() {
    let stored = null;
    try {
      stored = JSON.parse(fs.readFileSync(this.providerFile, 'utf8'));
    } catch (_) {
      stored = null;
    }
    const ids = new Map();
    for (const r of isObj(stored) && Array.isArray(stored.ids) ? stored.ids.slice(-MAX_PROVIDER_IDS) : []) {
      if (!isObj(r) || !PROVIDER_KINDS.includes(r.kind) || !providerId(r.id) || !PHONE_AGENTS.includes(r.agent)) continue;
      if (typeof r.conversationId !== 'string' || !CONVERSATION_ID.test(r.conversationId) || typeof r.pairingId !== 'string' || !this.hub.cfg.pairing(r.pairingId)) continue;
      ids.set(`${r.kind}\n${r.id}`, { kind: r.kind, id: r.id, pairingId: r.pairingId, conversationId: r.conversationId, agent: r.agent });
    }
    return ids;
  }

  saveProviderIds() {
    try {
      writeJsonAtomic(this.providerFile, { version: 1, ids: [...this.providerIds.values()] });
    } catch (err) {
      this.log('could not save the sessions of phone chats:', err && err.message);
    }
  }

  // The adapter gave the turn's chat a session or thread: the phone may send it back for this chat, and only this one.
  bindProvider(turn, patch) {
    // A turn of a phone removed meanwhile is still stopping.
    if (!this.hub.cfg.pairing(turn.pairingId)) return;
    let changed = false;
    for (const kind of PROVIDER_KINDS) {
      if (!providerId(patch[kind])) continue;
      const key = `${kind}\n${patch[kind]}`;
      const was = this.providerIds.get(key);
      const r = { kind, id: patch[kind], pairingId: turn.pairingId, conversationId: turn.phoneCid, agent: turn.agent };
      this.providerIds.delete(key);
      this.providerIds.set(key, r);
      if (!was || was.pairingId !== r.pairingId || was.conversationId !== r.conversationId || was.agent !== r.agent) changed = true;
    }
    if (!changed) return;
    // Past the cap, the chats used longest ago start a new session next time.
    for (const key of this.providerIds.keys()) {
      if (this.providerIds.size <= MAX_PROVIDER_IDS) break;
      this.providerIds.delete(key);
    }
    this.saveProviderIds();
  }

  // The session and thread the phone sent back, as far as this computer gave them to this turn's chat: the same
  // phone, chat and agent. The rest go. An id one of this computer's own chats has never counts.
  ownProvider(turn, provider) {
    const out = {};
    for (const kind of PROVIDER_KINDS) {
      const id = provider[kind];
      const key = `${kind}\n${id}`;
      const r = id ? this.providerIds.get(key) : null;
      if (!r || r.pairingId !== turn.pairingId || r.conversationId !== turn.phoneCid || r.agent !== turn.agent || this.desktopHas(kind, id)) continue;
      out[kind] = id;
      // In use again: the last to go past the cap.
      this.providerIds.delete(key);
      this.providerIds.set(key, r);
    }
    return out;
  }

  desktopHas(kind, id) {
    if (kind === 'threadId' && this.hub.threads.has(id)) return true;
    for (const c of this.hub.conversations.values()) if (c.provider && c.provider[kind] === id) return true;
    return false;
  }

  // The phone deleted a chat, or was removed: its sessions and threads may not be resumed from a phone again.
  dropProviderIds(match) {
    const before = this.providerIds.size;
    for (const [key, r] of [...this.providerIds]) if (match(r)) this.providerIds.delete(key);
    if (this.providerIds.size !== before) this.saveProviderIds();
  }

  // The adapter's events as the phone's hub takes them (hub.js onAdapterEvent), with their strings clipped.
  onAdapterEvent(turn, event, ERROR_TEXT) {
    if (turn.done || !isObj(event) || !EVENT_TYPES.has(event.type)) return;
    let out;
    switch (event.type) {
      case 'text':
      case 'thinking':
        out = { type: event.type, delta: clip(event.delta, 40000) };
        break;
      case 'commentary':
        out = { type: 'commentary', text: clip(event.text, 40000) };
        break;
      case 'thinking-done':
        out = { type: 'thinking-done' };
        break;
      case 'tool-start':
        out = { type: 'tool-start', key: clip(event.key, 200), name: clip(event.name || 'tool', 200), detail: clip(event.detail, 2000) };
        break;
      case 'tool-end':
        out = { type: 'tool-end', key: clip(event.key, 200), error: Boolean(event.error), ...(event.detail ? { detail: clip(event.detail, 2000) } : {}) };
        break;
      case 'error': {
        const code = typeof event.code === 'string' && Object.prototype.hasOwnProperty.call(ERROR_TEXT, event.code) ? event.code : 'unknown';
        out = { type: 'error', code, detail: clip(event.detail, 500) };
        break;
      }
      case 'notice':
        out = { type: 'notice', text: clip(event.text, 500), tone: ['info', 'success', 'error'].includes(event.tone) ? event.tone : 'info', code: event.code ? clip(event.code, 60) : null };
        break;
      case 'usage': {
        let data;
        try {
          data = JSON.stringify(event);
        } catch (_) {
          return;
        }
        if (data.length > 4000) return;
        out = JSON.parse(data);
        break;
      }
    }
    this.push(turn, out);
  }

  // ---------- the stream ----------

  push(turn, event) {
    if (turn.ended) return;
    const ev = { seq: ++turn.seq, ...event };
    turn.events.push(ev);
    if (turn.events.length > MAX_EVENTS) {
      // Streamed text goes first: the phone continues without it. Approvals, provider and done stay longest.
      let at = turn.events.findIndex((e) => e.type === 'text' || e.type === 'thinking');
      if (at < 0) at = 0;
      const [gone] = turn.events.splice(at, 1);
      turn.droppedMax = Math.max(turn.droppedMax, gone.seq);
    }
    if (turn.res) turn.res.write(line(ev));
  }

  // Sends the events after seq `after` on res, then what comes. A newer connection replaces an older one.
  attach(turn, req, res, after) {
    if (turn.res && turn.res !== res) this.detach(turn, true);
    clearTimeout(turn.graceTimer);
    turn.graceTimer = null;
    res.writeHead(200, { 'Content-Type': 'application/x-ndjson; charset=utf-8', 'Cache-Control': 'no-store' });
    if (req.socket) req.socket.setNoDelay(true);
    res.flushHeaders();
    if (turn.droppedMax > after) res.write(line({ type: 'gap' }));
    for (const ev of turn.events) if (ev.seq > after) res.write(line(ev));
    if (turn.ended) return res.end();
    turn.res = res;
    turn.heartbeat = setInterval(() => res.write(PING), this.waits.heartbeat);
    if (turn.heartbeat.unref) turn.heartbeat.unref();
    res.on('close', () => this.streamClosed(turn, res));
  }

  // end: close the connection from this side (a reattach replaced it, or the turn is over).
  detach(turn, end) {
    const res = turn.res;
    turn.res = null;
    clearInterval(turn.heartbeat);
    turn.heartbeat = null;
    if (res && end) res.end();
  }

  // The phone lost the stream: the turn keeps running for REATTACH_GRACE_MS, then stops.
  streamClosed(turn, res) {
    if (turn.res !== res) return;
    this.detach(turn, false);
    if (turn.done) return;
    turn.graceTimer = setTimeout(() => {
      turn.graceTimer = null;
      if (!turn.res) this.stopTurn(turn);
    }, this.waits.grace);
    if (turn.graceTimer.unref) turn.graceTimer.unref();
  }

  finish(turn, result) {
    if (turn.done) return;
    turn.done = true;
    clearTimeout(turn.stopTimer);
    clearTimeout(turn.graceTimer);
    this.expireAll(turn);
    for (const done of [...turn.startOvers.values()]) done(null);
    const r = isObj(result) ? result : { status: 'error', error: { code: 'protocol', detail: 'the adapter returned nothing' } };
    const status = ['done', 'stopped', 'error'].includes(r.status) ? r.status : 'error';
    const error = status === 'error' ? hubModule().toError(r.error || { code: 'unknown' }) : null;
    this.push(turn, { type: 'done', status, ...(error ? { error } : {}) });
    turn.ended = true;
    if (this.byConversation.get(turn.hostCid) === turn.id) this.byConversation.delete(turn.hostCid);
    const route = this.routes.get(turn.hostCid);
    if (route) route.at = this.now();
    this.detach(turn, true);
    this.removeTemp(turn);
    turn.dropTimer = setTimeout(() => this.turns.delete(turn.id), KEEP_FINISHED_MS);
    if (turn.dropTimer.unref) turn.dropTimer.unref();
  }

  stopTurn(turn) {
    if (turn.done) return;
    turn.controller.abort();
    this.expireAll(turn);
    for (const done of [...turn.startOvers.values()]) done(null);
    if (!turn.stopTimer) {
      turn.stopTimer = setTimeout(() => turn.force({ status: 'stopped' }), this.waits.stop);
      if (turn.stopTimer.unref) turn.stopTimer.unref();
    }
  }

  ownTurn(pairing, turnId) {
    const turn = this.turns.get(turnId);
    return turn && turn.pairingId === pairing.id ? turn : null;
  }

  // GET /phone/v1/turns/:turnId/events?after=<seq>
  async events(req, res, { pairing, params, query }) {
    const turn = this.ownTurn(pairing, params[0]);
    if (!turn) return fail(res, 404, 'not-found', 'No such turn, or it ended more than two minutes ago');
    const after = Number.parseInt(query.get('after') || '0', 10);
    this.attach(turn, req, res, Number.isFinite(after) && after > 0 ? after : 0);
  }

  // POST /phone/v1/turns/:turnId/stop
  async stop(req, res, { pairing, params }) {
    const turn = this.ownTurn(pairing, params[0]);
    if (!turn) return fail(res, 404, 'not-found', 'No such turn');
    this.stopTurn(turn);
    // The stream carries the outcome.
    json(res, 200, { ok: true });
  }

  // ---------- approvals ----------

  approve(turn, request) {
    const { card, refusal } = hubModule().approvalCard(isObj(request) ? request : {});
    Object.assign(card, { source: turn.agent, kind: 'runtime' });
    const approvalId = `a_${rand(10)}`;
    const answered = (choice) => {
      const p = Promise.resolve(choice);
      p.itemId = approvalId;
      return p;
    };
    if (turn.done) return answered('cancel');
    if (refusal) {
      turn.settled.add(approvalId);
      this.push(turn, { type: 'approval', approvalId, card, status: 'denied', decision: refusal.id, declinedBy: 'rukoo' });
      return answered(refusal.id);
    }
    let resolve;
    const promise = new Promise((r) => {
      resolve = r;
    });
    // Adapters hand this back to hub.expireApproval() when the agent withdraws the request.
    promise.itemId = approvalId;
    turn.approvals.set(approvalId, { resolve, key: request && request.key != null ? String(request.key) : null, choices: card.choices });
    this.push(turn, { type: 'approval', approvalId, card, status: 'pending' });
    return promise;
  }

  // POST /phone/v1/turns/:turnId/approvals/:approvalId {choice}
  async decide(req, res, { pairing, params, body }) {
    const turn = this.ownTurn(pairing, params[0]);
    if (!turn) return fail(res, 404, 'not-found', 'No such turn');
    const approvalId = params[1];
    const w = turn.approvals.get(approvalId);
    if (!w) return turn.settled.has(approvalId) ? json(res, 200, { status: 'settled' }) : fail(res, 404, 'not-found', 'No such approval');
    const choice = typeof body.choice === 'string' ? body.choice : '';
    if (!w.choices.some((c) => c.id === choice)) return fail(res, 422, 'invalid', 'That is not one of the choices on the card');
    turn.approvals.delete(approvalId);
    turn.settled.add(approvalId);
    w.resolve(choice);
    this.push(turn, { type: 'approval-decided', approvalId, decision: choice });
    json(res, 200, { ok: true });
  }

  expire(turn, approvalId, w) {
    turn.approvals.delete(approvalId);
    turn.settled.add(approvalId);
    w.resolve('cancel');
    this.push(turn, { type: 'approval-expired', approvalId });
  }

  expireAll(turn) {
    for (const [approvalId, w] of [...turn.approvals]) this.expire(turn, approvalId, w);
  }

  // From hub.expireApproval(): the agent withdrew a request (claude.js control_cancel_request, Codex's
  // serverRequest/resolved). True for every chat of a phone, so the hub looks no further.
  expireApproval(cid, itemOrKey) {
    if (!this.owns(cid)) return false;
    const turn = this.turns.get(this.byConversation.get(cid));
    if (!turn) return true;
    for (const [approvalId, w] of turn.approvals) {
      if (approvalId !== itemOrKey && (w.key === null || w.key !== String(itemOrKey))) continue;
      this.expire(turn, approvalId, w);
      break;
    }
    return true;
  }

  // ---------- start over ----------

  // The agent lost the chat's session and starts a new one (hub.js startOver). The phone has the transcript, so it
  // writes the first turn with the email and a recap, and sends it to start-over/:requestId. Never throws.
  startOver(turn, handle) {
    if (turn.done || turn.controller.signal.aborted) return Promise.resolve(handle.input);
    const requestId = `r_${rand(10)}`;
    return new Promise((resolve) => {
      const done = (input, timedOut = false) => {
        clearTimeout(timer);
        turn.startOvers.delete(requestId);
        if (typeof input === 'string') handle.input = input;
        else if (timedOut) this.push(turn, { type: 'notice', tone: 'info', code: 'start-over-missed', text: 'The new session did not get the email again' });
        resolve(handle.input);
      };
      const timer = setTimeout(() => done(null, true), this.waits.startOver);
      if (timer.unref) timer.unref();
      turn.startOvers.set(requestId, done);
      this.push(turn, { type: 'start-over', requestId });
    });
  }

  // POST /phone/v1/turns/:turnId/start-over/:requestId {input}
  async startOverReply(req, res, { pairing, params, body }) {
    const turn = this.ownTurn(pairing, params[0]);
    const done = turn && turn.startOvers.get(params[1]);
    if (!done) return fail(res, 404, 'not-found', 'No such request, or it timed out');
    if (typeof body.input !== 'string' || !body.input || body.input.length > INPUT_MAX) return fail(res, 422, 'invalid', `input must be text of at most ${INPUT_MAX} characters`);
    done(body.input);
    json(res, 200, { ok: true });
  }

  // ---------- chats and pairing (phone side) ----------

  // POST /phone/v1/conversations/forget {agent, conversationId}
  async forget(req, res, { pairing, body }) {
    if (!PHONE_AGENTS.includes(body.agent) || typeof body.conversationId !== 'string' || !CONVERSATION_ID.test(body.conversationId)) {
      return fail(res, 422, 'invalid', 'Send {agent: "claude" or "codex", conversationId}');
    }
    this.forgetChat(`p_${pairing.id}_${body.conversationId}`, body.agent);
    this.dropProviderIds((r) => r.pairingId === pairing.id && r.conversationId === body.conversationId && r.agent === body.agent);
    json(res, 200, { ok: true });
  }

  // DELETE /phone/v1/pairing
  async unpairSelf(req, res, { pairing }) {
    this.unpair(pairing.id, 'phone');
    json(res, 200, { ok: true });
  }

  // ---------- Rukoo's tools ----------

  // Where a tool call from Claude Code or Codex goes: the route of a phone's chat, GONE for a phone's chat without
  // one, or null for this computer's own (tools.callTool). Only loopback identities: the CLIs run here.
  route(identity, args, meta) {
    if (!identity || identity.remote) return null;
    const cid = identity.conversationId;
    // Claude's token is for one chat.
    if (cid) return this.owns(cid) ? this.routes.get(cid) || GONE : null;
    // Codex's token is shared: its thread id comes from Codex, not the model, so it counts before conversation_id.
    const m = isObj(meta) ? meta : {};
    const codex = isObj(m['x-codex-turn-metadata']) ? m['x-codex-turn-metadata'].thread_id : null;
    const threadId = m.threadId || codex;
    if (threadId) {
      const key = String(threadId);
      if (this.hub.threads.has(key)) return null;
      const hostCid = this.threads.get(key);
      if (hostCid) return this.routes.get(hostCid) || GONE;
    }
    const asked = isObj(args) && typeof args.conversation_id === 'string' ? args.conversation_id : '';
    if (!asked || this.hub.conversations.has(asked)) return null;
    const hits = [...this.routes.values()].filter((r) => r.phoneCid === asked && r.agent === identity.agent);
    if (hits.length === 1) return hits[0];
    return !hits.length && this.chats.has(`${identity.agent}\n${asked}`) ? GONE : null;
  }

  // Sends tools/call to the phone's MCP server with the token the phone gave for this chat, and returns its result as
  // it is. The phone finds the chat by _meta "rukoo/conversation", which this computer sets, not by arguments.
  async forward(route, identity, name, args, meta) {
    const hub = this.hub;
    if (route === GONE || !isObj(route) || !route.url) {
      return errorResult(`Rukoo could not run ${name}: this chat belongs to Rukoo on a phone, and the phone's connection to this chat has ended. Ask the user to send the message again from the phone.`);
    }
    // A custom agent's settings are in the custom list (config.agentData); a removed one has none.
    const agentCfg = hub.cfg && (typeof hub.cfg.agentData === 'function' ? hub.cfg.agentData(identity.agent) : hub.cfg.data && hub.cfg.data[identity.agent]);
    if (agentCfg && agentCfg.enabled === false) {
      return errorResult(`${hub.agentName(identity.agent)} is turned off in Rukoo, so Rukoo's tools are not available. The user can turn it on in Settings > Agents.`);
    }
    const failed = (detail) => errorResult(`Rukoo on your phone could not run ${name}: ${detail}`);
    if (!this.enabled() || !hub.cfg.pairing(route.pairingId)) return failed('this computer no longer lets the phone use it');
    route.at = this.now();
    const id = `f_${rand(10)}`;
    const message = {
      jsonrpc: '2.0',
      id,
      method: 'tools/call',
      params: {
        name,
        arguments: isObj(args) ? args : {},
        _meta: { ...(isObj(meta) ? meta : {}), 'rukoo/conversation': route.phoneCid, 'rukoo/agent': route.agent, 'rukoo/host': this.hostname() }
      }
    };
    let res;
    try {
      res = await this.fetch(route.url, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Accept: 'application/json, text/event-stream',
          Authorization: `Bearer ${route.token}`,
          'MCP-Protocol-Version': '2025-06-18'
        },
        body: JSON.stringify(message),
        redirect: 'error',
        signal: AbortSignal.timeout(FORWARD_TIMEOUT_MS)
      });
    } catch (err) {
      const timedOut = err && (err.name === 'TimeoutError' || err.name === 'AbortError');
      return failed(timedOut ? `no answer within ${FORWARD_TIMEOUT_MS / 1000} s` : describe(err));
    }
    let text;
    try {
      text = await readAll(res, FORWARD_MAX);
    } catch (err) {
      return failed(err instanceof AgentError ? err.detail : `the answer broke off: ${describe(err)}`);
    }
    if (res.status === 401) return failed('the phone did not accept the token of this chat');
    if (res.status !== 200) return failed(`HTTP ${res.status}`);
    const reply = parseReply(text, res.headers.get('content-type'), id);
    if (!reply) return failed('the phone sent an answer this computer did not understand');
    if (reply.error !== undefined) return failed(clip(oneLine((isObj(reply.error) && reply.error.message) || reply.error), 300) || 'error');
    if (!isObj(reply.result)) return failed('the phone sent an answer this computer did not understand');
    return reply.result;
  }
}

// The response body as text, up to max bytes. Larger answers fail rather than arrive cut.
async function readAll(res, max) {
  if (!res.body) return '';
  const reader = res.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > max) throw new AgentError('protocol', `the answer is larger than ${Math.round(max / 1024 / 1024)} MB`);
      chunks.push(value);
    }
  } finally {
    reader.cancel().catch(() => {});
  }
  return Buffer.concat(chunks).toString('utf8');
}

// A JSON-RPC answer, plain or as Server-Sent Events (Streamable HTTP lets a server choose); null when it is neither.
function parseReply(text, type, id) {
  const parse = (s) => {
    try {
      const v = JSON.parse(s);
      return isObj(v) ? v : null;
    } catch (_) {
      return null;
    }
  };
  if (!/text\/event-stream/i.test(String(type || ''))) return parse(text);
  let found = null;
  for (const block of String(text).split(/\r?\n\r?\n/)) {
    const data = block
      .split(/\r?\n/)
      .filter((l) => l.startsWith('data:'))
      .map((l) => l.slice(5).replace(/^ /, ''))
      .join('\n');
    const msg = data ? parse(data) : null;
    if (msg && msg.id === id && ('result' in msg || 'error' in msg)) found = msg;
  }
  return found;
}

// The checked parts of a turn request (docs/agents.md); throws BadRequest with what is wrong.
function turnRequest(body, ip) {
  const { cleanModel } = hubModule();
  const text = (v, max, field, required = false) => {
    if (v == null && !required) return '';
    if (typeof v !== 'string' || (required && !v) || v.length > max) bad(`${field} must be text of at most ${max} characters`);
    return v;
  };
  if (!PHONE_AGENTS.includes(body.agent)) bad('agent must be claude or codex');
  const c = isObj(body.conversation) ? body.conversation : bad('conversation is missing');
  if (typeof c.id !== 'string' || !CONVERSATION_ID.test(c.id)) bad('conversation.id must be 1 to 100 letters, digits, - or _');
  const provider = {};
  if (isObj(c.provider)) {
    for (const k of PROVIDER_KINDS) if (providerId(c.provider[k])) provider[k] = c.provider[k];
  }
  const skills = body.skills == null ? [] : body.skills;
  if (!Array.isArray(skills) || skills.length > MAX_SKILLS) bad(`skills must be a list of at most ${MAX_SKILLS}`);
  const files = body.files == null ? [] : body.files;
  if (!Array.isArray(files) || files.length > MAX_FILES) bad(`files must be a list of at most ${MAX_FILES}`);
  const emails = body.emails == null ? [] : body.emails;
  if (!Array.isArray(emails) || emails.length > MAX_EMAILS || emails.some((e) => typeof e !== 'string' || !e || e.length > 4000)) bad(`emails must be a list of at most ${MAX_EMAILS} ids`);
  let effort = null;
  if (body.effort != null && body.effort !== '') {
    if (!EFFORTS.includes(body.effort)) bad(`effort must be one of ${EFFORTS.join(', ')}`);
    effort = body.effort;
  }
  let model = null;
  try {
    model = cleanModel(body.model);
  } catch (_) {
    bad('model must be a model name of at most 200 characters');
  }
  return {
    agent: body.agent,
    conversation: { id: c.id, title: oneLine(typeof c.title === 'string' ? c.title : '').slice(0, 80), provider },
    // The phone builds the input as Rukoo does for an agent elsewhere (context.turnText with local false): it names
    // no paths on the phone, and email inside it is in <unsafe_content> tags.
    input: text(body.input, INPUT_MAX, 'input', true),
    text: text(body.text, TEXT_MAX, 'text'),
    instructions: text(body.instructions, INSTRUCTIONS_MAX, 'instructions') || null,
    action: text(body.action, 40, 'action') || null,
    skills: skills.map((s) => (isObj(s) && typeof s.name === 'string' && s.name && s.name.length <= 200 ? s.name : bad('each skill is {name}'))),
    firstTurn: body.firstTurn === true,
    model,
    effort,
    files: files.map((f, i) => phoneImage(f, i)),
    emails: emails.slice(),
    mcp: body.mcp == null ? null : phoneMcp(body.mcp, ip)
  };
}

function phoneImage(f, i) {
  if (!isObj(f) || typeof f.data !== 'string') bad(`files[${i}] must be {id, name, type, kind: "image", data}`);
  if (f.kind != null && f.kind !== 'image') bad(`files[${i}]: only images go along with a turn`);
  // Room for line breaks: Android's Base64.DEFAULT wraps at 76 characters.
  if (f.data.length > Math.ceil((INLINE_IMAGE_MAX * 4) / 3) * 1.05 + 16 || !/^[A-Za-z0-9+/=\s]*$/.test(f.data)) bad(`files[${i}] is not base64 of at most ${INLINE_IMAGE_MAX} bytes`);
  const data = Buffer.from(f.data, 'base64');
  const type = imageType(data);
  if (!type) bad(`files[${i}] is not a PNG, JPEG, GIF or WebP image`);
  if (data.length > INLINE_IMAGE_MAX) bad(`files[${i}] is larger than ${INLINE_IMAGE_MAX} bytes`);
  const id = typeof f.id === 'string' && /^[A-Za-z0-9_-]{1,100}$/.test(f.id) ? f.id : `phone-${i + 1}`;
  const name = oneLine(typeof f.name === 'string' ? f.name : '').slice(0, 200) || `image-${i + 1}`;
  return { id, name, type, data };
}

// The phone's own MCP server. Its address must be the one this request came from, so a paired phone can't make this
// computer post tool calls to another host on the tailnet.
function phoneMcp(m, ip) {
  if (!isObj(m)) bad('mcp must be {url, token}');
  let u;
  try {
    u = new URL(String(m.url || ''));
  } catch (_) {
    u = null;
  }
  const port = u ? Number(u.port) : 0;
  const ok =
    u &&
    u.protocol === 'http:' &&
    !u.username &&
    !u.password &&
    u.pathname === '/mcp' &&
    !u.search &&
    !u.hash &&
    /^\d{1,3}(\.\d{1,3}){3}$/.test(u.hostname) &&
    u.hostname === ip &&
    Number.isInteger(port) &&
    port >= 1024 &&
    port <= 65535;
  if (!ok) bad(`mcp.url must be http://${ip}:<port>/mcp, on the address this request came from`);
  if (typeof m.token !== 'string' || !m.token || m.token.length > 512 || /\s/.test(m.token)) bad('mcp.token must be a token of at most 512 characters');
  return { url: `http://${u.hostname}:${port}/mcp`, token: m.token };
}

module.exports = {
  PhoneHost,
  // media.js reuses these for its links' rate limits.
  Limiter,
  peerOf,
  normalizeCode,
  imageType,
  CODE_ALPHABET,
  PAIR_TTL_MS,
  PAIR_FAILURES,
  EXPORT_WINDOW_MS,
  HEARTBEAT_MS,
  REATTACH_GRACE_MS,
  STOP_GRACE_MS,
  START_OVER_WAIT_MS,
  PHONE_TURN_MAX,
  MAX_TURNS_PER_PHONE,
  MAX_TURNS_ALL,
  MAX_PROVIDER_IDS
};
