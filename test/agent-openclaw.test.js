'use strict';

// The OpenClaw adapter against a scripted gateway that answers the way OpenClaw's OpenAI-compatible endpoint does
// (docs.openclaw.ai/gateway/openai-http-api; openclaw v2026.10.5 src/gateway/openai-http.ts): Bearer auth, a 404
// while the endpoint is off, /v1/models listing agent targets, SSE chat.completion.chunk frames ending in [DONE], an
// error object in the stream, and a run that is cancelled when the client closes the connection.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const http = require('http');
const path = require('path');
const { tempDir } = require('./tmp');

const { OpenClawAdapter } = require('../src/main/agents/openclaw');
const { AgentConfig, openclawTokenFor } = require('../src/main/agents/config');
const { AgentHub } = require('../src/main/agents/hub');
const { Engine } = require('../src/main/engine');

const TOKEN = 'gateway-token-123';
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// What "Copy OpenClaw setup" copies: the token in a line for the gateway's .env file, and the command, whose header
// names the variable, so the token stays out of openclaw.json (where `openclaw mcp doctor` flags it).
const openclawSetupText = (url, token) =>
  [
    '# Step 1, not a command: put this line in the OpenClaw .env file, ~/.openclaw/.env or $OPENCLAW_STATE_DIR/.env, in place of any older RUKOO_MCP_TOKEN line.',
    `RUKOO_MCP_TOKEN=${token}`,
    '# Step 2: run this command where the gateway runs, then restart the gateway.',
    `openclaw mcp set rukoo '{"url":"${url}","transport":"streamable-http","headers":{"Authorization":"Bearer \${RUKOO_MCP_TOKEN}"}}'`,
    ''
  ].join('\n');
const secrets = {
  encrypt: (s) => `enc:${Buffer.from(String(s)).toString('hex')}`,
  decrypt: (s) => (String(s).startsWith('enc:') ? Buffer.from(String(s).slice(4), 'hex').toString() : String(s))
};

async function waitFor(check, ms = 5000) {
  const until = Date.now() + ms;
  while (Date.now() < until) {
    if (check()) return true;
    await sleep(10);
  }
  throw new Error('timed out waiting');
}

async function closedPort() {
  const server = http.createServer().listen(0, '127.0.0.1');
  await new Promise((r) => server.once('listening', r));
  const { port } = server.address();
  await new Promise((r) => server.close(r));
  return `http://127.0.0.1:${port}`;
}

// A gateway with agents main (the default) and research. enabled: false answers every path with the plain 404 the
// gateway gives while gateway.http.endpoints.chatCompletions is off. The reply depends on the newest user message:
//   "slow"     streams a word, then waits until the client goes away (recorded in state.cancelled)
//   "fail"     streams a word, then the error object a failed run sends, then [DONE]
//   "cut"      streams a word and drops the connection without [DONE]
//   "silent"   sends the role chunk and nothing more
//   "archived" answers 409, as for an archived session
// Anything else streams "Turn <n> of <session key>", n counting the turns of that session.
async function gateway({ enabled = true } = {}) {
  const state = { requests: [], sessions: new Map(), cancelled: [] };
  const server = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8');
      let body = null;
      try {
        body = raw ? JSON.parse(raw) : null;
      } catch {}
      const url = new URL(req.url, 'http://x');
      state.requests.push({ method: req.method, path: url.pathname, headers: req.headers, body });
      if (!enabled) {
        res.writeHead(404, { 'Content-Type': 'text/plain' });
        return res.end('Not Found');
      }
      const json = (status, data) => {
        res.writeHead(status, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify(data));
      };
      if (req.headers.authorization !== `Bearer ${TOKEN}`) return json(401, { error: { message: 'Unauthorized', type: 'unauthorized' } });
      if (req.method === 'GET' && url.pathname === '/v1/models') {
        const model = (id) => ({ id, object: 'model', created: 0, owned_by: 'openclaw' });
        return json(200, { object: 'list', data: [model('openclaw'), model('openclaw/default'), model('openclaw/main'), model('openclaw/research')] });
      }
      if (req.method !== 'POST' || url.pathname !== '/v1/chat/completions') return json(404, { error: { message: 'Not Found', type: 'not_found' } });
      const m = /^openclaw[/:]([a-z0-9_-]+)$/i.exec(String(body.model || ''));
      if (!m) return json(400, { error: { message: 'Invalid `model`.', type: 'invalid_request_error' } });
      const agent = m[1] === 'default' ? 'main' : m[1].toLowerCase();
      if (agent !== 'main' && agent !== 'research') return json(400, { error: { message: `Unknown agent '${agent}'.`, type: 'invalid_request_error' } });
      const last = [...body.messages].reverse().find((x) => x.role === 'user');
      // The hub puts its own lines before the user's text; the script word is the last line.
      const said = String((last && last.content) || '').trim().split('\n').at(-1);
      if (said === 'archived') return json(409, { error: { message: 'This session is archived', type: 'invalid_request_error' } });
      const key = body.user ? `agent:${agent}:openai-user:${body.user}` : `agent:${agent}:openai:random`;
      const turn = (state.sessions.get(key) || 0) + 1;
      state.sessions.set(key, turn);
      res.writeHead(200, { 'Content-Type': 'text/event-stream; charset=utf-8', 'Cache-Control': 'no-cache', Connection: 'keep-alive' });
      res.flushHeaders();
      const id = `chatcmpl_${turn}`;
      const chunk = (delta, finish = null) => res.write(`data: ${JSON.stringify({ id, object: 'chat.completion.chunk', created: 0, model: body.model, choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`);
      chunk({ role: 'assistant' });
      if (said === 'silent') return;
      if (said === 'slow') {
        chunk({ content: 'Working' });
        res.on('close', () => state.cancelled.push(key));
        return;
      }
      if (said === 'fail') {
        chunk({ content: 'Partial' });
        res.write(`data: ${JSON.stringify({ error: { message: 'Agent run failed: provider overloaded', type: 'api_error' } })}\n\n`);
        res.write('data: [DONE]\n\n');
        return res.end();
      }
      if (said === 'cut') {
        chunk({ content: 'Half an ans' });
        return setTimeout(() => res.socket.destroy(), 20);
      }
      const words = ['\n\nTurn ', `${turn} `, 'of ', key];
      let i = 0;
      const next = () => {
        if (i < words.length) {
          chunk({ content: words[i++] });
          return setTimeout(next, 5);
        }
        chunk({}, 'stop');
        res.write('data: [DONE]\n\n');
        res.end();
      };
      next();
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return { server, state, url: `http://127.0.0.1:${server.address().port}`, chats: () => state.requests.filter((r) => r.path === '/v1/chat/completions') };
}

function fakeTurn(conversation, input, { model = null } = {}) {
  const controller = new AbortController();
  const turn = {
    id: `t_${Math.random().toString(36).slice(2, 10)}`,
    conversation,
    input,
    model,
    effort: null,
    skills: [],
    instructions: 'You are working inside Rukoo Mail.',
    events: [],
    accepted: () => {
      turn.reached = true;
    },
    emit: (event) => turn.events.push(event),
    setProvider: (patch) => Object.assign(conversation.provider, patch),
    signal: controller.signal,
    controller
  };
  return turn;
}
const conv = (id = 'c_test_1') => ({ id, agent: 'openclaw', title: 'Call on Thursday', provider: {}, items: [] });
const texts = (turn) => turn.events.filter((e) => e.type === 'text').map((e) => e.delta).join('');

test('openclaw: status tells unconfigured, unauthorized, a missing agent, an endpoint that is off, offline and ready', async () => {
  const gw = await gateway();
  const off = await gateway({ enabled: false });
  try {
    const make = (cfg) => new OpenClawAdapter({ config: () => cfg });
    assert.deepEqual(await make({ enabled: false, url: gw.url, key: TOKEN }).status(), { state: 'disabled', detail: '' });
    assert.deepEqual(await make({ url: '', key: TOKEN }).status(), { state: 'unconfigured', detail: 'No gateway URL' });
    assert.deepEqual(await make({ url: gw.url, key: '' }).status(), { state: 'unconfigured', detail: 'No gateway token' });
    assert.deepEqual(await make({ url: gw.url, key: 'wrong' }).status(), { state: 'unauthorized', detail: 'The gateway refused the token' });
    assert.deepEqual(await make({ url: gw.url, key: TOKEN }).status(), { state: 'ready', detail: 'OpenClaw gateway, agent default' });
    assert.deepEqual(await make({ url: gw.url, key: TOKEN, agent: 'research' }).status(), { state: 'ready', detail: 'OpenClaw gateway, agent research' });
    assert.deepEqual(await make({ url: gw.url, key: TOKEN, agent: 'sales' }).status(), { state: 'unknown', detail: 'The gateway has no agent "sales"' });
    const disabled = await make({ url: off.url, key: TOKEN }).status();
    assert.equal(disabled.state, 'unknown');
    assert.match(disabled.detail, /gateway\.http\.endpoints\.chatCompletions\.enabled/);
    assert.equal((await make({ url: await closedPort(), key: TOKEN }).status()).state, 'offline');
    // The check is GET /v1/models with the token: it needs both.
    const check = gw.state.requests.find((r) => r.path === '/v1/models' && r.headers.authorization === `Bearer ${TOKEN}`);
    assert.ok(check);

    // Cached for 30 s, unless forced (the Test button); a new token is a new check.
    const adapter = make({ url: gw.url, key: TOKEN });
    await adapter.status();
    const before = gw.state.requests.length;
    await adapter.status();
    assert.equal(gw.state.requests.length, before);
    await adapter.status({ force: true });
    assert.equal(gw.state.requests.length, before + 1);
  } finally {
    gw.server.close();
    off.server.close();
  }
});

test('openclaw: a turn streams the reply and keeps one gateway session per chat across turns', async () => {
  const gw = await gateway();
  try {
    let settings = { url: gw.url + '/', key: TOKEN, agent: '', model: '' };
    const adapter = new OpenClawAdapter({ config: () => settings });
    const c = conv();
    const first = fakeTurn(c, 'Summarize this email');
    assert.deepEqual(await adapter.runTurn(first), { status: 'done' });
    assert.equal(first.reached, true, 'the gateway took the message');
    // Several deltas, the leading blank lines of the answer dropped.
    assert.ok(first.events.filter((e) => e.type === 'text').length >= 3);
    assert.equal(texts(first), 'Turn 1 of agent:main:openai-user:rukoo:c_test_1');
    assert.deepEqual(c.provider, { agent: 'default', sessionUser: 'rukoo:c_test_1' });
    const [req] = gw.chats();
    assert.deepEqual(req.body, {
      model: 'openclaw/default',
      user: 'rukoo:c_test_1',
      stream: true,
      messages: [
        { role: 'system', content: first.instructions },
        { role: 'user', content: 'Summarize this email' }
      ]
    });
    assert.equal(req.headers.authorization, `Bearer ${TOKEN}`);
    assert.equal(req.headers['x-openclaw-model'], undefined, 'no model: the agent uses its own');

    // The next turn goes to the same session with only the new message: the gateway keeps the transcript.
    const second = fakeTurn(c, 'And now in Dutch');
    assert.deepEqual(await adapter.runTurn(second), { status: 'done' });
    assert.equal(texts(second), 'Turn 2 of agent:main:openai-user:rukoo:c_test_1');
    assert.deepEqual(gw.chats()[1].body.messages.map((m) => m.role), ['system', 'user']);

    // Settings naming another agent later leave this chat with the agent its session belongs to; a new chat takes
    // the new agent. The chat's model, else the model in Settings, goes as x-openclaw-model.
    settings = { ...settings, agent: 'research', model: 'anthropic/claude-opus-5-5' };
    const third = fakeTurn(c, 'Thanks', { model: 'openai/gpt-6-astra' });
    assert.deepEqual(await adapter.runTurn(third), { status: 'done' });
    assert.equal(texts(third), 'Turn 3 of agent:main:openai-user:rukoo:c_test_1');
    assert.equal(gw.chats()[2].body.model, 'openclaw/default');
    assert.equal(gw.chats()[2].headers['x-openclaw-model'], 'openai/gpt-6-astra');
    const other = conv('c_test_2');
    const fresh = fakeTurn(other, 'Hello');
    assert.deepEqual(await adapter.runTurn(fresh), { status: 'done' });
    assert.equal(texts(fresh), 'Turn 1 of agent:research:openai-user:rukoo:c_test_2');
    assert.equal(gw.chats()[3].headers['x-openclaw-model'], 'anthropic/claude-opus-5-5');
    assert.deepEqual(other.provider, { agent: 'research', sessionUser: 'rukoo:c_test_2' });

    // The model menu has Default and the model in Settings only; there are no effort levels or own skills to list.
    assert.deepEqual(await adapter.models(), { models: [], efforts: [], custom: [] });
    assert.deepEqual(await adapter.skills(), []);
  } finally {
    gw.server.close();
  }
});

test('openclaw: errors from the gateway reach the chat with their reason', async () => {
  const gw = await gateway();
  const off = await gateway({ enabled: false });
  try {
    const run = async (settings, input, c = conv()) => {
      const turn = fakeTurn(c, input);
      const result = await new OpenClawAdapter({ config: () => settings }).runTurn(turn);
      return { result, turn };
    };
    const ok = { url: gw.url, key: TOKEN };

    const unauthorized = await run({ ...ok, key: 'wrong' }, 'hello');
    assert.equal(unauthorized.result.error.code, 'unauthorized');
    assert.match(unauthorized.result.error.detail, /^HTTP 401: Unauthorized/);
    assert.equal(unauthorized.turn.reached, undefined, 'nothing reached the agent');

    const disabled = await run({ url: off.url, key: TOKEN }, 'hello');
    assert.equal(disabled.result.error.code, 'not-configured');
    assert.match(disabled.result.error.detail, /chatCompletions\.enabled/);

    const missing = await run({ ...ok }, 'hello', { ...conv('c_x'), provider: { agent: 'sales', sessionUser: 'rukoo:c_x' } });
    assert.deepEqual(missing.result, { status: 'error', error: { code: 'protocol', detail: "HTTP 400: Unknown agent 'sales'." } });

    const archived = await run(ok, 'archived');
    assert.equal(archived.result.error.code, 'unknown');
    assert.match(archived.result.error.detail, /^This session is archived\. Restore the session in OpenClaw’s Control UI/);

    // A run that fails part-way: the text so far stays, and the error follows.
    const failed = await run(ok, 'fail');
    assert.equal(texts(failed.turn), 'Partial');
    assert.deepEqual(failed.result, { status: 'error', error: { code: 'unknown', detail: 'Agent run failed: provider overloaded' } });
    assert.deepEqual(failed.turn.events.at(-1), { type: 'error', code: 'unknown', detail: 'Agent run failed: provider overloaded' });

    // A connection that drops before [DONE] is not a finished answer.
    const cut = await run(ok, 'cut');
    assert.equal(texts(cut.turn), 'Half an ans');
    assert.equal(cut.result.error.code, 'offline');

    const offline = await run({ url: await closedPort(), key: TOKEN }, 'hello');
    assert.equal(offline.result.error.code, 'offline');
    assert.deepEqual((await run({ url: '', key: TOKEN }, 'hello')).result, { status: 'error', error: { code: 'not-configured', detail: 'No gateway URL' } });
    assert.deepEqual((await run({ ...ok, enabled: false }, 'hello')).result, { status: 'error', error: { code: 'disabled', detail: 'OpenClaw is turned off' } });

    // The gateway sends no keepalives; a stream silent for longer than the limit is closed, which stops the run.
    const quiet = fakeTurn(conv('c_quiet'), 'silent');
    const result = await new OpenClawAdapter({ config: () => ok, timing: { silence: 150 } }).runTurn(quiet);
    assert.equal(result.error.code, 'timeout');
  } finally {
    gw.server.close();
    off.server.close();
  }
});

test('openclaw: stop closes the connection, which cancels the run on the gateway; so does quitting', async () => {
  const gw = await gateway();
  try {
    const adapter = new OpenClawAdapter({ config: () => ({ url: gw.url, key: TOKEN }) });
    const c = conv();
    const turn = fakeTurn(c, 'slow');
    const running = adapter.runTurn(turn);
    await waitFor(() => turn.events.some((e) => e.type === 'text'));
    assert.equal(texts(turn), 'Working');
    turn.controller.abort();
    assert.deepEqual(await running, { status: 'stopped' });
    await waitFor(() => gw.state.cancelled.length === 1);
    assert.deepEqual(gw.state.cancelled, ['agent:main:openai-user:rukoo:c_test_1']);
    assert.equal(adapter.running.size, 0);

    // A stopped turn that never started sends nothing.
    const early = fakeTurn(c, 'hello');
    early.controller.abort();
    const before = gw.chats().length;
    assert.deepEqual(await adapter.runTurn(early), { status: 'stopped' });
    assert.equal(gw.chats().length, before);

    const other = fakeTurn(conv('c_test_2'), 'slow');
    const pending = adapter.runTurn(other);
    await waitFor(() => other.events.some((e) => e.type === 'text'));
    await adapter.dispose();
    await waitFor(() => gw.state.cancelled.length === 2);
    assert.equal((await pending).status !== 'done', true);
  } finally {
    gw.server.close();
  }
});

test('openclaw: config keeps the token encrypted, checks the agent id, and the setup command reaches Rukoo’s MCP server', async () => {
  const dir = tempDir('rukoo-openclaw-');
  try {
    const file = path.join(dir, 'agents.json');
    const cfg = new AgentConfig({ file, secrets }).load();
    assert.deepEqual(cfg.publicView().openclaw, { enabled: true, url: 'http://127.0.0.1:18789', agent: '', model: '', remote: false, hasKey: false });
    assert.equal(cfg.openclawSetup({ port: 47800 }), '', 'no token, no setup');
    cfg.update({ defaultAgent: 'openclaw', openclaw: { url: 'http://127.0.0.1:18789/', agent: 'research', model: 'openai/gpt-6-astra' } });
    assert.throws(() => cfg.update({ openclaw: { agent: 'not an id' } }), (e) => /agent id/.test(e.detail));
    assert.throws(() => cfg.update({ openclaw: { url: 'ftp://gateway' } }), (e) => /http/.test(e.detail));
    cfg.setSecret('openclaw', `  ${TOKEN}  `);
    const stored = JSON.parse(fs.readFileSync(file, 'utf8'));
    assert.equal(stored.defaultAgent, 'openclaw');
    assert.equal(stored.openclaw.url, 'http://127.0.0.1:18789');
    assert.notEqual(stored.openclaw.key, TOKEN, 'encrypted on disk');
    assert.ok(!JSON.stringify(cfg.publicView()).includes(TOKEN), 'never to the renderer');
    assert.equal(cfg.get('openclaw').key, TOKEN);
    assert.equal(cfg.displayName('openclaw'), 'OpenClaw');
    // The token OpenClaw sends Rukoo is made from the gateway token, not the gateway token itself.
    const token = openclawTokenFor(TOKEN);
    assert.notEqual(token, TOKEN);
    assert.equal(cfg.openclawToken(), token);
    const setup = cfg.openclawSetup({ port: 47811 });
    assert.equal(setup, openclawSetupText('http://127.0.0.1:47811/mcp', token));
    // The remote token is another one, and there is none until the user allows a gateway on another computer.
    const remote = openclawTokenFor(TOKEN, true);
    assert.notEqual(remote, token);
    assert.equal(cfg.openclawToken(true), '');
    assert.equal(cfg.openclawSetup({ remote: true, host: '100.64.0.5', port: 47800 }), '');
    assert.throws(() => cfg.update({ openclaw: { remote: 'yes' } }), (e) => /true or false/.test(e.detail));
    cfg.update({ openclaw: { remote: true } });
    assert.equal(cfg.publicView().openclaw.remote, true);
    assert.equal(cfg.openclawToken(true), remote);
    assert.equal(cfg.openclawSetup({ remote: true, host: '100.64.0.5', port: 47800 }), openclawSetupText('http://100.64.0.5:47800/mcp', remote));
    assert.equal(cfg.openclawSetup({ remote: true, host: '', port: 47800 }), '', 'no address, no setup');
    // A file edited by hand with an agent id the gateway would refuse falls back to the default agent.
    fs.writeFileSync(file, JSON.stringify({ ...stored, openclaw: { ...stored.openclaw, agent: '../x' } }));
    assert.equal(new AgentConfig({ file, secrets }).load().data.openclaw.agent, '');
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('openclaw: the copied setup keeps the token out of the command, matches the phone’s, and is safe to paste into a shell', () => {
  const dir = tempDir('rukoo-openclaw-setup-');
  try {
    const cfg = new AgentConfig({ file: path.join(dir, 'agents.json'), secrets }).load();
    cfg.setSecret('openclaw', 'gw-token-123');
    cfg.update({ openclaw: { remote: true } });
    const setup = cfg.openclawSetup({ remote: true, host: '100.64.0.3', port: 47800 });
    // Word for word what the phone copies (android SettingsRulesTest.openClawSetupMatchesDesktop pins the same text).
    assert.equal(
      setup,
      '# Step 1, not a command: put this line in the OpenClaw .env file, ~/.openclaw/.env or $OPENCLAW_STATE_DIR/.env, in place of any older RUKOO_MCP_TOKEN line.\n' +
        'RUKOO_MCP_TOKEN=zH_liMsCMypd2NxR_kgVAU2gmnl7jdrAiIwiXN0sqds\n' +
        '# Step 2: run this command where the gateway runs, then restart the gateway.\n' +
        `openclaw mcp set rukoo '{"url":"http://100.64.0.3:47800/mcp","transport":"streamable-http","headers":{"Authorization":"Bearer \${RUKOO_MCP_TOKEN}"}}'\n`
    );
    const token = cfg.openclawToken(true);
    const lines = setup.split('\n');
    assert.deepEqual(
      lines.filter((l) => l.includes(token)),
      [`RUKOO_MCP_TOKEN=${token}`],
      'the token is in the .env line only'
    );
    assert.equal(JSON.parse(lines[3].match(/'(.*)'$/)[1]).headers.Authorization, 'Bearer ${RUKOO_MCP_TOKEN}');
    // The step labels hold nothing a shell acts on, also in zsh, which reads # as a command without interactivecomments.
    for (const label of [lines[0], lines[2]]) assert.doesNotMatch(label, /['"`()[\]{};&|<>*?!\\]/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// Pasting all four lines into a POSIX shell runs the command with the variable left for OpenClaw to fill in.
test('openclaw: the copied setup pasted into sh passes ${RUKOO_MCP_TOKEN} to openclaw as it is', { skip: process.platform === 'win32' && 'no POSIX shell on Windows' }, () => {
  const { spawnSync } = require('child_process');
  const dir = tempDir('rukoo-openclaw-sh-');
  try {
    const cfg = new AgentConfig({ file: path.join(dir, 'agents.json'), secrets }).load();
    cfg.setSecret('openclaw', TOKEN);
    const setup = cfg.openclawSetup({ port: 47811 });
    // openclaw stands in as a function that prints its arguments; env shows whether the token line exported anything.
    const script = `openclaw() { printf '%s\\n' "$@"; }\n${setup}env | grep -c '^RUKOO_MCP_TOKEN=' || true\n`;
    const r = spawnSync('sh', ['-c', script], { encoding: 'utf8', env: { PATH: process.env.PATH }, timeout: 10000 });
    assert.equal(r.status, 0, r.stderr);
    const out = r.stdout.trim().split('\n');
    assert.deepEqual(out.slice(0, 3), ['mcp', 'set', 'rukoo']);
    assert.deepEqual(JSON.parse(out[3]), { url: 'http://127.0.0.1:47811/mcp', transport: 'streamable-http', headers: { Authorization: 'Bearer ${RUKOO_MCP_TOKEN}' } });
    assert.equal(out[4], '0', 'the token line sets a shell variable only, which the gateway never sees');
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('openclaw: through the hub, a chat streams from the gateway and OpenClaw’s MCP token works on 127.0.0.1 only', async () => {
  const gw = await gateway();
  const dir = tempDir('rukoo-openclaw-hub-');
  const engine = new Engine({ dataDir: dir }).init();
  const acc = await engine.addAccount({ type: 'demo' });
  await engine.syncAccount(acc.id);
  const probe = http.createServer();
  await new Promise((r) => probe.listen(0, '127.0.0.1', r));
  const free = probe.address().port;
  await new Promise((r) => probe.close(r));
  fs.writeFileSync(path.join(dir, 'agents.json'), JSON.stringify({ mcp: { port: free < 1024 ? 47898 : free }, openclaw: { url: gw.url } }));
  const clipboard = { text: '', writeText: (s) => (clipboard.text = s) };
  const hub = new AgentHub({ engine, dataDir: dir, secrets, deps: { appVersion: '1.0.0', clipboard }, adapters: { openclaw: (opts) => new OpenClawAdapter(opts) } });
  await hub.start();
  try {
    assert.throws(() => hub.copyOpenClawSetup(), /not-configured/);
    hub.setSecret('openclaw', TOKEN);
    // The gateway takes no message during a turn, so the panel queues them (steer: false).
    assert.deepEqual(await hub.test('openclaw'), { state: 'ready', detail: 'OpenClaw gateway, agent default', steer: false });

    const call = engine.listMessages({ view: 'inbox' }).find((m) => m.subject === 'Call on Thursday');
    hub.view({ openMessageId: call.id, checkedIds: [], scope: 'all', view: 'inbox', folder: null, composer: null });
    const c = hub.create({ agent: 'openclaw', message: { id: call.id } });
    hub.send(c.id, { text: 'Summarize this email' });
    await waitFor(() => !hub.turns.has(c.id));
    assert.equal(c.status, 'idle');
    assert.equal(c.items.filter((i) => i.type === 'assistant').at(-1).text, `Turn 1 of agent:main:openai-user:rukoo:${c.id}`);
    // The first message carries the email and the chat's id for Rukoo's tools; Rukoo's instructions go as the system
    // message. OpenClaw may run on another machine, so a file goes by read_chat_file, not a local path.
    const first = gw.chats()[0].body.messages;
    assert.equal(first[0].role, 'system');
    assert.match(first[0].content, /Rukoo gives you an MCP server called "rukoo"/);
    assert.match(first[1].content, new RegExp(`Pass conversation_id "${c.id}"`));
    assert.match(first[1].content, /Call on Thursday/);
    assert.equal(hub.localAgent(c), false);
    hub.send(c.id, { text: 'Shorter please' });
    await waitFor(() => !hub.turns.has(c.id));
    assert.equal(c.items.filter((i) => i.type === 'assistant').at(-1).text, `Turn 2 of agent:main:openai-user:rukoo:${c.id}`);
    assert.doesNotMatch(gw.chats()[1].body.messages[1].content, /Call on Thursday/, 'the email went with the first turn only');

    // Stop from the panel cancels the gateway run.
    hub.send(c.id, { text: 'slow' });
    await waitFor(() => gw.chats().length === 3 && c.items.some((i) => i.type === 'assistant' && i.text === 'Working'));
    hub.stop(c.id);
    await waitFor(() => !hub.turns.has(c.id));
    await waitFor(() => gw.state.cancelled.length === 1);

    // The setup OpenClaw gets, and the token in its .env line, which Rukoo's loopback listener accepts.
    const port = hub.mcp.ports().local;
    assert.deepEqual(hub.copyOpenClawSetup(), { remote: false, url: `http://127.0.0.1:${port}/mcp` });
    const token = openclawTokenFor(TOKEN);
    assert.equal(clipboard.text, openclawSetupText(`http://127.0.0.1:${port}/mcp`, token));
    assert.deepEqual(hub.identify(token), { agent: 'openclaw', conversationId: null, remote: false });
    assert.equal(hub.identify(TOKEN), null, 'the gateway token itself is no MCP token');
    const rpc = (body) =>
      fetch(`http://127.0.0.1:${port}/mcp`, { method: 'POST', headers: { 'Content-Type': 'application/json', Accept: 'application/json, text/event-stream', Authorization: `Bearer ${token}` }, body: JSON.stringify(body) }).then((r) => r.json());
    const list = await rpc({ jsonrpc: '2.0', id: 1, method: 'tools/list' });
    assert.ok(list.result.tools.some((t) => t.name === 'write_draft'));
    // Without a chat token, the conversation_id the turn named routes the call to this chat.
    const ctx = await rpc({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'get_context', arguments: { conversation_id: c.id } } });
    assert.equal(JSON.parse(ctx.result.content[0].text).conversation_id, c.id);

    // A new gateway token replaces the MCP token; turning OpenClaw off closes Rukoo's tools to it.
    hub.setSecret('openclaw', 'another-token');
    assert.equal(hub.identify(token), null);
    const next = openclawTokenFor('another-token');
    await hub.updateConfig({ openclaw: { enabled: false } });
    const refused = await fetch(`http://127.0.0.1:${port}/mcp`, { method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${next}` }, body: JSON.stringify({ jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'get_context', arguments: {} } }) }).then((r) => r.json());
    assert.equal(refused.result.isError, true);
    assert.match(refused.result.content[0].text, /OpenClaw is turned off in Rukoo/);
  } finally {
    await hub.dispose();
    await engine.close();
    gw.server.close();
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
});

// A plain request to an MCP listener, with any Host header (fetch() won't send another host's name).
function mcpRequest(port, token, { host = `127.0.0.1:${port}`, body = { jsonrpc: '2.0', id: 1, method: 'tools/list' } } = {}) {
  return new Promise((resolve, reject) => {
    const data = JSON.stringify(body);
    const headers = { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(data), Host: host };
    if (token) headers.Authorization = `Bearer ${token}`;
    const req = http.request({ host: '127.0.0.1', port, method: 'POST', path: '/mcp', headers, agent: false }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        let json = null;
        try {
          json = JSON.parse(Buffer.concat(chunks).toString('utf8'));
        } catch {}
        resolve({ status: res.statusCode, json });
      });
    });
    req.on('error', reject);
    req.end(data);
  });
}

test('openclaw: a gateway on another computer gets Rukoo’s tools on the Tailscale listener, only while the user allows it', async () => {
  const dir = tempDir('rukoo-openclaw-remote-');
  const engine = new Engine({ dataDir: dir }).init();
  const probe = http.createServer();
  await new Promise((r) => probe.listen(0, '127.0.0.1', r));
  const free = probe.address().port;
  await new Promise((r) => probe.close(r));
  // remoteHost stands in for this computer's Tailscale address; the listener itself goes on loopback below.
  fs.writeFileSync(path.join(dir, 'agents.json'), JSON.stringify({ mcp: { port: free < 1024 ? 47897 : free, remoteHost: '100.64.0.9' } }));
  const clipboard = { text: '', writeText: (s) => (clipboard.text = s) };
  const hub = new AgentHub({ engine, dataDir: dir, secrets, deps: { appVersion: '1.0.0', clipboard }, adapters: {} });
  await hub.start();
  const listen = hub.mcp.listen.bind(hub.mcp);
  hub.mcp.listen = (opts) => listen(opts.remote ? { ...opts, host: '127.0.0.1', port: 0 } : opts);
  const remotePort = () => hub.mcp.ports().remote;
  const local = hub.mcp.ports().local;
  try {
    hub.setSecret('openclaw', TOKEN);
    hub.setSecret('clark', 'hermes-key');
    const localToken = openclawTokenFor(TOKEN);
    const remoteToken = openclawTokenFor(TOKEN, true);
    const hermes = hub.cfg.remoteToken();
    const chat = hub.issueToken({ agent: 'claude', conversationId: null });

    // Off by default: no Tailscale listener, no remote OpenClaw token, and the copied setup is for this computer.
    assert.equal(remotePort(), null);
    assert.equal(hub.identify(remoteToken), null);
    assert.deepEqual(hub.copyOpenClawSetup(), { remote: false, url: `http://127.0.0.1:${local}/mcp` });
    assert.equal(clipboard.text, openclawSetupText(`http://127.0.0.1:${local}/mcp`, localToken));

    // Allowing it opens the Tailscale listener, which takes the remote token and not the local one.
    await hub.updateConfig({ openclaw: { remote: true } });
    assert.ok(remotePort(), 'the Tailscale listener is open');
    assert.deepEqual(hub.identify(remoteToken), { agent: 'openclaw', conversationId: null, remote: true });
    const listed = await mcpRequest(remotePort(), remoteToken);
    assert.equal(listed.status, 200);
    assert.ok(listed.json.result.tools.some((t) => t.name === 'write_draft'));
    assert.equal((await mcpRequest(remotePort(), localToken)).status, 401, 'the local token is no use from the tailnet');
    assert.equal((await mcpRequest(local, remoteToken)).status, 401, 'the remote token is no use on loopback');
    assert.equal((await mcpRequest(local, localToken)).status, 200, 'a gateway on this computer keeps working');
    // Clark's token needs "Let Clark use Rukoo", also now that the listener is open; a chat's works on loopback only.
    assert.equal((await mcpRequest(remotePort(), hermes)).status, 401);
    assert.equal((await mcpRequest(local, hermes)).status, 401);
    assert.equal((await mcpRequest(remotePort(), chat)).status, 401);
    assert.equal((await mcpRequest(local, chat)).status, 200);
    assert.equal((await mcpRequest(remotePort(), null)).status, 401);
    // The DNS-rebinding guard: a Host that isn't this listener's name is refused, token or not.
    assert.equal((await mcpRequest(remotePort(), remoteToken, { host: `evil.example:${remotePort()}` })).status, 403);
    assert.equal((await mcpRequest(remotePort(), remoteToken, { host: `127.0.0.1:${remotePort() + 1}` })).status, 403);

    // The copied setup now names Rukoo's Tailscale address and the listener's real port, with the remote token.
    assert.deepEqual(hub.copyOpenClawSetup(), { remote: true, url: `http://100.64.0.9:${remotePort()}/mcp` });
    assert.equal(clipboard.text, openclawSetupText(`http://100.64.0.9:${remotePort()}/mcp`, remoteToken));
    assert.ok(!clipboard.text.includes(localToken) && !clipboard.text.includes(TOKEN));

    // A new gateway token gives new MCP tokens; the old ones stop working at once.
    hub.setSecret('openclaw', 'another-token');
    assert.equal((await mcpRequest(remotePort(), remoteToken)).status, 401);
    assert.equal((await mcpRequest(remotePort(), openclawTokenFor('another-token', true))).status, 200);
    hub.setSecret('openclaw', TOKEN);

    // Turned off again: the listener closes, as nothing else wants it.
    await hub.updateConfig({ openclaw: { remote: false } });
    assert.equal(remotePort(), null);
    assert.equal(hub.identify(remoteToken), null);
    // With Clark's listener open, the remote OpenClaw token still gets nowhere while the user doesn't allow it.
    await hub.updateConfig({ mcp: { remote: true } });
    assert.ok(remotePort());
    assert.equal((await mcpRequest(remotePort(), remoteToken)).status, 401);
    assert.equal((await mcpRequest(remotePort(), hermes)).status, 200);
    await hub.updateConfig({ mcp: { remote: false } });
    assert.equal(remotePort(), null);
    // OpenClaw turned off: no listener for it, even with the remote toggle on.
    await hub.updateConfig({ openclaw: { remote: true, enabled: false } });
    assert.equal(remotePort(), null);
    await hub.updateConfig({ openclaw: { enabled: true } });
    assert.ok(remotePort());
  } finally {
    await hub.dispose();
    await engine.close();
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
});
