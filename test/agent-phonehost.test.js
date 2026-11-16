'use strict';

// The phone host (src/main/agents/phonehost.js): pairing, the routes under /phone/v1 on the Tailscale listener, turns
// streamed as NDJSON, approvals, stop, reattach, start-over, the export, and Rukoo's tool calls forwarded to the phone.
// A real hub with stub Claude and Codex adapters; the Tailscale listener is a second listener on 127.0.0.1, and the
// phone's MCP server a small HTTP server here.

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const http = require('http');
const crypto = require('crypto');
const { tempDir } = require('./tmp');
const { AgentHub } = require('../src/main/agents/hub');
const { AgentConfig, MAX_PAIRINGS } = require('../src/main/agents/config');
const { Engine } = require('../src/main/engine');
const { ClaudeAdapter } = require('../src/main/agents/claude');
const { CodexAdapter } = require('../src/main/agents/codex');
const context = require('../src/main/agents/context');
const { PhoneHost, normalizeCode, imageType, MAX_PROVIDER_IDS } = require('../src/main/agents/phonehost');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==';
const DEVICE = { name: 'Galaxy Z Fold7', platform: 'android', model: 'SM-F966B', appVersion: '0.1.0' };
const PHONE_CID = 'c_mh2k3l9a_x8f2qa';
const PHONE_TOKEN = 'phone-issued-token-0123456789';
const ALLOW = [
  { id: 'allow', label: 'Allow', kind: 'primary' },
  { id: 'deny', label: 'Deny', kind: 'danger' }
];

// Stands in for ClaudeAdapter and CodexAdapter: each test gives it a script for runTurn.
class Stub {
  constructor({ id, hub }) {
    this.id = id;
    this.hub = hub;
    this.turns = [];
    this.forgotten = [];
    this.script = async () => ({ status: 'done' });
  }

  async status() {
    return { state: 'ready', detail: `stub ${this.id}` };
  }

  async models() {
    return { models: [{ id: 'opus', label: 'Opus', efforts: ['low', 'high'] }], efforts: ['low', 'high'], custom: ['low', 'high'] };
  }

  async skills() {
    return [{ name: 'pdf', description: 'Read PDFs', scope: 'user' }];
  }

  forget(c) {
    this.forgotten.push(c.id);
  }

  async runTurn(turn) {
    this.turns.push(turn);
    return this.script(turn);
  }
}

async function freePort() {
  const s = http.createServer();
  await new Promise((r) => s.listen(0, '127.0.0.1', r));
  const port = s.address().port;
  await new Promise((r) => s.close(r));
  return port < 1024 ? 47899 : port;
}

// A hub whose Tailscale listener is a second listener on 127.0.0.1, with 127.0.0.1 counting as a tailnet address.
async function setup(t, { phone = true, clock = null, isPeer = (ip) => ip === '127.0.0.1', secrets, accounts, settings, google = null } = {}) {
  const dir = tempDir('sem-phone-');
  const data = path.join(dir, 'data');
  fs.mkdirSync(data, { recursive: true });
  if (accounts) fs.writeFileSync(path.join(data, 'accounts.json'), JSON.stringify(accounts));
  if (settings) fs.writeFileSync(path.join(data, 'settings.json'), JSON.stringify(settings));
  fs.writeFileSync(path.join(data, 'agents.json'), JSON.stringify({ mcp: { port: await freePort() }, phone: { enabled: phone } }));
  const engine = new Engine({ dataDir: data, secrets, google }).init();
  const adapters = {};
  const hub = new AgentHub({
    engine,
    dataDir: data,
    secrets,
    deps: {
      appVersion: '9.8.7',
      // A Tailscale address Rukoo can't bind: start() leaves the Tailscale listener to the test.
      interfaces: () => ({ tailscale0: [{ family: 'IPv4', address: '100.64.0.7', internal: false }] }),
      tempDir: path.join(dir, 'tmp'),
      workspace: path.join(dir, 'workspace'),
      skills: { bundled: path.join(dir, 'no-skills'), user: path.join(dir, 'no-user-skills') },
      phone: {
        isPeer,
        now: clock ? () => clock.t : undefined,
        hostname: () => 'test-desk',
        graceMs: 300,
        stopGraceMs: 300,
        startOverWaitMs: 400
      }
    },
    adapters: {
      claude: (o) => (adapters.claude = new Stub(o)),
      codex: (o) => (adapters.codex = new Stub(o))
    }
  });
  await hub.start();
  const remote = await hub.mcp.listen({ host: '127.0.0.1', port: 0, remote: true });
  const events = [];
  hub.on('event', (e) => e.kind === 'phone' && events.push(e));
  t.after(async () => {
    await hub.dispose();
    await engine.close();
  });
  return { hub, engine, adapters, remote, local: hub.mcp.ports().local, data, dir, events };
}

function call(port, method, p, { token, body, headers = {} } = {}) {
  return new Promise((resolve, reject) => {
    const data = body === undefined ? null : typeof body === 'string' ? body : JSON.stringify(body);
    const h = { ...(token ? { authorization: `Bearer ${token}` } : {}), ...(data ? { 'content-type': 'application/json', 'content-length': Buffer.byteLength(data) } : {}), ...headers };
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers: h, agent: false }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let json;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (_) {
          json = undefined;
        }
        resolve({ status: res.statusCode, headers: res.headers, text, json });
      });
    });
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

// An NDJSON stream: events as they come, next(pred) waits for one, drop() cuts the connection like a lost network.
function stream(port, method, p, { token, body } = {}) {
  const events = [];
  const waiters = [];
  let status = null;
  let headers = null;
  let done;
  const ended = new Promise((r) => {
    done = r;
  });
  const check = () => {
    for (const w of [...waiters]) {
      const ev = events.find(w.pred);
      if (!ev) continue;
      waiters.splice(waiters.indexOf(w), 1);
      clearTimeout(w.timer);
      w.resolve(ev);
    }
  };
  const data = body === undefined ? null : JSON.stringify(body);
  const req = http.request(
    {
      host: '127.0.0.1',
      port,
      method,
      path: p,
      agent: false,
      headers: { authorization: `Bearer ${token}`, ...(data ? { 'content-type': 'application/json', 'content-length': Buffer.byteLength(data) } : {}) }
    },
    (res) => {
      status = res.statusCode;
      headers = res.headers;
      let buf = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => {
        buf += chunk;
        let i;
        while ((i = buf.indexOf('\n')) >= 0) {
          const l = buf.slice(0, i);
          buf = buf.slice(i + 1);
          if (l.trim()) events.push(JSON.parse(l));
        }
        check();
      });
      res.on('end', () => {
        if (buf.trim()) events.push(JSON.parse(buf));
        check();
        done();
      });
      res.on('error', () => done());
      res.on('close', () => done());
    }
  );
  req.on('error', () => done());
  if (data) req.write(data);
  req.end();
  return {
    events,
    ended,
    get status() {
      return status;
    },
    get headers() {
      return headers;
    },
    next(pred, ms = 5000) {
      const ev = events.find(pred);
      if (ev) return Promise.resolve(ev);
      return new Promise((resolve, reject) => {
        const w = { pred, resolve };
        w.timer = setTimeout(() => reject(new Error(`timed out; events so far: ${JSON.stringify(events)}`)), ms);
        waiters.push(w);
      });
    },
    drop() {
      req.destroy();
    },
    types() {
      return events.filter((e) => e.type !== 'ping').map((e) => e.type);
    }
  };
}

// Rukoo's MCP server on the phone: answer(message, req) gives {status, type, body}.
async function phoneServer(answer = (msg) => ({ body: { jsonrpc: '2.0', id: msg.id, result: { content: [{ type: 'text', text: 'from the phone' }] } } })) {
  const seen = [];
  const phone = { seen, answer };
  const server = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', async () => {
      const body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      seen.push({ headers: req.headers, body });
      const out = await phone.answer(body, req);
      res.writeHead(out.status || 200, { 'content-type': out.type || 'application/json' });
      res.end(typeof out.body === 'string' ? out.body : JSON.stringify(out.body));
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  phone.url = `http://127.0.0.1:${server.address().port}/mcp`;
  phone.close = () =>
    new Promise((r) => {
      if (!server.listening) return r();
      server.close(() => r());
      server.closeAllConnections();
    });
  return phone;
}

async function pair(env, { allowExport = false, device = DEVICE } = {}) {
  const shown = await env.hub.phonePairStart({ allowExport });
  const res = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: shown.code, device } });
  assert.equal(res.status, 200, res.text);
  return res.json;
}

const turnBody = (phone, extra = {}) => ({
  agent: 'claude',
  conversation: { id: PHONE_CID, title: 'Re: invoice', provider: {} },
  input: `[Rukoo conversation ${PHONE_CID}. Pass conversation_id "${PHONE_CID}" to rukoo tools.]\n\nhello`,
  text: 'hello',
  firstTurn: true,
  mcp: phone ? { url: phone.url, token: PHONE_TOKEN } : null,
  ...extra
});

const rpc = (id, method, params) => ({ jsonrpc: '2.0', id, method, params });

// ---------- pairing ----------

test('pairing: the code from Settings pairs once, in any case and with O, I or L typed; the token is stored as a hash only', async (t) => {
  const env = await setup(t);
  const shown = await env.hub.phonePairStart({ allowExport: false });
  assert.match(shown.code, /^[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}$/);
  assert.equal(shown.address, '100.64.0.7');
  assert.equal(shown.port, env.remote);
  assert.equal(shown.url, `http://100.64.0.7:${env.remote}`);
  assert.equal(env.hub.config().phone.pairingActive, true);
  const typed = ` ${shown.code.toLowerCase().replace('-', ' ').replace(/0/g, 'o').replace(/1/g, 'l')} `;
  assert.equal(normalizeCode(typed), shown.code.replace('-', ''));
  const res = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: typed, device: DEVICE } });
  assert.equal(res.status, 200, res.text);
  assert.match(res.json.pairingId, /^ph_[a-z0-9]{10}$/);
  assert.match(res.json.token, /^[A-Za-z0-9_-]{43}$/);
  assert.deepEqual(res.json.desktop, { name: 'test-desk', version: '9.8.7', platform: process.platform, address: '127.0.0.1', port: env.remote });
  assert.equal(res.json.exportUntil, null, 'Pair only');
  assert.deepEqual(env.events.map((e) => e.event), ['paired']);
  assert.deepEqual(env.events[0].pairing, { id: res.json.pairingId, name: 'Galaxy Z Fold7' });
  assert.equal(env.hub.config().phone.pairingActive, false, 'the code is used up');

  const hash = crypto.createHash('sha256').update(res.json.token).digest('hex');
  const stored = fs.readFileSync(path.join(env.data, 'agents.json'), 'utf8');
  assert.ok(!stored.includes(res.json.token), 'never the token itself');
  assert.ok(stored.includes(hash));
  const view = env.hub.config().phone.pairings;
  assert.equal(view.length, 1);
  assert.equal(view[0].name, 'Galaxy Z Fold7');
  assert.equal(view[0].platform, 'android');
  assert.equal(view[0].model, 'SM-F966B');
  assert.equal(view[0].exportOpen, false);
  assert.ok(!JSON.stringify(env.hub.config()).includes(hash), 'no hash for the renderer');

  const again = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: shown.code, device: DEVICE } });
  assert.equal(again.status, 410);
  assert.equal(again.json.error, 'expired');

  const hello = await call(env.remote, 'GET', '/phone/v1/hello', { token: res.json.token });
  assert.equal(hello.status, 200);
  assert.equal(hello.json.pairingId, res.json.pairingId);
  assert.equal(hello.json.desktop.name, 'test-desk');
  assert.deepEqual(hello.json.agents.claude, { enabled: true, state: 'ready', detail: 'stub claude', model: '', access: 'ask' });
  assert.equal(hello.json.agents.codex.access, 'ask', 'phone turns always ask');
  assert.equal(hello.json.exportUntil, null);

  // Kept after a restart.
  const reloaded = new AgentConfig({ file: path.join(env.data, 'agents.json') }).load();
  assert.equal(reloaded.pairing(res.json.pairingId).name, 'Galaxy Z Fold7');
});

test('pairing: five wrong codes burn it, an expired code is gone, and a bad body is 422', async (t) => {
  const clock = { t: Date.now() };
  const env = await setup(t, { clock });
  const shown = await env.hub.phonePairStart({});
  const wrong = shown.code.startsWith('AAAA') ? 'BBBB-BBBB' : 'AAAA-AAAA';
  for (let i = 1; i <= 5; i++) {
    const r = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: wrong, device: DEVICE } });
    assert.equal(r.status, 401, `try ${i}`);
    assert.equal(r.json.error, 'wrong-code');
  }
  assert.deepEqual(env.events.map((e) => e.event), ['burned'], 'Settings says the code was entered wrong too often');
  clock.t += 61000;
  const right = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: shown.code, device: DEVICE } });
  assert.equal(right.status, 410, 'burned, even for the right code');

  const next = await env.hub.phonePairStart({});
  clock.t += 5 * 60 * 1000 + 1;
  const late = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: next.code, device: DEVICE } });
  assert.equal(late.status, 410);
  assert.equal(late.json.error, 'expired');
  assert.equal(env.hub.config().phone.pairings.length, 0);

  await env.hub.phonePairStart({});
  for (const body of [{ code: 'AAAA-AAAA' }, { code: 5, device: DEVICE }, { code: 'x'.repeat(41), device: DEVICE }, '[]']) {
    const r = await call(env.remote, 'POST', '/phone/v1/pair', { body });
    assert.equal(r.status, 422, JSON.stringify(body));
    assert.equal(r.json.error, 'invalid');
  }
  clock.t += 61000;
  assert.equal((await call(env.remote, 'POST', '/phone/v1/pair', { body: 'not json' })).status, 422);
  // Cancel in Settings: the code stops working.
  const cancelled = await env.hub.phonePairStart({});
  env.hub.phonePairCancel();
  assert.equal((await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: cancelled.code, device: DEVICE } })).status, 410);
});

test('pairing: five tries a minute from one address, then 429 with Retry-After', async (t) => {
  const clock = { t: Date.now() };
  const env = await setup(t, { clock });
  const shown = await env.hub.phonePairStart({});
  const wrong = shown.code.startsWith('AAAA') ? 'BBBB-BBBB' : 'AAAA-AAAA';
  for (let i = 0; i < 4; i++) assert.equal((await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: wrong, device: DEVICE } })).status, 401);
  // The fifth try is the right one; the sixth is over the limit whatever it holds.
  assert.equal((await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: shown.code, device: DEVICE } })).status, 200);
  const next = await env.hub.phonePairStart({});
  const limited = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: next.code, device: DEVICE } });
  assert.equal(limited.status, 429);
  assert.equal(limited.json.error, 'rate-limited');
  assert.ok(Number(limited.headers['retry-after']) >= 1);
  clock.t += 60000;
  assert.equal((await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: next.code, device: DEVICE } })).status, 200, 'a minute later it works');
  assert.equal(env.hub.config().phone.pairings.length, 2);
});

// ---------- access ----------

test('access: phone routes only on the Tailscale listener, only while switched on, only with a pairing token', async (t) => {
  const clock = { t: Date.now() };
  const env = await setup(t, { clock });
  const { token } = await pair(env);
  assert.equal((await call(env.remote, 'GET', '/phone/v1/hello', { token })).status, 200);
  // Not on the loopback listener.
  assert.equal((await call(env.local, 'GET', '/phone/v1/hello', { token })).status, 404);
  // Not a token for /mcp, on either listener.
  for (const port of [env.local, env.remote]) {
    const r = await call(port, 'POST', '/mcp', { token, body: rpc(1, 'tools/list', {}) });
    assert.equal(r.status, 401, `port ${port}`);
  }
  assert.equal((await call(env.remote, 'GET', '/phone/v1/hello')).status, 401);
  const unknown = await call(env.remote, 'GET', '/phone/v1/hello', { token: 'x'.repeat(43) });
  assert.equal(unknown.status, 401);
  assert.deepEqual(unknown.json, { error: 'unauthorized' });
  assert.equal((await call(env.remote, 'GET', '/phone/v1/nope', { token })).status, 404);
  const method = await call(env.remote, 'POST', '/phone/v1/hello', { token, body: {} });
  assert.equal(method.status, 405);
  assert.equal(method.headers.allow, 'GET');
  // A browser page can't get in: Origin is refused before the phone routes.
  assert.equal((await call(env.remote, 'GET', '/phone/v1/hello', { token, headers: { origin: 'https://evil.example' } })).status, 403);

  // Switched off: as if nothing were there, for the pairing route too.
  env.hub.cfg.update({ phone: { enabled: false } });
  for (const [verb, p] of [
    ['GET', '/phone/v1/hello'],
    ['POST', '/phone/v1/pair'],
    ['GET', '/phone/v1/export']
  ]) {
    const r = await call(env.remote, verb, p, { token, body: verb === 'POST' ? { code: 'AAAA-AAAA', device: DEVICE } : undefined });
    assert.equal(r.status, 404, p);
    assert.deepEqual(r.json, { error: 'not found' });
  }
  env.hub.cfg.update({ phone: { enabled: true } });

  // Twenty wrong tokens in a minute lock the address out for five minutes, valid token or not.
  clock.t += 61000;
  for (let i = 0; i < 21; i++) assert.equal((await call(env.remote, 'GET', '/phone/v1/hello', { token: `wrong-${i}` })).status, 401);
  const blocked = await call(env.remote, 'GET', '/phone/v1/hello', { token });
  assert.equal(blocked.status, 429);
  assert.ok(Number(blocked.headers['retry-after']) > 200);
  clock.t += 5 * 60 * 1000 + 1000;
  assert.equal((await call(env.remote, 'GET', '/phone/v1/hello', { token })).status, 200);

  // At most 60 requests a minute per phone.
  clock.t += 61000;
  for (let i = 0; i < 60; i++) assert.equal((await call(env.remote, 'GET', '/phone/v1/export', { token })).status, 403);
  const over = await call(env.remote, 'GET', '/phone/v1/export', { token });
  assert.equal(over.status, 429);
  assert.equal(over.json.error, 'rate-limited');
});

test('access: an address outside the tailnet gets 404', async (t) => {
  const env = await setup(t, { isPeer: () => false });
  const shown = await env.hub.phonePairStart({});
  const r = await call(env.remote, 'POST', '/phone/v1/pair', { body: { code: shown.code, device: DEVICE } });
  assert.equal(r.status, 404);
  assert.equal(env.hub.config().phone.pairings.length, 0);
});

test('access: Settings turns the switch on to pair, and says so when there is no Tailscale listener', async (t) => {
  const env = await setup(t, { phone: false });
  await env.hub.mcp.closeRemote();
  assert.equal(env.hub.wantsRemote(), false);
  // updateConfig opens the Tailscale listener, which can't bind 100.64.0.7 here: no listener, no code.
  await assert.rejects(env.hub.phonePairStart({}), (err) => err.code === 'not-configured');
  assert.equal(env.hub.cfg.data.phone.enabled, true, 'switched on');
  assert.equal(env.hub.wantsRemote(), true, 'the phone opens the Tailscale listener');
  assert.equal(env.hub.phoneHost.pairCode(), null);
});

// ---------- export ----------

const enc = (s) => `enc:${Buffer.from(s).toString('base64')}`;
const SECRETS = {
  encrypt: enc,
  decrypt: (v) => {
    if (v === 'enc:other-user') throw new Error('DPAPI: wrong user');
    return String(v).startsWith('enc:') ? Buffer.from(String(v).slice(4), 'base64').toString() : v;
  }
};
const ACCOUNTS = [
  {
    id: 'a1',
    type: 'imap',
    provider: 'other',
    email: 'me@example.com',
    name: 'Me',
    label: 'Work',
    color: '#4f7cff',
    signature: null,
    imap: { host: 'imap.example.com', port: 993, secure: true, user: 'me' },
    smtp: { host: 'smtp.example.com', port: 465, secure: true, user: 'me' },
    secret: enc('imap-password'),
    createdAt: 1,
    caldav: { url: 'https://dav.example.com', home: '/me/' },
    aliases: [{ address: 'sales@example.com', name: 'Sales' }],
    defaultFrom: 'sales@example.com'
  },
  {
    id: 'a2',
    type: 'imap',
    provider: 'google',
    auth: 'oauth2',
    email: 'me@gmail.com',
    name: 'Me',
    label: 'Gmail',
    color: '#e5484d',
    signature: 'Bart',
    imap: { host: 'imap.gmail.com', port: 993, secure: true, user: 'me@gmail.com' },
    smtp: { host: 'smtp.gmail.com', port: 465, secure: true, user: 'me@gmail.com' },
    secret: enc('refresh-token'),
    scopes: ['email', 'https://mail.google.com/'],
    createdAt: 2
  },
  { id: 'a3', type: 'imap', provider: 'other', email: 'old@example.com', name: 'Old', label: 'Old', color: '#999', signature: null, imap: { host: 'imap.example.com', port: 993, secure: true, user: 'old' }, smtp: null, secret: 'enc:other-user', createdAt: 3 },
  { id: 'a4', type: 'demo', provider: 'demo', email: 'demo@rukoo.app', name: 'Demo', label: 'Demo', color: '#888', signature: null, imap: null, smtp: null, secret: null, createdAt: 4 }
];

test('export: one shot in the window after pairing, with the secrets; closed afterwards until Settings opens it again', async (t) => {
  const env = await setup(t, {
    secrets: SECRETS,
    accounts: ACCOUNTS,
    settings: { vips: ['boss@example.com'], spam: ['spam@example.com'], language: 'nl', syncInterval: 15, darkEmails: false, swipeActions: false, swipeRight: 'archive', swipeLeft: 'star' },
    google: { client: () => ({ id: 'client.apps.googleusercontent.com', secret: 'client-secret' }) }
  });
  env.hub.setSecret('clark', 'hermes-key');
  env.hub.setSecret('openclaw', 'gateway-token');
  env.hub.cfg.update({ claude: { exe: 'C:\\tools\\claude.exe', configDir: 'C:\\claude', model: 'opus', access: 'full' }, defaultAgent: 'claude' });

  const paired = await pair(env, { allowExport: true });
  assert.ok(paired.exportUntil > Date.now() + 9 * 60 * 1000 && paired.exportUntil <= Date.now() + 10 * 60 * 1000);
  assert.equal(env.hub.config().phone.pairings[0].exportOpen, true);
  const hello = await call(env.remote, 'GET', '/phone/v1/hello', { token: paired.token });
  assert.equal(hello.json.exportUntil, paired.exportUntil);

  const res = await call(env.remote, 'GET', '/phone/v1/export', { token: paired.token });
  assert.equal(res.status, 200, res.text);
  const b = res.json;
  assert.equal(b.version, 1);
  assert.deepEqual(b.desktop, { name: 'test-desk', version: '9.8.7' });
  assert.deepEqual(b.google, { client_id: 'client.apps.googleusercontent.com', client_secret: 'client-secret' });
  const [imap, gmail, other, demo] = b.accounts;
  assert.equal(imap.password, 'imap-password');
  assert.equal(imap.auth, 'password');
  assert.equal(imap.refreshToken, undefined);
  assert.deepEqual(imap.caldav, { url: 'https://dav.example.com', home: '/me/' });
  assert.deepEqual(imap.aliases, [{ address: 'sales@example.com', name: 'Sales' }]);
  assert.equal(imap.defaultFrom, 'sales@example.com');
  assert.deepEqual(imap.imap, { host: 'imap.example.com', port: 993, secure: true, user: 'me' });
  assert.equal(gmail.refreshToken, 'refresh-token');
  assert.equal(gmail.auth, 'oauth2');
  assert.equal(gmail.password, undefined);
  assert.deepEqual(gmail.scopes, ['email', 'https://mail.google.com/']);
  assert.equal(other.secretError, 'decrypt-failed', 'a secret this computer cannot decrypt is marked, not sent');
  assert.equal(other.password, undefined);
  assert.equal(demo.auth, 'none');
  assert.equal(demo.password, undefined);
  assert.ok(!JSON.stringify(b.accounts).includes('enc:'), 'never the encrypted form');
  assert.deepEqual(b.settings.vips, ['boss@example.com']);
  assert.equal(b.settings.language, 'nl');
  assert.equal(b.settings.syncInterval, 15);
  assert.equal(b.settings.darkEmails, false);
  // The phone takes the swipe actions too (DesktopImport.kt).
  assert.equal(b.settings.swipeActions, false);
  assert.equal(b.settings.swipeRight, 'archive');
  assert.equal(b.settings.swipeLeft, 'star');
  assert.equal(b.agents.defaultAgent, 'claude');
  assert.deepEqual(b.agents.clark, { enabled: true, name: 'Hermes', url: '', key: 'hermes-key', model: '' });
  assert.equal(b.agents.openclaw.key, 'gateway-token');
  assert.deepEqual(b.agents.claude, { enabled: true, model: 'opus', access: 'full' }, 'no program path or config folder');
  assert.deepEqual(b.agents.mcp, { port: env.hub.cfg.data.mcp.port });

  await sleep(20);
  assert.deepEqual(env.events.map((e) => e.event), ['paired', 'exported']);
  const view = env.hub.config().phone.pairings[0];
  assert.equal(view.exportOpen, false);
  assert.ok(view.lastExportAt >= paired.exportUntil - 10 * 60 * 1000);

  const twice = await call(env.remote, 'GET', '/phone/v1/export', { token: paired.token });
  assert.equal(twice.status, 403);
  assert.equal(twice.json.error, 'export-closed');
  assert.match(twice.json.detail, /Settings > Agents > Phone/);

  env.hub.phoneAllowExport(paired.pairingId);
  assert.equal(env.hub.config().phone.pairings[0].exportOpen, true);
  assert.equal((await call(env.remote, 'GET', '/phone/v1/export', { token: paired.token })).status, 200);
  assert.throws(() => env.hub.phoneAllowExport('ph_nosuchphone'), (err) => err.code === 'invalid');

  const only = await pair(env);
  assert.equal((await call(env.remote, 'GET', '/phone/v1/export', { token: only.token })).status, 403, 'Pair only sends nothing');
});

test('export: swipe actions the user never changed go as the desktop defaults', async (t) => {
  const env = await setup(t);
  const { settings } = env.hub.phoneHost.bundle();
  assert.equal(settings.swipeActions, true);
  assert.equal(settings.swipeRight, 'read');
  assert.equal(settings.swipeLeft, 'delete');
});

// ---------- turns ----------

test('turn: events stream as NDJSON with increasing seq, and the adapter gets a handle like a desktop chat with access ask', async (t) => {
  const env = await setup(t);
  const phone = await phoneServer();
  t.after(() => phone.close());
  const { token, pairingId } = await pair(env);
  const hostCid = `p_${pairingId}_${PHONE_CID}`;
  let imageSeen = null;
  env.adapters.claude.script = async (turn) => {
    imageSeen = turn.files.map((f) => ({ ...f, exists: fs.existsSync(f.path), bytes: fs.readFileSync(f.path).length }));
    turn.setProvider({ sessionId: 'sess-1', other: 'dropped' });
    turn.accepted();
    turn.accepted();
    turn.emit({ type: 'thinking', delta: 'Reading the email' });
    turn.emit({ type: 'thinking-done' });
    turn.emit({ type: 'tool-start', key: 'k1', name: 'mcp__rukoo__get_context', detail: 'x'.repeat(3000) });
    turn.emit({ type: 'tool-end', key: 'k1', error: false });
    turn.emit({ type: 'text', delta: 'Hello ' });
    turn.emit({ type: 'text', delta: 'there' });
    turn.emit({ type: 'commentary', text: 'Checking' });
    turn.emit({ type: 'notice', tone: 'loud', code: 'new-session', text: 'Claude started a new session' });
    turn.emit({ type: 'usage', costUsd: 0.01, usage: { input_tokens: 5 } });
    turn.emit({ type: 'something-new', x: 1 });
    turn.emit({ type: 'error', code: 'made-up', detail: 'odd' });
    return { status: 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', {
    token,
    body: turnBody(phone, {
      model: 'opus',
      effort: 'high',
      skills: [{ name: 'pdf' }, { name: 'not-a-skill' }],
      text: 'hello $pdf',
      input: 'Rukoo text\n\nhello $pdf',
      files: [{ id: 'f1', name: 'shot.png', type: 'image/jpeg', kind: 'image', data: PNG }]
    })
  });
  await s.ended;
  assert.equal(s.status, 200);
  assert.match(s.headers['content-type'], /^application\/x-ndjson/);
  assert.equal(s.headers['cache-control'], 'no-store');
  assert.deepEqual(s.types(), ['started', 'provider', 'accepted', 'thinking', 'thinking-done', 'tool-start', 'tool-end', 'text', 'text', 'commentary', 'notice', 'usage', 'error', 'done']);
  assert.deepEqual(
    s.events.map((e) => e.seq),
    s.events.map((_, i) => i + 1)
  );
  const [started, provider] = s.events;
  assert.match(started.turnId, /^t_[a-z0-9]{12}$/);
  assert.equal(started.conversationId, PHONE_CID);
  assert.equal(started.agent, 'claude');
  assert.deepEqual(provider.patch, { sessionId: 'sess-1' });
  assert.equal(s.events.find((e) => e.type === 'tool-start').detail.length, 2000, 'clipped');
  assert.equal(s.events.find((e) => e.type === 'notice').tone, 'info');
  assert.deepEqual(s.events.find((e) => e.type === 'usage'), { seq: 12, type: 'usage', costUsd: 0.01, usage: { input_tokens: 5 } });
  assert.equal(s.events.find((e) => e.type === 'error').code, 'unknown');
  assert.deepEqual(s.events.at(-1), { seq: 14, type: 'done', status: 'done' });

  const h = env.adapters.claude.turns[0];
  assert.equal(h.conversation.id, hostCid);
  assert.equal(h.conversation.title, 'Re: invoice');
  assert.equal(h.input, 'Rukoo text\n\nhello $pdf');
  assert.equal(h.text, 'hello $pdf');
  assert.equal(h.instructions, context.INSTRUCTIONS);
  assert.deepEqual(h.skills, [{ name: 'pdf' }], 'only skills this computer lists');
  assert.equal(h.firstTurn, true);
  assert.equal(h.model, 'opus');
  assert.equal(h.effort, 'high');
  assert.equal(h.access, 'ask');
  assert.equal(h.mcp.url, `http://127.0.0.1:${env.local}/mcp`, "this Rukoo's loopback listener, not the phone");
  assert.notEqual(h.mcp.token, PHONE_TOKEN, "the phone's token stays out of the command line");
  assert.deepEqual(env.hub.identify(h.mcp.token), { agent: 'claude', conversationId: hostCid, remote: false });
  assert.equal(imageSeen.length, 1);
  assert.equal(imageSeen[0].type, 'image/png', 'the type from the bytes, not the phone');
  assert.equal(imageSeen[0].kind, 'image');
  assert.equal(imageSeen[0].inline, true);
  assert.equal(imageSeen[0].name, 'shot.png');
  assert.equal(imageSeen[0].exists, true);
  assert.equal(imageSeen[0].bytes, Buffer.from(PNG, 'base64').length);
  assert.ok(imageSeen[0].path.startsWith(env.hub.tempBase()), "inside Claude's --add-dir");
  await sleep(100);
  assert.equal(fs.existsSync(imageSeen[0].path), false, 'removed when the turn ends');

  // The same chat again: the token stays the same, so Claude's process stays.
  env.adapters.claude.script = async () => ({ status: 'error', error: { code: 'offline', detail: 'no network' } });
  const again = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(phone, { conversation: { id: PHONE_CID, title: 'Re: invoice', provider: { sessionId: 'sess-1' } }, firstTurn: false }) });
  await again.ended;
  assert.deepEqual(again.events.at(-1).error, { code: 'offline', detail: 'no network' });
  assert.equal(env.adapters.claude.turns[1].mcp.token, h.mcp.token);
  assert.deepEqual(env.adapters.claude.turns[1].conversation.provider, { sessionId: 'sess-1' });
  assert.equal(env.adapters.claude.turns[1].firstTurn, false);
});

test('turn: requests that are refused before a stream starts', async (t) => {
  const env = await setup(t);
  const phone = await phoneServer();
  t.after(() => phone.close());
  const { token } = await pair(env);
  const bad = async (body, why) => {
    const r = await call(env.remote, 'POST', '/phone/v1/turns', { token, body });
    assert.equal(r.status, 422, `${why}: ${r.text}`);
    assert.equal(r.json.error, 'invalid');
    return r.json.detail;
  };
  // The phone's MCP server must be on the address the request came from, so the phone can't aim this computer at
  // another host on the tailnet.
  assert.match(await bad(turnBody({ url: 'http://100.64.0.9:47800/mcp' }), 'another host'), /mcp\.url must be http:\/\/127\.0\.0\.1:<port>\/mcp/);
  await bad(turnBody(null, { mcp: { url: 'http://127.0.0.1:80/mcp', token: PHONE_TOKEN } }), 'a low port');
  await bad(turnBody(null, { mcp: { url: 'http://127.0.0.1:47800/other', token: PHONE_TOKEN } }), 'another path');
  await bad(turnBody(null, { mcp: { url: 'https://127.0.0.1:47800/mcp', token: PHONE_TOKEN } }), 'https');
  await bad(turnBody(null, { mcp: { url: 'http://localhost:47800/mcp', token: PHONE_TOKEN } }), 'a name');
  await bad(turnBody(null, { mcp: { url: 'http://user:pw@127.0.0.1:47800/mcp', token: PHONE_TOKEN } }), 'credentials');
  await bad(turnBody(null, { mcp: { url: phone.url, token: '' } }), 'no token');
  await bad(turnBody(phone, { agent: 'clark' }), 'not an agent of this computer');
  await bad(turnBody(phone, { conversation: { id: 'has space' } }), 'conversation id');
  await bad(turnBody(phone, { input: '' }), 'empty input');
  await bad(turnBody(phone, { input: 'x'.repeat(400001) }), 'long input');
  await bad(turnBody(phone, { effort: 'enormous' }), 'effort');
  await bad(turnBody(phone, { files: [{ id: 'f', name: 'a.txt', kind: 'image', data: Buffer.from('plain text').toString('base64') }] }), 'not an image');
  await bad(turnBody(phone, { files: Array.from({ length: 11 }, () => ({ data: PNG })) }), 'too many files');
  assert.equal(env.adapters.claude.turns.length, 0);
  assert.equal(imageType(Buffer.from(PNG, 'base64')), 'image/png');
});

test('turn: without the phone tools the agent runs without Rukoo tools; a turned-off agent ends the stream with disabled', async (t) => {
  const env = await setup(t);
  const { token } = await pair(env);
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  await s.ended;
  assert.deepEqual(s.events.at(-1), { seq: 2, type: 'done', status: 'done' });
  assert.deepEqual(env.adapters.claude.turns[0].mcp, { url: '', token: '' });

  env.hub.cfg.update({ codex: { enabled: false } });
  const off = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null, { agent: 'codex' }) });
  await off.ended;
  assert.equal(off.events.at(-1).status, 'error');
  assert.equal(off.events.at(-1).error.code, 'disabled');
  assert.equal(env.adapters.codex.turns.length, 0);
  const models = await call(env.remote, 'GET', '/phone/v1/agents/codex/models', { token });
  assert.equal(models.status, 409);
  assert.equal(models.json.error, 'disabled');
});

test('turn: approvals go to the phone and its answer back to the adapter; one turn per chat at a time', async (t) => {
  const env = await setup(t);
  const phone = await phoneServer();
  t.after(() => phone.close());
  const { token } = await pair(env);
  const answers = [];
  let itemId = null;
  env.adapters.claude.script = async (turn) => {
    // Too long to show whole: declined without asking, with the adapter's own deny choice.
    answers.push(await turn.approve({ title: 'Claude wants to run a command', fields: [{ key: 'command', label: 'Command', value: 'x'.repeat(20001) }], choices: ALLOW, key: 'long' }));
    const p = turn.approve({ title: 'Claude wants to run a command', detail: 'List files', fields: [{ key: 'command', label: 'Command', value: 'ls -la' }], tool: { name: 'Bash', kind: 'command' }, choices: ALLOW, key: 'req-1' });
    itemId = p.itemId;
    answers.push(await p);
    turn.emit({ type: 'text', delta: answers.join(',') });
    return { status: 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(phone) });
  const denied = await s.next((e) => e.type === 'approval' && e.status === 'denied');
  assert.equal(denied.declinedBy, 'rukoo');
  assert.equal(denied.decision, 'deny');
  assert.equal(denied.card.fields[0].truncated, 1);
  const ap = await s.next((e) => e.type === 'approval' && e.status === 'pending');
  assert.equal(itemId, ap.approvalId, 'adapters pass this to hub.expireApproval');
  assert.deepEqual(ap.card, {
    title: 'Claude wants to run a command',
    detail: 'List files',
    fields: [{ key: 'command', label: 'Command', value: 'ls -la' }],
    tool: { name: 'Bash', kind: 'command' },
    choices: ALLOW,
    source: 'claude',
    kind: 'runtime'
  });
  const { turnId } = s.events[0];

  const busy = await call(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(phone) });
  assert.equal(busy.status, 409);
  assert.equal(busy.json.error, 'busy');

  const url = `/phone/v1/turns/${turnId}/approvals/${ap.approvalId}`;
  assert.equal((await call(env.remote, 'POST', url, { token, body: { choice: 'maybe' } })).status, 422);
  assert.equal((await call(env.remote, 'POST', `/phone/v1/turns/${turnId}/approvals/a_nosuchcard`, { token, body: { choice: 'allow' } })).status, 404);
  assert.equal((await call(env.remote, 'POST', `/phone/v1/turns/t_nosuchturn00/approvals/${ap.approvalId}`, { token, body: { choice: 'allow' } })).status, 404);
  // Another phone can't answer this phone's cards.
  const other = await pair(env, { device: { ...DEVICE, name: 'Pixel' } });
  assert.equal((await call(env.remote, 'POST', url, { token: other.token, body: { choice: 'deny' } })).status, 404);
  const ok = await call(env.remote, 'POST', url, { token, body: { choice: 'allow' } });
  assert.deepEqual(ok.json, { ok: true });
  await s.ended;
  assert.deepEqual(answers, ['deny', 'allow']);
  const decided = s.events.find((e) => e.type === 'approval-decided');
  assert.deepEqual(decided, { seq: decided.seq, type: 'approval-decided', approvalId: ap.approvalId, decision: 'allow' });
  assert.equal(s.events.find((e) => e.type === 'text').delta, 'deny,allow');
  assert.deepEqual((await call(env.remote, 'POST', url, { token, body: { choice: 'deny' } })).json, { status: 'settled' });
});

// Sends the headers and part of the body now; finish() sends the rest and resolves with the answer.
function slowCall(port, method, p, { token, body }) {
  const data = Buffer.from(JSON.stringify(body));
  let resolveAnswer;
  const answer = new Promise((r) => {
    resolveAnswer = r;
  });
  const req = http.request({ host: '127.0.0.1', port, method, path: p, agent: false, headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json', 'content-length': data.length } }, (res) => {
    const chunks = [];
    res.on('data', (c) => chunks.push(c));
    res.on('end', () => resolveAnswer({ status: res.statusCode, text: Buffer.concat(chunks).toString('utf8') }));
  });
  req.on('error', (err) => resolveAnswer({ status: 0, text: err.message }));
  req.write(data.subarray(0, 10));
  return {
    finish() {
      req.end(data.subarray(10));
      return answer;
    },
    // The phone hangs up halfway through the upload.
    drop() {
      req.destroy();
    }
  };
}

test('turn: a request authenticated before the phone was removed does nothing once its body arrives', async (t) => {
  const env = await setup(t);
  const { token, pairingId } = await pair(env);
  const slow = slowCall(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  await sleep(100);
  env.hub.phoneUnpair(pairingId);
  const answer = await slow.finish();
  assert.equal(answer.status, 401, answer.text);
  assert.equal(env.adapters.claude.turns.length, 0, 'no agent started');

  // The same when the switch went off meanwhile.
  const second = await pair(env, { allowExport: true });
  const late = slowCall(env.remote, 'POST', '/phone/v1/turns', { token: second.token, body: turnBody(null) });
  await sleep(100);
  env.hub.cfg.update({ phone: { enabled: false } });
  assert.equal((await late.finish()).status, 404);
  assert.equal(env.adapters.claude.turns.length, 0);
  env.hub.cfg.update({ phone: { enabled: true } });

  // A body over a route's cap is refused before the route acts: 16 KB for stop.
  let release;
  env.adapters.claude.script = (turn) =>
    new Promise((resolve) => {
      release = () => resolve({ status: 'done' });
      turn.signal.addEventListener('abort', () => resolve({ status: 'stopped' }));
    });
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token: second.token, body: turnBody(null) });
  const { turnId } = await s.next((e) => e.type === 'started');
  const big = await call(env.remote, 'POST', `/phone/v1/turns/${turnId}/stop`, { token: second.token, body: { pad: 'x'.repeat(17000) } });
  assert.equal(big.status, 413);
  assert.equal(env.adapters.claude.turns.at(-1).signal.aborted, false, 'still running');
  // A GET takes no body at all.
  assert.equal((await call(env.remote, 'GET', '/phone/v1/export', { token: second.token, body: { x: 1 } })).status, 413);
  release();
  await s.ended;
  assert.equal(s.events.at(-1).status, 'done');
});

async function until(pred, ms = 5000) {
  const end = Date.now() + ms;
  while (!pred()) {
    if (Date.now() > end) throw new Error('timed out');
    await sleep(10);
  }
}

test('turn: an upload takes its turn place before the body is read, four per phone and eight in all; refusals, hang-ups and the end free it', async (t) => {
  const env = await setup(t);
  const a = await pair(env);
  const b = await pair(env, { device: { ...DEVICE, name: 'Pixel' } });
  const c = await pair(env, { device: { ...DEVICE, name: 'Tablet' } });
  const chat = (i, extra = {}) => turnBody(null, { conversation: { id: `c_upload_${i}`, provider: {} }, ...extra });
  const start = (who, i, extra) => call(env.remote, 'POST', '/phone/v1/turns', { token: who.token, body: chat(i, extra) });
  const upload = (who, i) => slowCall(env.remote, 'POST', '/phone/v1/turns', { token: who.token, body: chat(i) });

  // Four uploads are as many turns as one phone may run: a fifth is refused before its body is kept.
  const fromA = [1, 2, 3, 4].map((i) => upload(a, i));
  await sleep(150);
  const fifth = await start(a, 5);
  assert.equal(fifth.status, 429, fifth.text);
  assert.equal(fifth.json.error, 'rate-limited');
  assert.match(fifth.json.detail, /At most 4 turns from one phone/);
  // Eight for all phones together.
  const fromB = [6, 7, 8, 9].map((i) => upload(b, i));
  await sleep(150);
  const ninth = await start(c, 10);
  assert.equal(ninth.status, 429, ninth.text);
  assert.match(ninth.json.detail, /At most 8 turns from phones/);
  assert.equal(env.adapters.claude.turns.length, 0);

  // A phone that hangs up halfway frees its place.
  fromA.shift().drop();
  await sleep(100);
  assert.equal((await start(c, 10)).status, 200);
  // So do refused turns, however many.
  for (let i = 0; i < 6; i++) assert.equal((await start(a, 11, { input: '' })).status, 422);
  assert.equal((await start(a, 11)).status, 200);

  // Uploaded, the turns run and keep their places until they end.
  let open;
  const gate = new Promise((r) => {
    open = r;
  });
  env.adapters.claude.script = (turn) => Promise.race([gate.then(() => ({ status: 'done' })), new Promise((r) => turn.signal.addEventListener('abort', () => r({ status: 'stopped' })))]);
  const answers = [...fromA, ...fromB].map((u) => u.finish());
  await until(() => env.adapters.claude.turns.length === 9);
  const eighth = stream(env.remote, 'POST', '/phone/v1/turns', { token: c.token, body: chat(12) });
  await eighth.next((e) => e.type === 'started');
  const full = await start(c, 13);
  assert.equal(full.status, 429, full.text);
  assert.match(full.json.detail, /At most 8 turns from phones/);
  open();
  for (const answer of await Promise.all(answers)) assert.equal(answer.status, 200);
  await eighth.ended;
  assert.equal(eighth.events.at(-1).status, 'done');
  assert.equal(env.hub.phoneHost.uploads.size, 0);
  assert.equal((await start(a, 14)).status, 200, 'free again once they ended');
});

test('turn: a phone resumes only the sessions and threads this computer gave that chat of that phone', async (t) => {
  const env = await setup(t);
  const answerAs = (text) => (msg) => ({ body: { jsonrpc: '2.0', id: msg.id, result: { content: [{ type: 'text', text }] } } });
  const phoneA = await phoneServer(answerAs('phone A'));
  const phoneB = await phoneServer(answerAs('phone B'));
  t.after(() => Promise.all([phoneA.close(), phoneB.close()]));
  const a = await pair(env);
  const b = await pair(env, { device: { ...DEVICE, name: 'Pixel' } });
  // What this computer gave phone A: a Claude Code session and a Codex thread.
  env.adapters.claude.script = async (turn) => {
    turn.setProvider({ sessionId: 'sess-a' });
    return { status: 'done' };
  };
  await stream(env.remote, 'POST', '/phone/v1/turns', { token: a.token, body: turnBody(phoneA) }).ended;
  env.adapters.codex.script = async (turn) => {
    turn.setProvider({ threadId: 'thr-a' });
    return { status: 'done' };
  };
  await stream(env.remote, 'POST', '/phone/v1/turns', { token: a.token, body: turnBody(phoneA, { agent: 'codex', conversation: { id: 'c_codex_a', provider: {} } }) }).ended;
  // And one of this computer's own chats.
  const desk = env.hub.create({ agent: 'claude' });
  env.hub.conversations.get(desk.id).provider.sessionId = 'sess-desk';

  let given = null;
  env.adapters.claude.script = env.adapters.codex.script = async (turn) => {
    given = { ...turn.conversation.provider };
    return { status: 'done' };
  };
  const resume = async (who, phone, agent, id, provider) => {
    given = null;
    const s = stream(env.remote, 'POST', '/phone/v1/turns', { token: who.token, body: turnBody(phone, { agent, conversation: { id, provider } }) });
    await s.ended;
    assert.equal(s.events.at(-1).status, 'done');
    return given;
  };
  assert.deepEqual(await resume(a, phoneA, 'claude', PHONE_CID, { sessionId: 'sess-a' }), { sessionId: 'sess-a' }, 'its own chat');
  assert.deepEqual(await resume(b, phoneB, 'claude', PHONE_CID, { sessionId: 'sess-a' }), {}, 'another phone, with the same chat id');
  assert.deepEqual(await resume(a, phoneA, 'claude', 'c_other_000001', { sessionId: 'sess-a' }), {}, 'another chat of the same phone');
  assert.deepEqual(await resume(a, phoneA, 'codex', PHONE_CID, { sessionId: 'sess-a' }), {}, 'another agent');
  assert.deepEqual(await resume(a, phoneA, 'claude', PHONE_CID, { sessionId: 'sess-desk' }), {}, "a desktop chat's session");
  assert.deepEqual(await resume(a, phoneA, 'claude', PHONE_CID, { sessionId: 'made-up' }), {}, 'one this computer never gave out');

  // Phone B names phone A's thread: it gets a new one, and the tool calls of A's thread still go to phone A.
  assert.deepEqual(await resume(b, phoneB, 'codex', 'c_codex_a', { threadId: 'thr-a' }), {});
  const codex = env.hub.identify(env.hub.issueToken({ agent: 'codex', conversationId: null }));
  const res = await env.hub.callTool(codex, 'get_context', {}, { 'x-codex-turn-metadata': { thread_id: 'thr-a' } });
  assert.equal(res.content[0].text, 'phone A');
  assert.equal(phoneB.seen.length, 0);
  assert.deepEqual(await resume(a, phoneA, 'codex', 'c_codex_a', { threadId: 'thr-a' }), { threadId: 'thr-a' });

  // On a later turn a dropped session goes as a lost one: the new session gets the email and a recap from the phone.
  let input = null;
  env.adapters.claude.script = async (turn) => {
    given = { ...turn.conversation.provider };
    input = turn.input;
    return { status: 'done' };
  };
  const later = stream(env.remote, 'POST', '/phone/v1/turns', { token: b.token, body: turnBody(phoneB, { conversation: { id: PHONE_CID, provider: { sessionId: 'sess-a' } }, firstTurn: false }) });
  const so = await later.next((e) => e.type === 'start-over');
  await call(env.remote, 'POST', `/phone/v1/turns/${later.events[0].turnId}/start-over/${so.requestId}`, { token: b.token, body: { input: 'FIRST TURN WITH THE EMAIL AND A RECAP' } });
  await later.ended;
  assert.deepEqual(later.types(), ['started', 'notice', 'start-over', 'done']);
  assert.equal(later.events[1].code, 'new-session');
  assert.deepEqual(given, {});
  assert.equal(input, 'FIRST TURN WITH THE EMAIL AND A RECAP');
  // Its own session goes on without one.
  const own = stream(env.remote, 'POST', '/phone/v1/turns', { token: a.token, body: turnBody(phoneA, { conversation: { id: PHONE_CID, provider: { sessionId: 'sess-a' } }, firstTurn: false }) });
  await own.ended;
  assert.deepEqual(own.types(), ['started', 'done']);
  assert.deepEqual(given, { sessionId: 'sess-a' });
});

test('turn: the sessions phones got survive a restart, go with the chat or the phone, and stay under the cap', async (t) => {
  const env = await setup(t);
  const a = await pair(env);
  const b = await pair(env, { device: { ...DEVICE, name: 'Pixel' } });
  const file = path.join(env.data, 'phone-sessions.json');
  const stored = () => JSON.parse(fs.readFileSync(file, 'utf8')).ids;
  // As after a restart of Rukoo: the phone host starts again from the file.
  const restart = () => {
    env.hub.phoneHost.dispose();
    env.hub.phoneHost = new PhoneHost(env.hub, env.hub.deps.phone);
  };
  let give = null;
  let given = null;
  env.adapters.claude.script = env.adapters.codex.script = async (turn) => {
    given = { ...turn.conversation.provider };
    if (give) turn.setProvider(give);
    return { status: 'done' };
  };
  const run = async (who, agent, id, provider, gives = null) => {
    give = gives;
    given = null;
    await stream(env.remote, 'POST', '/phone/v1/turns', { token: who.token, body: turnBody(null, { agent, conversation: { id, provider } }) }).ended;
    return given;
  };
  await run(a, 'claude', PHONE_CID, {}, { sessionId: 'sess-a' });
  await run(a, 'codex', 'c_codex_a', {}, { threadId: 'thr-a' });
  await run(b, 'claude', PHONE_CID, {}, { sessionId: 'sess-b' });
  assert.deepEqual(stored(), [
    { kind: 'sessionId', id: 'sess-a', pairingId: a.pairingId, conversationId: PHONE_CID, agent: 'claude' },
    { kind: 'threadId', id: 'thr-a', pairingId: a.pairingId, conversationId: 'c_codex_a', agent: 'codex' },
    { kind: 'sessionId', id: 'sess-b', pairingId: b.pairingId, conversationId: PHONE_CID, agent: 'claude' }
  ]);

  restart();
  assert.deepEqual(await run(a, 'claude', PHONE_CID, { sessionId: 'sess-a' }), { sessionId: 'sess-a' });
  assert.deepEqual(await run(b, 'claude', PHONE_CID, { sessionId: 'sess-a' }), {});
  assert.deepEqual(await run(a, 'codex', 'c_codex_a', { threadId: 'thr-a' }), { threadId: 'thr-a' });

  // A chat the phone deleted, and a phone that was removed, take their ids along.
  await call(env.remote, 'POST', '/phone/v1/conversations/forget', { token: a.token, body: { agent: 'claude', conversationId: PHONE_CID } });
  assert.deepEqual(await run(a, 'claude', PHONE_CID, { sessionId: 'sess-a' }), {});
  env.hub.phoneUnpair(b.pairingId);
  assert.deepEqual(
    stored().map((r) => r.id),
    ['thr-a']
  );

  // What doesn't belong to a paired phone's chat is not loaded.
  const own = { kind: 'sessionId', id: 'kept', pairingId: a.pairingId, conversationId: 'c_kept', agent: 'claude' };
  fs.writeFileSync(file, JSON.stringify({ version: 1, ids: [own, { ...own, id: 'of-b', pairingId: b.pairingId }, { ...own, id: 'odd', kind: 'cookie' }, { ...own, id: 'odd', agent: 'clark' }, { ...own, id: 'odd', conversationId: 'has space' }, { ...own, id: 'odd\n' }, 'junk'] }));
  restart();
  assert.equal(env.hub.phoneHost.providerIds.size, 1);
  assert.deepEqual(await run(a, 'claude', 'c_kept', { sessionId: 'kept' }), { sessionId: 'kept' });

  // At most MAX_PROVIDER_IDS: past it, the ones used longest ago go first.
  const old = (i) => ({ ...own, id: `old-${i}`, conversationId: `c_old_${i}` });
  fs.writeFileSync(file, JSON.stringify({ version: 1, ids: Array.from({ length: MAX_PROVIDER_IDS + 5 }, (_, i) => old(i)) }));
  restart();
  assert.equal(env.hub.phoneHost.providerIds.size, MAX_PROVIDER_IDS);
  assert.deepEqual(await run(a, 'claude', 'c_old_4', { sessionId: 'old-4' }), {}, 'past the cap when loaded');
  assert.deepEqual(await run(a, 'claude', 'c_old_5', { sessionId: 'old-5' }), { sessionId: 'old-5' }, 'now used most recently');
  await run(a, 'claude', 'c_new', {}, { sessionId: 'new' });
  const ids = stored().map((r) => r.id);
  assert.equal(ids.length, MAX_PROVIDER_IDS);
  assert.deepEqual(ids.slice(0, 2), ['old-7', 'old-8'], 'old-6 went for the new one');
  assert.deepEqual(ids.slice(-2), ['old-5', 'new']);
});

test('turn: stop expires a pending approval with cancel; an adapter that ignores stop is closed after the grace time', async (t) => {
  const env = await setup(t);
  const { token } = await pair(env);
  let choice = null;
  env.adapters.claude.script = async (turn) => {
    choice = await turn.approve({ title: 'x', choices: ALLOW, key: 'k' });
    return { status: turn.signal.aborted ? 'stopped' : 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  const ap = await s.next((e) => e.type === 'approval');
  const stop = await call(env.remote, 'POST', `/phone/v1/turns/${s.events[0].turnId}/stop`, { token, body: {} });
  assert.deepEqual(stop.json, { ok: true });
  await s.ended;
  assert.equal(choice, 'cancel');
  assert.deepEqual(s.types().slice(-2), ['approval-expired', 'done']);
  assert.equal(s.events.at(-2).approvalId, ap.approvalId);
  assert.equal(s.events.at(-1).status, 'stopped');

  env.adapters.claude.script = () => new Promise(() => {});
  const stubborn = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  const { turnId } = await stubborn.next((e) => e.type === 'started');
  await call(env.remote, 'POST', `/phone/v1/turns/${turnId}/stop`, { token, body: {} });
  await stubborn.ended;
  assert.equal(stubborn.events.at(-1).status, 'stopped');
  assert.equal(env.adapters.claude.turns[1].signal.aborted, true);
});

test('turn: a dropped stream can reattach after its last seq, live or after the end; unattached, the turn stops after the grace time', async (t) => {
  const env = await setup(t);
  const { token } = await pair(env);
  let release;
  const gate = new Promise((r) => {
    release = r;
  });
  env.adapters.claude.script = async (turn) => {
    turn.emit({ type: 'text', delta: 'one ' });
    await gate;
    turn.emit({ type: 'text', delta: 'two ' });
    await sleep(80);
    turn.emit({ type: 'text', delta: 'three' });
    return { status: 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  const first = await s.next((e) => e.type === 'text');
  const { turnId } = s.events[0];
  s.drop();
  await s.ended;
  await sleep(50);
  release();
  await sleep(10);
  // Reattached while it still runs: what came meanwhile, then live.
  const r = stream(env.remote, 'GET', `/phone/v1/turns/${turnId}/events?after=${first.seq}`, { token });
  await r.ended;
  assert.deepEqual(
    r.events.map((e) => e.delta || e.type),
    ['two ', 'three', 'done']
  );
  assert.ok(r.events.every((e) => e.seq > first.seq));
  // After the end, everything is still there for two minutes.
  const all = stream(env.remote, 'GET', `/phone/v1/turns/${turnId}/events`, { token });
  await all.ended;
  assert.deepEqual(all.types(), ['started', 'text', 'text', 'text', 'done']);
  assert.equal((await call(env.remote, 'GET', '/phone/v1/turns/t_nosuchturn00/events', { token })).status, 404);

  // Nobody comes back: stopped after the grace time (300 ms here, 30 s in Rukoo).
  env.adapters.claude.script = (turn) => new Promise((resolve) => turn.signal.addEventListener('abort', () => resolve({ status: 'stopped' })));
  const lost = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  const started = await lost.next((e) => e.type === 'started');
  lost.drop();
  await sleep(150);
  assert.equal(env.adapters.claude.turns[1].signal.aborted, false, 'still running inside the grace time');
  await sleep(400);
  assert.equal(env.adapters.claude.turns[1].signal.aborted, true);
  const after = stream(env.remote, 'GET', `/phone/v1/turns/${started.turnId}/events?after=1`, { token });
  await after.ended;
  assert.deepEqual(after.events.at(-1), { seq: 2, type: 'done', status: 'stopped' });
});

test('turn: start-over asks the phone for the first turn with the email again; without an answer the old input stays', async (t) => {
  const env = await setup(t);
  const { token } = await pair(env);
  const inputs = [];
  env.adapters.claude.script = async (turn) => {
    turn.emit({ type: 'notice', tone: 'info', code: 'new-session', text: 'Claude started a new session' });
    inputs.push(await turn.startOver());
    inputs.push(turn.input);
    return { status: 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null, { firstTurn: false }) });
  const so = await s.next((e) => e.type === 'start-over');
  assert.match(so.requestId, /^r_[a-z0-9]{10}$/);
  const { turnId } = s.events[0];
  assert.equal((await call(env.remote, 'POST', `/phone/v1/turns/${turnId}/start-over/r_nosuchreq00`, { token, body: { input: 'x' } })).status, 404);
  assert.equal((await call(env.remote, 'POST', `/phone/v1/turns/${turnId}/start-over/${so.requestId}`, { token, body: { input: '' } })).status, 422);
  const ok = await call(env.remote, 'POST', `/phone/v1/turns/${turnId}/start-over/${so.requestId}`, { token, body: { input: 'FIRST TURN WITH THE EMAIL AND A RECAP' } });
  assert.deepEqual(ok.json, { ok: true });
  await s.ended;
  assert.deepEqual(inputs, ['FIRST TURN WITH THE EMAIL AND A RECAP', 'FIRST TURN WITH THE EMAIL AND A RECAP']);

  const quiet = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null, { firstTurn: false }) });
  await quiet.ended;
  assert.equal(inputs[2], turnBody(null).input, 'the old input after 400 ms here, 60 s in Rukoo');
  const notice = quiet.events.find((e) => e.code === 'start-over-missed');
  assert.equal(notice.text, 'The new session did not get the email again');
});

// ---------- Rukoo's tools ----------

test("tools: Claude's calls in a phone chat go to the phone's MCP server with its token and _meta; tools/list stays here", async (t) => {
  const env = await setup(t);
  const phone = await phoneServer((msg) => ({ body: { jsonrpc: '2.0', id: msg.id, result: { content: [{ type: 'text', text: 'from the phone' }], structuredContent: { ok: true } } } }));
  t.after(() => phone.close());
  const { token, pairingId } = await pair(env);
  let got = null;
  let list = null;
  env.adapters.claude.script = async (turn) => {
    const res = await call(env.local, 'POST', '/mcp', {
      token: turn.mcp.token,
      body: rpc(5, 'tools/call', { name: 'get_context', arguments: { conversation_id: 'c_forged_000000' }, _meta: { progressToken: 3 } })
    });
    got = res.json;
    list = (await call(env.local, 'POST', '/mcp', { token: turn.mcp.token, body: rpc(6, 'tools/list', {}) })).json;
    return { status: 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(phone) });
  await s.ended;
  assert.deepEqual(got, { jsonrpc: '2.0', id: 5, result: { content: [{ type: 'text', text: 'from the phone' }], structuredContent: { ok: true } } });
  assert.equal(phone.seen.length, 1);
  const [{ headers, body }] = phone.seen;
  assert.equal(headers.authorization, `Bearer ${PHONE_TOKEN}`);
  assert.equal(headers['content-type'], 'application/json');
  assert.equal(headers.accept, 'application/json, text/event-stream');
  assert.equal(headers['mcp-protocol-version'], '2025-06-18');
  assert.equal(body.jsonrpc, '2.0');
  assert.equal(body.method, 'tools/call');
  assert.match(body.id, /^f_[a-z0-9]{10}$/);
  assert.deepEqual(body.params, {
    name: 'get_context',
    arguments: { conversation_id: 'c_forged_000000' },
    _meta: { progressToken: 3, 'rukoo/conversation': PHONE_CID, 'rukoo/agent': 'claude', 'rukoo/host': 'test-desk' }
  });
  assert.equal(list.result.tools.length, 22, "this computer's tool list");

  // Between turns the chat's route stays, so a late call still reaches the phone.
  const identity = env.hub.identify(env.adapters.claude.turns[0].mcp.token);
  const late = await env.hub.callTool(identity, 'search_mail', { query: 'x' }, {});
  assert.equal(late.content[0].text, 'from the phone');
  assert.equal(phone.seen[1].body.params.name, 'search_mail');

  // Forgotten by the phone: the token goes, Claude's process closes, and calls fail instead of reading this
  // computer's mail.
  const forget = await call(env.remote, 'POST', '/phone/v1/conversations/forget', { token, body: { agent: 'claude', conversationId: PHONE_CID } });
  assert.deepEqual(forget.json, { ok: true });
  await sleep(10);
  assert.deepEqual(env.adapters.claude.forgotten, [`p_${pairingId}_${PHONE_CID}`]);
  assert.equal(env.hub.identify(env.adapters.claude.turns[0].mcp.token), null);
  const gone = await env.hub.callTool(identity, 'get_context', {}, {});
  assert.equal(gone.isError, true);
  assert.match(gone.content[0].text, /belongs to Rukoo on a phone/);
  assert.equal(phone.seen.length, 2);
  assert.equal((await call(env.remote, 'POST', '/phone/v1/conversations/forget', { token, body: { agent: 'clark', conversationId: PHONE_CID } })).status, 422);
});

test("tools: Codex's shared token is routed by thread id, then by conversation_id; this computer's own chats stay here", async (t) => {
  const env = await setup(t);
  const phone = await phoneServer();
  t.after(() => phone.close());
  const { token } = await pair(env);
  env.adapters.codex.script = async (turn) => {
    turn.setProvider({ threadId: 'thr-phone' });
    return { status: 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(phone, { agent: 'codex' }) });
  await s.ended;
  assert.deepEqual(s.events.find((e) => e.type === 'provider').patch, { threadId: 'thr-phone' });
  assert.deepEqual(env.adapters.codex.turns[0].mcp, { url: `http://127.0.0.1:${env.local}/mcp`, token: '' }, 'Codex signs in with its app-server token');

  const codex = env.hub.identify(env.hub.issueToken({ agent: 'codex', conversationId: null }));
  const byThread = await env.hub.callTool(codex, 'get_context', {}, { 'x-codex-turn-metadata': { thread_id: 'thr-phone' } });
  assert.equal(byThread.content[0].text, 'from the phone');
  assert.equal(phone.seen.at(-1).body.params._meta['rukoo/agent'], 'codex');
  assert.equal(phone.seen.at(-1).body.params._meta['rukoo/conversation'], PHONE_CID);
  await env.hub.callTool(codex, 'get_context', { conversation_id: PHONE_CID }, {});
  assert.equal(phone.seen.length, 2, 'by conversation_id when no thread says otherwise');

  // A desktop Codex chat: its thread and its conversation id stay on this computer.
  const desk = env.hub.create({ agent: 'codex' });
  env.hub.mapThread('thr-desk', desk.id);
  const local = await env.hub.callTool(codex, 'get_context', { conversation_id: PHONE_CID }, { threadId: 'thr-desk' });
  assert.equal(phone.seen.length, 2);
  assert.equal(JSON.parse(local.content[0].text).conversation_id, desk.id);
  await env.hub.callTool(codex, 'get_context', { conversation_id: desk.id }, {});
  assert.equal(phone.seen.length, 2);
  // Claude's identity names no phone chat of Codex.
  await env.hub.callTool({ agent: 'claude', conversationId: null, remote: false }, 'get_context', { conversation_id: PHONE_CID }, {});
  assert.equal(phone.seen.length, 2);

  // Forgotten by the phone: Codex's shared token still works, but a call from that thread fails instead of reading
  // this computer's mail.
  await call(env.remote, 'POST', '/phone/v1/conversations/forget', { token, body: { agent: 'codex', conversationId: PHONE_CID } });
  for (const [args, meta] of [
    [{}, { 'x-codex-turn-metadata': { thread_id: 'thr-phone' } }],
    [{ conversation_id: PHONE_CID }, {}]
  ]) {
    const gone = await env.hub.callTool(codex, 'get_context', args, meta);
    assert.equal(gone.isError, true, JSON.stringify(args));
    assert.match(gone.content[0].text, /belongs to Rukoo on a phone/);
  }
  assert.equal(phone.seen.length, 2);

  // A phone can't take over a desktop chat's thread.
  let given = null;
  env.adapters.codex.script = async (turn) => {
    given = { ...turn.conversation.provider };
    return { status: 'done' };
  };
  const claim = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(phone, { agent: 'codex', conversation: { id: 'c_other_000001', provider: { threadId: 'thr-desk' } } }) });
  await claim.ended;
  assert.deepEqual(given, {});
  assert.equal(env.hub.threads.get('thr-desk'), desk.id);
});

test('tools: what the phone answers besides a result becomes a tool error that names the phone', async (t) => {
  const env = await setup(t);
  const phone = await phoneServer();
  t.after(() => phone.close());
  const { token } = await pair(env);
  await stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(phone) }).ended;
  const identity = env.hub.identify(env.adapters.claude.turns[0].mcp.token);
  const run = () => env.hub.callTool(identity, 'get_context', {}, {});
  const text = (r) => {
    assert.equal(r.isError, true, JSON.stringify(r));
    return r.content[0].text;
  };

  phone.answer = (msg) => ({ body: { jsonrpc: '2.0', id: msg.id, error: { code: -32603, message: 'the reader is closed' } } });
  assert.equal(text(await run()), 'Rukoo on your phone could not run get_context: the reader is closed');
  phone.answer = () => ({ status: 500, body: { error: 'boom' } });
  assert.equal(text(await run()), 'Rukoo on your phone could not run get_context: HTTP 500');
  phone.answer = () => ({ status: 401, body: { error: 'unauthorized' } });
  assert.match(text(await run()), /did not accept the token/);
  phone.answer = () => ({ body: 'not json' });
  assert.match(text(await run()), /did not understand/);
  // Streamable HTTP lets a server answer as Server-Sent Events.
  phone.answer = (msg) => ({ type: 'text/event-stream', body: `event: message\ndata: ${JSON.stringify({ jsonrpc: '2.0', id: msg.id, result: { content: [{ type: 'text', text: 'sse' }] } })}\n\n` });
  assert.equal((await run()).content[0].text, 'sse');
  // The agent turned off on this computer.
  env.hub.cfg.update({ claude: { enabled: false } });
  assert.match(text(await run()), /turned off in Rukoo/);
  env.hub.cfg.update({ claude: { enabled: true } });
  // The switch off: no more calls to the phone.
  env.hub.cfg.update({ phone: { enabled: false } });
  assert.match(text(await run()), /no longer lets the phone use it/);
  env.hub.cfg.update({ phone: { enabled: true } });
  const seen = phone.seen.length;
  await phone.close();
  // Refused, or on Linux a kept-alive socket the closed server dropped (undici's UND_ERR_SOCKET).
  assert.match(text(await run()), /^Rukoo on your phone could not run get_context: .*(ECONN(REFUSED|RESET)|UND_ERR_SOCKET)/);
  assert.equal(phone.seen.length, seen);
});

// ---------- hub hooks ----------

test('hub: expireApproval, unpairing and switching off reach phone turns', async (t) => {
  const env = await setup(t);
  const { token, pairingId } = await pair(env);
  const hostCid = `p_${pairingId}_${PHONE_CID}`;
  let choice = null;
  env.adapters.claude.script = async (turn) => {
    choice = await turn.approve({ title: 'x', choices: ALLOW, key: 'req-9' });
    return { status: 'done' };
  };
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  const ap = await s.next((e) => e.type === 'approval');
  // Claude's control_cancel_request.
  assert.equal(env.hub.expireApproval(hostCid, 'req-9'), true);
  await s.ended;
  assert.equal(choice, 'cancel');
  assert.equal(s.events.find((e) => e.type === 'approval-expired').approvalId, ap.approvalId);

  // The phone unpairs itself.
  env.adapters.claude.script = (turn) => new Promise((resolve) => turn.signal.addEventListener('abort', () => resolve({ status: 'stopped' })));
  const running = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  await running.next((e) => e.type === 'started');
  const del = await call(env.remote, 'DELETE', '/phone/v1/pairing', { token });
  assert.deepEqual(del.json, { ok: true });
  await running.ended;
  assert.equal(running.events.at(-1).status, 'stopped');
  assert.equal((await call(env.remote, 'GET', '/phone/v1/hello', { token })).status, 401, 'the token is gone');
  assert.deepEqual(env.events.at(-1), { kind: 'phone', event: 'removed', pairing: { id: pairingId, name: 'Galaxy Z Fold7' }, by: 'phone' });
  assert.equal(env.hub.config().phone.pairings.length, 0);

  // Removed in Settings.
  const second = await pair(env);
  assert.equal(env.hub.phoneUnpair(second.pairingId).phone.pairings.length, 0);
  assert.equal(env.events.at(-1).by, 'desktop');
  assert.throws(() => env.hub.phoneUnpair(second.pairingId), (err) => err.code === 'invalid');

  // Switched off in Settings: turns stop and a code shown stops working.
  const third = await pair(env);
  const last = stream(env.remote, 'POST', '/phone/v1/turns', { token: third.token, body: turnBody(null) });
  await last.next((e) => e.type === 'started');
  await env.hub.phonePairStart({});
  await env.hub.updateConfig({ phone: { enabled: false } });
  assert.equal(env.adapters.claude.turns.at(-1).signal.aborted, true);
  assert.equal(env.hub.phoneHost.pairCode(), null);
  assert.equal(env.hub.wantsRemote(), false);
});

test('hub: quitting ends phone streams with done/stopped', async (t) => {
  const env = await setup(t);
  const { token } = await pair(env);
  env.adapters.claude.script = () => new Promise(() => {});
  const s = stream(env.remote, 'POST', '/phone/v1/turns', { token, body: turnBody(null) });
  await s.next((e) => e.type === 'started');
  await env.hub.dispose();
  await s.ended;
  assert.deepEqual(s.events.at(-1), { seq: 2, type: 'done', status: 'stopped' });
});

test('models and skills: the lists this computer has for Claude Code and Codex', async (t) => {
  const env = await setup(t);
  const { token } = await pair(env);
  const models = await call(env.remote, 'GET', '/phone/v1/agents/claude/models?refresh=1', { token });
  assert.equal(models.status, 200);
  assert.equal(models.json.agent, 'claude');
  assert.deepEqual(
    models.json.models.map((m) => m.id),
    ['opus']
  );
  assert.deepEqual(models.json.efforts, ['low', 'high']);
  const skills = await call(env.remote, 'GET', '/phone/v1/agents/codex/skills', { token });
  assert.deepEqual(skills.json, { agent: 'codex', skills: [{ name: 'pdf', description: 'Read PDFs', scope: 'user' }], error: null });
  assert.equal((await call(env.remote, 'GET', '/phone/v1/agents/clark/models', { token })).status, 404);
});

// ---------- config and adapters ----------

test('config: pairings load only when well formed, the renderer cannot patch them, and the view has no hashes', () => {
  const dir = tempDir('sem-phone-cfg-');
  const file = path.join(dir, 'agents.json');
  const hash = 'a'.repeat(64);
  const good = { id: 'ph_abcdefghij', name: '  Galaxy\nZ  Fold7 ', platform: 'android', createdAt: 5, lastSeenAt: 6, tokenHash: hash };
  fs.writeFileSync(
    file,
    JSON.stringify({
      phone: {
        enabled: true,
        pairings: [good, { ...good }, { ...good, id: 'ph_UPPERCASE1' }, { ...good, id: 'ph_x:yzabcdef' }, { ...good, id: 'ph_bbbbbbbbbb', tokenHash: 'short' }, { ...good, id: 'ph_cccccccccc', name: 5 }, 'junk', null]
      }
    })
  );
  const cfg = new AgentConfig({ file }).load();
  assert.equal(cfg.data.phone.enabled, true);
  assert.deepEqual(
    cfg.data.phone.pairings.map((p) => p.id),
    ['ph_abcdefghij']
  );
  assert.equal(cfg.data.phone.pairings[0].name, 'Galaxy Z Fold7');
  cfg.update({ phone: { pairings: [] } });
  assert.equal(cfg.data.phone.pairings.length, 1, 'only enabled is patchable');
  assert.throws(() => cfg.update({ phone: { enabled: 'yes' } }), (err) => err.code === 'invalid');
  assert.ok(!JSON.stringify(cfg.publicView()).includes(hash));
  assert.deepEqual(Object.keys(cfg.publicView().phone.pairings[0]).sort(), ['createdAt', 'exportOpen', 'id', 'lastExportAt', 'lastSeenAt', 'model', 'name', 'platform']);
  // lastSeenAt is saved at most once a minute.
  const at = Date.now();
  assert.equal(cfg.touchPairing('ph_abcdefghij', at), true);
  assert.equal(cfg.touchPairing('ph_abcdefghij', at + 1000), false);
  assert.equal(cfg.touchPairing('ph_abcdefghij', at + 61000), true);
  let saves = 0;
  for (let t = at + 66000; t <= at + 181000; t += 5000) if (cfg.touchPairing('ph_abcdefghij', t)) saves++;
  assert.equal(saves, 2, 'polling every 5 s saves once a minute');
  assert.equal(new AgentConfig({ file }).load().pairing('ph_abcdefghij').lastSeenAt, at + 181000);
  for (let i = 1; i < MAX_PAIRINGS; i++) assert.ok(cfg.addPairing({ id: `ph_${String(i).padStart(10, '0')}`, name: `Phone ${i}`, tokenHash: hash }));
  assert.equal(cfg.addPairing({ id: 'ph_zzzzzzzzzz', name: 'One too many', tokenHash: hash }), null);
  assert.equal(new AgentConfig({ file: path.join(dir, 'none.json') }).load().data.phone.enabled, false);
});

test('adapters: a turn can lower access to ask, never raise it', () => {
  const full = { config: () => ({ access: 'full' }) };
  const ask = { config: () => ({ access: 'ask' }) };
  for (const Adapter of [ClaudeAdapter, CodexAdapter]) {
    const a = new Adapter(full);
    assert.equal(a.withChoice(a.settings(), { access: 'ask' }).access, 'ask', Adapter.name);
    assert.equal(a.withChoice(a.settings(), {}).access, 'full', Adapter.name);
    const b = new Adapter(ask);
    assert.equal(b.withChoice(b.settings(), { access: 'full' }).access, 'ask', Adapter.name);
  }
});
