'use strict';

// The Voice bridge client and the service around it, against the hand-written fake Voice in
// test/fixtures/voice-server.js: hello and welcome, replay after the cursor, a gap and meeting.get, a new Voice
// data folder, command timeouts, a connection cut halfway through a message, stale and changed discovery files, and
// hiding the windows before a meeting starts.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const http = require('http');
const path = require('path');
const { spawn } = require('child_process');
const { tempDir } = require('./tmp');
const { startVoiceServer } = require('./fixtures/voice-server');
const { VoiceBridge } = require('../src/main/voice/bridge');
const { VoiceService } = require('../src/main/voice');

const M1 = 'm-20261030-090000-0a1b';
const M2 = 'm-20261030-100000-0c2d';
const FAST = { backoffMs: [30], pollMs: 50, heartbeatMs: 0 };

async function until(check, what, ms = 5000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await check();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 15));
  }
}

const started = (title, at = Date.now()) => ({ title, startedAt: at, tracks: ['mic', 'call'], screen: 'off', startedBy: 'voice', context: null });
const passage = (track, start, text, revision = 1, extra = {}) => ({ id: `${track}-${String(start).padStart(9, '0')}`, track, speaker: track === 'mic' ? 'mic' : 'call-1', start, end: start + 2000, text, revision, dictation: false, language: 'en', ...extra });

async function setup(t, opts = {}) {
  const dir = tempDir('rukoo-voice-bridge-');
  const file = path.join(dir, 'voice-bridge.json');
  const voice = await startVoiceServer({ file, ...opts });
  t.after(() => voice.close());
  return { dir, file, voice };
}

function service(t, dir, file, extra = {}) {
  const protects = [];
  const s = new VoiceService({
    dataDir: path.join(dir, 'data'),
    discovery: file,
    appVersion: '9.9.9',
    hide: () => true,
    protect: (on) => protects.push({ on, at: Date.now() }),
    bridge: FAST,
    emitMs: 10,
    ...extra
  });
  s.protects = protects;
  t.after(() => s.close());
  return s;
}

// A process id that was in use a moment ago and is free now.
function deadPid() {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, ['-e', ''], { stdio: 'ignore' });
    child.on('exit', () => resolve(child.pid));
  });
}

test('the first hello carries the token and Rukoo\'s pid, no cursor and no Origin, and the welcome connects', async (t) => {
  const { file, voice } = await setup(t);
  voice.writeDiscovery();
  const bridge = new VoiceBridge({ file, clientVersion: '2.2.0', ...FAST }).start();
  t.after(() => bridge.close());
  const welcome = await new Promise((resolve) => bridge.once('welcome', resolve));
  assert.equal(welcome.instance, voice.instance);
  assert.equal(welcome.replay, 'full');
  assert.deepEqual(voice.hellos[0], { type: 'hello', protocol: 2, token: voice.token, client: 'rukoo', clientVersion: '2.2.0', pid: process.pid, instance: null, after: 0 });
  assert.equal(voice.upgrades[0].url, '/bridge');
  assert.equal(voice.upgrades[0].headers.origin, undefined, "Rukoo's main process sends no Origin");
  assert.equal(bridge.state().status, 'connected');
  assert.equal(bridge.state().voiceVersion, '0.1.0');
  const status = await bridge.command('status.get');
  assert.deepEqual(status, { recording: null, dictating: false });
});

test('the fake Voice refuses a page that names an Origin, as Voice does', async (t) => {
  const { voice } = await setup(t);
  const code = await new Promise((resolve) => {
    const req = http.request({ host: '127.0.0.1', port: voice.port, path: '/bridge', headers: { Connection: 'Upgrade', Upgrade: 'websocket', Origin: 'https://evil.example', 'Sec-WebSocket-Key': 'dGhlIHNhbXBsZSBub25jZQ==', 'Sec-WebSocket-Version': '13' } });
    req.on('response', (res) => resolve(res.statusCode));
    req.on('upgrade', () => resolve(101));
    req.on('error', () => resolve('error'));
    req.end();
  });
  assert.equal(code, 403);
});

test('after a restart Rukoo asks for what came after the last event it applied, and gets only that', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Weekly'));
  voice.append('passage', M1, passage('call', 1000, 'First.'));
  const first = service(t, dir, file).init();
  await until(() => first.store.cursor().after === 2, 'the first replay');
  first.close();
  assert.equal(first.bridge.connected, false);
  // Voice records on while Rukoo is closed.
  voice.append('passage', M1, passage('call', 4000, 'Second.'));
  voice.append('passage', M1, passage('call', 1000, 'First, again.', 2));
  const second = service(t, dir, file).init();
  await until(() => second.store.cursor().after === 4, 'the second replay');
  const hello = voice.hellos.at(-1);
  assert.equal(hello.instance, voice.instance);
  assert.equal(hello.after, 2);
  const m = second.get(M1);
  assert.deepEqual(m.passages.map((p) => [p.text, p.revision]), [['First, again.', 2], ['Second.', 1]]);
  assert.equal(m.meeting.title, 'Weekly');
});

test('a gap: Rukoo asks meeting.get for the meetings it has that may still change, and drops one Voice no longer has', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Kept'));
  voice.append('meeting.started', M2, started('Deleted'));
  const s1 = service(t, dir, file).init();
  await until(() => s1.store.cursor().after === 2, 'the first replay');
  s1.close();
  for (let i = 0; i < 6; i++) voice.append('status', null, { recording: null, dictating: false });
  // Events Rukoo never saw are gone already.
  voice.forget(6);
  const snap = {
    meeting: { id: M1, ...started('Kept'), state: 'done', endedAt: Date.now() },
    speakers: [{ id: 'call-1', track: 'call', label: 'Speaker 1' }],
    passages: [passage('call', 1000, 'From the snapshot.', 3)],
    marks: [],
    head: voice.head
  };
  voice.handlers['meeting.get'] = (args) => (args.meeting === M1 ? snap : { error: { code: 'not_found', message: 'No such meeting.' } });
  const s2 = service(t, dir, file).init();
  await until(() => voice.commands.filter((c) => c.command === 'meeting.get').length === 2, 'two meeting.get');
  await until(() => !s2.store.entry(M2), 'the deleted meeting to go');
  // Rukoo asks for one meeting at a time, the newest first, so the answer for M1 may still be on its way.
  await until(() => (s2.get(M1) || {}).meeting?.state === 'done', 'the snapshot of the kept meeting');
  assert.deepEqual(new Set(voice.commands.map((c) => c.args.meeting)), new Set([M1, M2]));
  const m = s2.get(M1);
  assert.equal(m.meeting.state, 'done');
  assert.deepEqual(m.passages.map((p) => p.text), ['From the snapshot.']);
  assert.equal(m.speakers[0].label, 'Speaker 1');
  assert.equal(s2.store.cursor().after, voice.head);
});

test('a gap: a done meeting Voice deleted while Rukoo was away goes too, with its notes kept', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Finished'));
  voice.append('passage', M1, passage('call', 1000, 'Said in a finished meeting.'));
  voice.append('meeting.state', M1, { state: 'done', at: 5000, endedAt: Date.now() });
  const s1 = service(t, dir, file).init();
  await until(() => s1.store.cursor().after === 3, 'the first replay');
  s1.saveNotes(M1, 'My own notes.', 0);
  s1.close();
  // Voice deleted the meeting, and its meeting.removed is among the events that are gone.
  voice.append('meeting.removed', M1, {});
  for (let i = 0; i < 4; i++) voice.append('status', null, { recording: null, dictating: false });
  voice.forget(7);
  voice.handlers['meeting.get'] = () => ({ error: { code: 'not_found', message: 'No such meeting.' } });
  const s2 = service(t, dir, file).init();
  await until(() => voice.commands.some((c) => c.command === 'meeting.get' && c.args.meeting === M1), 'meeting.get for the done meeting');
  await until(() => (s2.get(M1) || { passages: [1] }).passages.length === 0, 'its transcript to go');
  assert.equal(s2.get(M1).notes.text, 'My own notes.');
});

test('a meeting file found damaged when the meeting opens is fetched from Voice again', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Read later'));
  voice.append('passage', M1, passage('call', 1000, 'Kept by Voice.'));
  voice.append('meeting.state', M1, { state: 'done', at: 5000, endedAt: Date.now() });
  const s1 = service(t, dir, file).init();
  await until(() => s1.store.cursor().after === 3, 'the first replay');
  s1.close();
  // voice.json is fine and a partial replay brings nothing, but the meeting's own file is damaged.
  fs.writeFileSync(path.join(dir, 'data', 'meetings', `${M1}.json`), '{"passages":');
  voice.handlers['meeting.get'] = () => ({
    meeting: { id: M1, ...started('Read later'), state: 'done', endedAt: Date.now() },
    speakers: [{ id: 'call-1', track: 'call', label: 'Speaker 1' }],
    passages: [passage('call', 1000, 'Kept by Voice.')],
    marks: [],
    head: voice.head
  });
  const s2 = service(t, dir, file).init();
  await until(() => s2.connected, 'the connection');
  assert.equal(voice.commands.filter((c) => c.command === 'meeting.get').length, 0, 'nothing to fetch before the meeting is read');
  s2.get(M1);
  await until(() => ((s2.get(M1) || {}).passages || []).length === 1, 'the meeting from Voice');
  assert.equal(s2.get(M1).passages[0].text, 'Kept by Voice.');
});

test("a meeting.get answer doesn't undo the meeting's events past its head that Voice sent before it", async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const at = Date.now();
  voice.append('meeting.started', M1, started('Racing', at));
  voice.append('passage', M1, passage('call', 1000, 'Before the head.'));
  const s1 = service(t, dir, file).init();
  await until(() => s1.store.cursor().after === 2, 'the first replay');
  s1.close();
  for (let i = 0; i < 3; i++) voice.append('status', null, { recording: null, dictating: false });
  voice.forget(5);
  // Voice reads the snapshot at its head, and the transcript goes on: those events go out before the answer.
  voice.handlers['meeting.get'] = function () {
    const head = this.head;
    this.append('passage', M1, passage('call', 4000, 'After the head.'));
    this.append('meeting.state', M1, { state: 'processing', at: 5000, endedAt: at + 5000, reason: 'user' });
    return { meeting: { id: M1, ...started('Racing', at), state: 'recording', endedAt: null }, speakers: [], passages: [passage('call', 1000, 'Before the head.')], marks: [], head };
  };
  const s2 = service(t, dir, file);
  let refetched = false;
  s2.on('meeting', (p) => {
    if (p.kind === 'list') refetched = true;
  });
  s2.init();
  await until(() => refetched, 'the snapshot');
  const m = s2.get(M1);
  assert.deepEqual(m.passages.map((p) => p.text), ['Before the head.', 'After the head.']);
  assert.equal(m.meeting.state, 'processing');
  assert.equal(s2.store.cursor().after, voice.head);
});

test('a title or label cut at its limit never ends in half an emoji, which Voice could not read', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const s = service(t, dir, file, { hide: () => false }).init();
  await until(() => s.connected, 'the connection');
  const sent = [];
  const onText = voice.onText.bind(voice);
  voice.onText = (peer, text) => {
    sent.push(text);
    return onText(peer, text);
  };
  const emoji = '\u{1F600}';
  const first = await s.start({ title: `${'a'.repeat(199)}${emoji} and more` });
  await s.mark(first.meetingId, `${'b'.repeat(199)}${emoji}${emoji}`);
  await s.stop(first.meetingId);
  // main cuts in UTF-16 units first, which can leave the first half of an emoji at the end.
  await s.start({ title: `${emoji.repeat(149)}${emoji[0]}` });
  const args = voice.commands.filter((c) => c.command === 'meeting.start' || c.command === 'meeting.mark').map((c) => c.args.title || c.args.label);
  assert.deepEqual(args, [`${'a'.repeat(199)}${emoji}`, `${'b'.repeat(199)}${emoji}`, emoji.repeat(149)]);
  assert.equal(sent.length, 4);
  for (const text of sent) assert.doesNotMatch(text, /\\ud[89a-f][0-9a-f]{2}/i, 'no lone surrogate goes out');
});

test('a new Voice data folder: the cursor starts again from its first event, and Rukoo keeps the meetings it had', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Old folder'));
  voice.append('passage', M1, passage('mic', 0, 'Old words.'));
  const s = service(t, dir, file).init();
  const lists = [];
  s.on('meeting', (p) => p.kind === 'list' && lists.push(p));
  await until(() => s.store.cursor().after === 2, 'the first replay');
  const old = voice.instance;
  voice.reset();
  voice.append('meeting.started', M2, started('New folder'));
  voice.drop();
  // The entry is stored before the cursor moves past its event, so wait for both.
  await until(() => s.store.cursor().instance === voice.instance && s.store.entry(M2) && s.store.cursor().after === 1, 'the new instance');
  assert.notEqual(voice.instance, old);
  assert.deepEqual(voice.hellos.at(-1), { ...voice.hellos.at(-1), instance: old, after: 2 });
  assert.equal(s.store.cursor().after, 1);
  assert.ok(s.get(M1), 'the old meeting stays');
  assert.equal(lists.length, 1);
});

test('a command Voice never answers fails with timeout, and the connection keeps working', async (t) => {
  const { file, voice } = await setup(t);
  voice.writeDiscovery();
  const bridge = new VoiceBridge({ file, ...FAST }).start();
  t.after(() => bridge.close());
  await until(() => bridge.connected, 'the connection');
  voice.silent.add('meeting.stop');
  const err = await bridge.command('meeting.stop', { meeting: M1 }, { timeoutMs: 80 }).then(() => null, (e) => e);
  assert.equal(err.code, 'timeout');
  assert.deepEqual(await bridge.command('status.get'), { recording: null, dictating: false });
  // An error reply carries Voice's code and meeting.
  voice.status = { recording: M2, dictating: false };
  const busy = await bridge.command('meeting.start', {}).then(() => null, (e) => e);
  assert.equal(busy.code, 'busy');
  assert.equal(busy.meeting, M2);
  // Commands that don't follow the protocol never leave Rukoo.
  const bad = await bridge.command('meeting.stop', {}).then(() => null, (e) => e);
  assert.equal(bad.code, 'bad_request');
  // Nor does text with a lone surrogate, which Voice can't parse and so never answers.
  const asked = voice.commands.length;
  for (const args of [{ title: 'Half an emoji \uD83D' }, { context: { calendarEvent: 'x\uDE00' } }, { context: { ['\uD83Dkey']: 1 } }]) {
    const lone = await bridge.command('meeting.start', args, { timeoutMs: 2000 }).then(() => null, (e) => e);
    assert.equal(lone.code, 'bad_request', JSON.stringify(args));
  }
  assert.equal(voice.commands.length, asked, 'none of them reached Voice');
});

test('a connection cut halfway through a message comes back, and the replay brings that message once', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Cut'));
  const s = service(t, dir, file).init();
  await until(() => s.store.cursor().after === 1, 'the first event');
  const pending = s.bridge.command('status.get', {}, { timeoutMs: 5000 });
  voice.silent.add('status.get');
  const e = voice.append('passage', M1, passage('call', 2000, 'Halfway.'), { push: false });
  voice.drop(e);
  const err = await pending.then(() => null, (x) => x);
  assert.equal(err.code, 'offline', 'a command waiting on the dropped socket fails at once');
  await until(() => s.store.cursor().after === e.seq && s.bridge.connected, 'the replay');
  assert.equal(voice.hellos.at(-1).after, 1);
  assert.deepEqual(s.get(M1).passages.map((p) => p.text), ['Halfway.']);
});

test('a stale discovery file is left alone until Voice writes a new one, which connects at once', async (t) => {
  const { file, voice } = await setup(t);
  voice.writeDiscovery({ pid: await deadPid() });
  const bridge = new VoiceBridge({ file, backoffMs: [30000], pollMs: 50, heartbeatMs: 0 }).start();
  t.after(() => bridge.close());
  await until(() => bridge.state().reason === 'stale', 'the stale state');
  assert.equal(bridge.state().status, 'absent');
  await new Promise((r) => setTimeout(r, 150));
  assert.equal(voice.upgrades.length, 0, 'Rukoo never connects to a dead Voice');
  assert.ok(fs.existsSync(file), 'and never deletes its file');
  voice.writeDiscovery();
  await until(() => bridge.connected, 'the connection to the new Voice', 2000);
});

test('Voice started again on another port: the changed file connects at once, without waiting for the backoff', async (t) => {
  const { file, voice } = await setup(t);
  voice.writeDiscovery();
  const bridge = new VoiceBridge({ file, backoffMs: [30000], pollMs: 50, heartbeatMs: 0 }).start();
  t.after(() => bridge.close());
  await until(() => bridge.connected, 'the first connection');
  await voice.close();
  await until(() => !bridge.connected, 'the close');
  const next = await startVoiceServer({ file });
  t.after(() => next.close());
  next.writeDiscovery();
  await until(() => bridge.connected && next.hellos.length === 1, 'the connection to the new Voice', 2000);
  assert.equal(next.hellos[0].token, next.token);
});

test('a missing file, a damaged one and a Voice that speaks only newer versions each say why Rukoo is not connected', async (t) => {
  const { file, voice } = await setup(t);
  const bridge = new VoiceBridge({ file, ...FAST }).start();
  t.after(() => bridge.close());
  assert.deepEqual([bridge.state().status, bridge.state().reason], ['absent', 'missing']);
  // Only the owner may read it, as Voice writes it: otherwise macOS and Linux refuse the file for its permissions first.
  fs.writeFileSync(file, '{"port":', { mode: 0o600 });
  await until(() => bridge.state().reason === 'invalid', 'the invalid state');
  voice.writeDiscovery({ protocol: 3, minProtocol: 3 });
  await until(() => bridge.state().status === 'incompatible', 'the incompatible state');
  assert.equal(bridge.state().older, 'rukoo');
  assert.equal(voice.upgrades.length, 0);
  const none = new VoiceBridge({ file: null }).start();
  assert.deepEqual([none.state().status, none.state().reason], ['absent', 'none']);
});

test('a bad event is skipped and the ones after it still apply', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Strict'));
  voice.append('passage', M1, { ...passage('call', 1000, 'Broken'), revision: 0 });
  voice.append('passage', M1, passage('call', 3000, 'Fine.'));
  voice.append('agent.suggestion', M1, { text: 'A newer Voice' });
  const s = service(t, dir, file);
  const invalid = [];
  s.bridge.on('invalid', (x) => invalid.push(x));
  s.init();
  await until(() => s.store.cursor().after === 4, 'the replay');
  assert.deepEqual(invalid, [{ type: 'event', reason: 'revision' }]);
  assert.deepEqual(s.get(M1).passages.map((p) => p.text), ['Fine.']);
});

test('a wrong token keeps Rukoo trying, never connected', async (t) => {
  const { file, voice } = await setup(t);
  voice.writeDiscovery({ token: 'x'.repeat(43) });
  const bridge = new VoiceBridge({ file, ...FAST }).start();
  t.after(() => bridge.close());
  await until(() => voice.hellos.length >= 2, 'a second attempt');
  assert.equal(bridge.connected, false);
  assert.equal(bridge.state().status, 'connecting');
});

test('starting from Rukoo hides the windows first, waits a frame, and they stay hidden until Voice says the meeting ended', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const s = service(t, dir, file);
  const recording = [];
  s.on('recording', (id) => recording.push(id));
  s.init();
  await until(() => s.connected, 'the connection');
  assert.deepEqual(s.protects.map((p) => p.on), [false]);
  // When the command reaches Voice.
  let received = null;
  const onText = voice.onText.bind(voice);
  voice.onText = (peer, text) => {
    if (!received && text.includes('"meeting.start"')) received = Date.now();
    return onText(peer, text);
  };
  const { meetingId } = await s.start({ title: 'Stand-up', eventId: 'acc:cal:evt' });
  const command = voice.commands.find((c) => c.command === 'meeting.start');
  assert.deepEqual(command.args, { title: 'Stand-up', context: { calendarEvent: 'acc:cal:evt' } });
  const on = s.protects.find((p) => p.on);
  assert.ok(on, 'the windows were protected');
  // Protection takes effect with the next composition, so the command waits about 100 ms after it.
  assert.ok(received - on.at >= 90, `${received - on.at} ms between protecting and starting`);
  await until(() => s.store.entry(meetingId), 'the meeting');
  assert.equal(s.get(meetingId).meeting.eventId, 'acc:cal:evt');
  assert.equal(s.state().recording, meetingId);
  // The connection drops mid-meeting: Rukoo still believes it records, so the windows stay hidden.
  voice.drop();
  await until(() => !s.connected, 'the drop');
  assert.equal(s.protects.at(-1).on, true);
  await until(() => s.connected, 'the reconnect');
  await s.stop(meetingId);
  await until(() => s.protects.at(-1).on === false, 'protection to go');
  assert.deepEqual(recording, [meetingId, null]);
  assert.equal(s.get(meetingId).meeting.state, 'processing');
});

test('with the setting off nothing is hidden and nothing waits', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const s = service(t, dir, file, { hide: () => false, protectDelayMs: 5000 });
  s.init();
  await until(() => s.connected, 'the connection');
  const t0 = Date.now();
  await s.start({});
  assert.ok(Date.now() - t0 < 2000);
  assert.ok(s.protects.every((p) => !p.on));
});

test('a meeting Voice started is recording as far as Rukoo knows after a restart without Voice: the windows start hidden', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.append('meeting.started', M1, started('Live'));
  voice.append('meeting.state', M1, { state: 'recording', at: 0 });
  const s1 = service(t, dir, file).init();
  await until(() => s1.state().recording === M1, 'the recording');
  s1.close();
  voice.removeDiscovery();
  const s2 = service(t, dir, file).init();
  assert.deepEqual(s2.protects.map((p) => p.on), [true]);
  assert.equal(s2.state().status, 'absent');
});

test('a meeting Voice started: meeting.started, then meeting.state recording, tells the renderer and protects every window', (t) => {
  const dir = tempDir('rukoo-voice-started-');
  const s = service(t, dir, null).init();
  const states = [];
  const recording = [];
  s.on('state', (x) => states.push(x));
  s.on('recording', (id) => recording.push(id));
  assert.deepEqual(s.protects.map((p) => p.on), [false]);
  const now = Date.now();
  s.onEvent({ type: 'event', seq: 1, event: 'meeting.started', meeting: M1, createdAt: now, data: started('From Voice', now) });
  // meeting.started already marks the entry recording, so this event changes only what records now.
  s.onEvent({ type: 'event', seq: 2, event: 'meeting.state', meeting: M1, createdAt: now, data: { state: 'recording', at: 0 } });
  assert.equal(s.state().recording, M1);
  assert.ok(states.some((x) => x.recording === M1), 'a state event told the renderer');
  assert.deepEqual(recording, [M1]);
  assert.equal(s.protects.at(-1).on, true, 'the windows are protected');
  s.onEvent({ type: 'event', seq: 3, event: 'meeting.state', meeting: M1, createdAt: now, data: { state: 'processing', at: 5000, endedAt: now + 5000, reason: 'user' } });
  assert.equal(s.protects.at(-1).on, false);
  assert.deepEqual(recording, [M1, null]);
});

test('without a connection, meeting commands fail with offline', async (t) => {
  const dir = tempDir('rukoo-voice-off-');
  const s = service(t, dir, null).init();
  for (const call of [() => s.start({}), () => s.stop(M1), () => s.mark(M1, 'x'), () => s.frame(M1, 0), () => s.mediaOpen({ meeting: M1, kind: 'audio', start: 0, end: 1000 }), () => s.mediaRead('md-a1B2c3D4e5F6g7H8i9J0kL', 0, 1024)]) {
    const err = await call().then(() => null, (e) => e);
    assert.equal(err.code, 'offline');
  }
  assert.deepEqual(s.state(), { status: 'absent', reason: 'none', voiceVersion: null, protocol: null, older: null, recording: null, dictating: false, screenDefault: null });
});

test('a meeting Voice starts gets the one calendar event that fits, and a 1:1 gets the guest\'s name', async (t) => {
  const dir = tempDir('rukoo-voice-link-');
  const now = Date.now();
  const event = { id: 'acc:cal:e1', title: 'Sanne / Demo', start: now - 60000, end: now + 1740000, meeting: 'https://meet.example.com/x', attendees: [{ name: 'Demo User', address: 'demo@example.com', response: 'accepted', self: true }, { name: 'Sanne', address: 'sanne@example.com', response: 'accepted' }] };
  const calendars = { read: async () => ({ events: [event], failed: [] }), event: (id) => (id === event.id ? event : null) };
  const s = service(t, dir, null, { calendars }).init();
  s.onEvent({ type: 'event', seq: 1, event: 'meeting.started', meeting: M1, createdAt: now, data: started('', now) });
  await until(() => s.get(M1).meeting.eventId === event.id, 'the link');
  assert.deepEqual(s.get(M1).event, { id: event.id, title: 'Sanne / Demo', start: event.start, end: event.end, video: true });
  s.onEvent({ type: 'event', seq: 2, event: 'speaker', meeting: M1, createdAt: now, data: { id: 'call-1', track: 'call', label: 'Speaker 1' } });
  const speakers = s.speakers(M1);
  assert.equal(speakers[0].name, 'Sanne');
  assert.equal(speakers[0].from, 'auto');
  assert.deepEqual(speakers[0].suggestions.map((x) => x.address), ['sanne@example.com']);
  assert.deepEqual((await s.events(M1)).events.map((e) => e.id), [event.id]);
  // The user's own pick replaces it; unlinking keeps the name they gave.
  s.setSpeaker(M1, 'call-1', { name: 'Sanne de Vries', address: 'sanne@example.com' });
  assert.equal(s.setEvent(M1, null)[0].name, 'Sanne de Vries');
  // An old meeting from the first replay isn't worth a calendar read.
  s.onEvent({ type: 'event', seq: 3, event: 'meeting.started', meeting: M2, createdAt: now, data: started('Old', now - 2 * 86400000) });
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(s.get(M2).meeting.eventId, null);
});

// ---------- the screen (RUK-108) ----------

const pause = (ms) => new Promise((r) => setTimeout(r, ms));

// When each command reached the fake Voice, by name.
function arrivals(voice) {
  const seen = [];
  const onText = voice.onText.bind(voice);
  voice.onText = (peer, text) => {
    try {
      const msg = JSON.parse(text);
      if (msg.type === 'command') seen.push({ command: msg.command, args: msg.args, at: Date.now() });
    } catch (_) {
      // Not JSON: the fake ignores it too.
    }
    return onText(peer, text);
  };
  return (name) => seen.filter((c) => c.command === name);
}

test('a meeting Voice starts itself: Rukoo hides its windows, waits a frame, confirms, and stays hidden until the meeting ends', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.handlers['meeting.protected'] = () => ({});
  const sent = arrivals(voice);
  const s = service(t, dir, file).init();
  await until(() => s.connected, 'the connection');
  assert.equal(s.screenRecording(), false);
  voice.append('meeting.started', M1, { ...started('From the pill'), screen: 'frames' });
  voice.append('meeting.starting', M1, { screen: 'frames' });
  await until(() => sent('meeting.protected').length === 1, 'meeting.protected');
  assert.deepEqual(sent('meeting.protected')[0].args, { meeting: M1 });
  const on = s.protects.find((p) => p.on);
  assert.ok(on, 'the windows were protected');
  // Protection takes effect with the next composition, so the answer waits about 100 ms after it.
  const waited = sent('meeting.protected')[0].at - on.at;
  assert.ok(waited >= 90, `${waited} ms between protecting and confirming`);
  // No meeting.state yet: the start keeps the windows hidden, and notifications held.
  await pause(150);
  assert.equal(s.protects.at(-1).on, true);
  assert.equal(s.screenRecording(), true);
  // The meeting's state takes over from the start, and the end of the meeting lets go.
  voice.append('meeting.state', M1, { state: 'recording', at: 0 });
  await until(() => s.recording() === M1, 'the recording');
  assert.equal(s.starting, 0);
  assert.equal(s.protects.at(-1).on, true);
  assert.equal(s.screenRecording(), true, 'a frames meeting records the screen');
  voice.append('meeting.state', M1, { state: 'processing', at: 5000, endedAt: Date.now(), reason: 'user' });
  await until(() => s.protects.at(-1).on === false, 'protection to go');
  assert.equal(s.screenRecording(), false);
  assert.equal(sent('meeting.protected').length, 1);
});

test("a connection that drops between meeting.starting and Voice's status keeps the windows hidden past the hold, also across a restart of Rukoo", async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const sent = arrivals(voice);
  const s1 = service(t, dir, file, { startingHoldMs: 100 }).init();
  await until(() => s1.connected, 'the connection');
  voice.append('meeting.started', M1, { ...started('From a key'), screen: 'frames' });
  voice.append('meeting.starting', M1, { screen: 'frames' });
  await until(() => sent('meeting.protected').length === 1, 'meeting.protected');
  // Voice's status and the meeting's state never reach Rukoo: the connection drops, and Voice stays out of reach
  // while it records.
  voice.removeDiscovery();
  voice.drop();
  await until(() => !s1.connected, 'the drop');
  voice.append('meeting.state', M1, { state: 'recording', at: 0 });
  await pause(400);
  assert.equal(s1.protects.at(-1).on, true, 'still hidden after the hold');
  assert.equal(s1.screenRecording(), true, 'notifications still held back');
  // Rukoo restarts while Voice is out of reach: the windows start hidden.
  s1.close();
  const s2 = service(t, dir, file, { startingHoldMs: 100 }).init();
  assert.deepEqual(s2.protects.map((p) => p.on), [true]);
  assert.equal(s2.screenRecording(), true);
  // Voice is back and still records: its status keeps them hidden, and the end of the meeting lets go.
  voice.writeDiscovery();
  await until(() => s2.connected, 'the reconnect');
  assert.equal(s2.protects.at(-1).on, true);
  voice.append('meeting.state', M1, { state: 'processing', at: 5000, endedAt: Date.now(), reason: 'user' });
  await until(() => s2.protects.at(-1).on === false, 'protection to go');
  assert.equal(s2.screenRecording(), false);
});

test('a hold whose time runs out while the connection is down lasts until the next welcome says what records', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const sent = arrivals(voice);
  // The heartbeat brings a status that hasn't caught up with the start: nothing records, as far as it says.
  const s = service(t, dir, file, { startingHoldMs: 300, bridge: { ...FAST, heartbeatMs: 30 } }).init();
  await until(() => s.connected, 'the connection');
  voice.append('meeting.started', M1, { ...started('From the tray'), screen: 'video' });
  voice.append('meeting.starting', M1, { screen: 'video' });
  await until(() => sent('meeting.protected').length === 1, 'meeting.protected');
  await until(() => s.holds.has(M1), 'the hold');
  assert.equal(s.recording(), null);
  voice.removeDiscovery();
  voice.drop();
  await until(() => !s.connected, 'the drop');
  await until(() => !s.holds.has(M1), 'the hold to run out', 2000);
  await pause(100);
  assert.equal(s.protects.at(-1).on, true, 'still hidden');
  assert.equal(s.screenRecording(), true, 'notifications still held back');
  // The meeting ended while Rukoo was away: the welcome's status says nothing records, and the hold goes.
  voice.append('meeting.state', M1, { state: 'processing', at: 5000, endedAt: Date.now(), reason: 'user' });
  voice.writeDiscovery();
  await until(() => s.connected, 'the reconnect');
  await until(() => s.protects.at(-1).on === false, 'protection to go');
  assert.equal(s.starting, 0);
  assert.equal(s.screenRecording(), false);
});

test('a meeting.starting in the replay is history and gets no answer; the fresh one Voice sends after a reconnect does', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.handlers['meeting.protected'] = () => ({});
  voice.append('meeting.started', M1, { ...started('Started before Rukoo'), screen: 'video' });
  voice.append('meeting.starting', M1, { screen: 'video' });
  const sent = arrivals(voice);
  const s = service(t, dir, file).init();
  await until(() => s.store.cursor().after === 2, 'the replay');
  await pause(250);
  assert.equal(sent('meeting.protected').length, 0, 'Voice stopped waiting long ago');
  assert.equal(s.starting, 0);
  // The connection drops while Voice starts another meeting; Rukoo is back, and Voice asks it after the welcome.
  const hellos = voice.hellos.length;
  voice.drop();
  await until(() => voice.hellos.length > hellos && s.connected, 'the reconnect');
  assert.equal(voice.hellos.at(-1).after, 2);
  voice.append('meeting.started', M2, { ...started('From the tray'), screen: 'frames' });
  voice.append('meeting.starting', M2, { screen: 'frames' });
  await until(() => sent('meeting.protected').length === 1, 'meeting.protected');
  assert.deepEqual(sent('meeting.protected')[0].args, { meeting: M2 });
  assert.ok(s.protects.some((p) => p.on));
});

test("with hiding off Rukoo confirms at once and hides nothing, but still holds back its notifications", async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  voice.handlers['meeting.protected'] = () => ({});
  const sent = arrivals(voice);
  const s = service(t, dir, file, { hide: () => false, protectDelayMs: 5000 }).init();
  await until(() => s.connected, 'the connection');
  const t0 = Date.now();
  voice.append('meeting.starting', M1, { screen: 'frames' });
  await until(() => sent('meeting.protected').length === 1, 'meeting.protected');
  assert.ok(sent('meeting.protected')[0].at - t0 < 1000, 'no wait for a composition');
  assert.ok(s.protects.every((p) => !p.on));
  assert.equal(s.screenRecording(), true);
});

test("windows that don't take the protection get no meeting.protected, and Rukoo keeps trying until they do", async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const sent = arrivals(voice);
  // How hiding the windows fails: false is how main says a window didn't take it, a throw is the other way.
  let failure = false;
  const tries = [];
  const applied = [];
  const s = service(t, dir, file, {
    protectRetryMs: 40,
    protect: (on) => {
      tries.push({ on, failure });
      if (on && failure === 'throw') throw new Error('SetWindowDisplayAffinity failed');
      if (on && failure === 'false') return false;
      applied.push(on);
      return true;
    }
  }).init();
  const states = [];
  s.on('state', (x) => states.push(x));
  await until(() => s.connected, 'the connection');
  failure = 'false';
  voice.append('meeting.started', M1, { ...started('From the pill'), screen: 'frames' });
  voice.append('meeting.starting', M1, { screen: 'frames' });
  await pause(700);
  assert.equal(sent('meeting.protected').length, 0, 'Voice stops waiting after 500 ms and starts as without Rukoo');
  assert.equal(s.protectedNow, null, "Rukoo doesn't know what the windows show");
  assert.ok(tries.filter((x) => x.on && x.failure === 'false').length >= 3, 'it tries again');
  failure = 'throw';
  await until(() => tries.filter((x) => x.on && x.failure === 'throw').length >= 2, 'tries that throw');
  assert.ok(!applied.includes(true));
  // The windows take it at last: they stay hidden, and the renderer hears of it.
  const before = states.length;
  failure = false;
  await until(() => applied.at(-1) === true, 'the windows hidden');
  assert.equal(s.protectedNow, true);
  await until(() => states.length > before, 'a state for the renderer');
  const settled = tries.length;
  await pause(200);
  assert.equal(tries.length, settled, 'no more tries once it worked');
  assert.equal(sent('meeting.protected').length, 0, 'Voice stopped waiting long ago');
});

test('screenRecording: a recording meeting that records the screen, by what meeting.started said or else what the start asked', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const s = service(t, dir, file, { hide: () => false }).init();
  await until(() => s.connected, 'the connection');
  const off = await s.start({ screen: 'off' });
  await until(() => s.store.entry(off.meetingId), 'the meeting');
  assert.equal(s.screenRecording(), false, 'audio only');
  await s.stop(off.meetingId);
  await until(() => !s.recording(), 'the stop');
  const video = await s.start({ screen: 'video' });
  assert.equal(s.recording(), video.meetingId);
  assert.equal(s.screenRecording(), true);
  await s.stop(video.meetingId);
  await until(() => !s.recording(), 'the stop');
  // Before meeting.started arrives, only the screen the start asked for says what the meeting records. Voice's
  // default doesn't: the meeting may have started with another one.
  voice.append('status', null, { recording: null, dictating: false, screenDefault: 'off' });
  await until(() => s.state().screenDefault === 'off', "Voice's default");
  voice.handlers['meeting.start'] = function () {
    return { meeting: this.meetingId() };
  };
  const asked = await s.start({ screen: 'off' });
  assert.equal(s.recording(), asked.meetingId);
  assert.equal(s.screenRecording(), false, 'the start asked for audio only');
  const unsaid = await s.start({});
  assert.equal(s.recording(), unsaid.meetingId);
  assert.equal(s.screenRecording(), true, "Voice's default is audio only, but Rukoo doesn't know what this meeting got");
});

test("screenRecording: a meeting Rukoo has no meeting.started for counts as recording the screen, whatever Voice's default", async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  // Rukoo connects while Voice records a meeting whose meeting.started hasn't reached it.
  voice.status = { recording: M1, dictating: false, screenDefault: 'off' };
  const s = service(t, dir, file, { hide: () => false }).init();
  await until(() => s.connected, 'the connection');
  assert.equal(s.recording(), M1);
  assert.equal(s.state().screenDefault, 'off');
  assert.equal(s.screenRecording(), true, 'the meeting may record the screen');
  // Its meeting.started says what it records.
  voice.append('meeting.started', M1, { ...started('Board review'), screen: 'off' });
  await until(() => s.store.entry(M1), 'the meeting');
  assert.equal(s.screenRecording(), false, 'audio only');
});

test('pause and resume do not release the notification hold of a pending screen change', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.append('meeting.started', M1, started('Pending screen change'));
  voice.append('status', null, { recording: M1, dictating: false });
  voice.writeDiscovery();
  const s = service(t, dir, file, { hide: () => false }).init();
  await until(() => s.connected && s.store.entry(M1), 'the meeting');
  voice.append('meeting.starting', M1, { screen: 'frames', request: 'mode-round' });
  await until(() => s.holds.has(M1), 'the screen protection hold');
  for (const state of ['paused', 'recording']) {
    voice.append('meeting.state', M1, { state, at: 1000 });
    await until(() => s.store.entry(M1).state === state, state);
    assert.equal(s.screenRecording(), true, 'notifications remain held before the first frame');
    assert.ok(s.starting > 0);
  }
  voice.append('mark', M1, { id: 'mode-frames', kind: 'screen_mode', at: 2000, until: null, track: 'screen', detail: 'user', label: 'frames' });
  await until(() => s.starting === 0, 'capture to take over the hold');
  voice.append('mark', M1, { id: 'mode-off', kind: 'screen_mode', at: 3000, until: null, track: 'screen', detail: 'user', label: 'off' });
  await until(() => !s.screenRecording(), 'Off to release notifications');
});

test('a live mode mark resolves reconnect uncertainty and a late screen reply cannot override it', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.append('meeting.started', M1, { ...started('Mode changes'), screen: 'frames' });
  voice.append('status', null, { recording: M1, dictating: false });
  voice.writeDiscovery();
  const s = service(t, dir, file, { hide: () => false }).init();
  await until(() => s.connected && s.store.entry(M1), 'the meeting');
  let resolve;
  s.screenInfo = () => new Promise((r) => { resolve = r; });
  s.onWelcome({ protocol: 2, instance: voice.instance, head: voice.head, oldest: 1, replay: 'partial', status: voice.status });
  assert.equal(s.screenRecording(), true);
  voice.append('mark', M1, { id: 'mode-off', kind: 'screen_mode', at: 1000, until: null, track: 'screen', detail: 'user', label: 'off' });
  await until(() => s.store.entry(M1).screen === 'off', 'the Off mark');
  assert.equal(s.screenRecording(), false, 'the live mark resolves uncertainty');
  resolve({ screen: 'frames' });
  await pause(10);
  assert.equal(s.screenRecording(), false, 'a stale screen.get cannot turn notifications off again');
});

test('screenRecording: not after a dropped mark says the screen recording stopped for good', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const s = service(t, dir, file, { hide: () => false }).init();
  await until(() => s.connected, 'the connection');
  voice.append('meeting.started', M1, { ...started('Long review'), screen: 'frames' });
  voice.append('meeting.state', M1, { state: 'recording', at: 0 });
  await until(() => s.recording() === M1, 'the recording');
  assert.equal(s.screenRecording(), true);
  // The video stopping leaves the frames, which still record the screen.
  voice.append('mark', M1, { id: 'dropped-video', kind: 'video_dropped', at: 1_700_000, until: null, track: 'screen', detail: 'disk', label: null });
  await until(() => s.store.meeting(M1).marks.length === 1, 'the video_dropped mark');
  assert.equal(s.screenRecording(), true, 'the frames go on');
  voice.append('mark', M1, { id: 'dropped-frames', kind: 'dropped', at: 1_800_000, until: null, track: 'screen', detail: 'disk', label: null });
  await until(() => s.store.meeting(M1).marks.length === 2, 'the dropped mark');
  assert.equal(s.recording(), M1, 'the audio still records');
  assert.equal(s.screenRecording(), false, 'Voice records no screen any more');
});

test('clip, screen and monitor commands reach Voice, and answers of the wrong shape fail', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const s = service(t, dir, file).init();
  await until(() => s.connected, 'the connection');
  const clipPath = path.join(dir, 'screen', 'clips', 'c-000060000-000075000-000001.mp4');
  voice.handlers['clip.get'] = (a) => ({ clips: [{ path: clipPath, start: a.start - 150, end: a.end + 120, width: 1920, height: 1080, bytes: 2481152, extra: 'dropped' }] });
  assert.deepEqual(await s.clip(M1, 60000, 75000), { clips: [{ path: clipPath, start: 59850, end: 75120, width: 1920, height: 1080, bytes: 2481152 }] });
  assert.deepEqual(voice.commands.at(-1).args, { meeting: M1, start: 60000, end: 75000 });
  voice.handlers['clip.get'] = () => ({ clips: [] });
  assert.deepEqual(await s.clip(M1, 0, 0), { clips: [] }, 'nothing cut yet');
  for (const bad of [{}, { clips: 'x' }, { clips: [{ path: 'relative.mp4', start: 0, end: 1, width: 1, height: 1, bytes: 1 }] }, { clips: [{ path: clipPath, start: 5, end: 1, width: 1, height: 1, bytes: 1 }] }, { clips: [{ path: `${clipPath}.exe`, start: 0, end: 1, width: 1, height: 1, bytes: 1 }] }]) {
    voice.handlers['clip.get'] = () => bad;
    const err = await s.clip(M1, 0, 1).then(() => null, (e) => e);
    assert.equal(err && err.code, 'failed', JSON.stringify(bad));
  }
  const screen = {
    screen: 'frames',
    state: 'recording',
    video: false,
    frames: true,
    monitor: '\\\\.\\DISPLAY1',
    monitors: [
      { id: '\\\\.\\DISPLAY1', name: 'DELL U2720Q', width: 2560, height: 1440, primary: true },
      { id: '\\\\.\\DISPLAY2', name: 'Display 2', width: 1440, height: 2560, primary: false }
    ]
  };
  voice.handlers['screen.get'] = () => screen;
  assert.deepEqual(await s.screenInfo(M1), screen);
  voice.handlers['screen.get'] = () => ({ ...screen, state: 'dancing' });
  assert.equal((await s.screenInfo(M1).then(() => null, (e) => e)).code, 'failed');
  voice.handlers['screen.get'] = () => ({ ...screen, monitors: [{ id: '', name: 'x', width: 1, height: 1, primary: false }] });
  assert.equal((await s.screenInfo(M1).then(() => null, (e) => e)).code, 'failed');
  voice.handlers['screen.select'] = (a) => ({ monitor: a.monitor });
  assert.equal(await s.selectMonitor(M1, '\\\\.\\DISPLAY2'), '\\\\.\\DISPLAY2');
  assert.deepEqual(voice.commands.at(-1).args, { meeting: M1, monitor: '\\\\.\\DISPLAY2' });
  voice.handlers['screen.select'] = () => ({ error: { code: 'not_found', message: 'No monitor with that id.' } });
  assert.equal((await s.selectMonitor(M1, 'x').then(() => null, (e) => e)).code, 'not_found');
  // A Voice 0.1 doesn't know them: Rukoo hides the Screen button and says so in the tool notes.
  voice.handlers['screen.get'] = (a, peer) => ({ error: { code: 'unknown_command', message: "Voice doesn't know the command `screen.get`." } });
  const old = await s.screenInfo(M1).then(() => null, (e) => e);
  assert.equal(old.code, 'unknown_command');
  assert.equal(old.constructor.name, 'VoiceError');
  // Nothing goes out without a connection.
  s.bridge.close();
  for (const call of [() => s.clip(M1, 0, 1), () => s.screenInfo(M1), () => s.selectMonitor(M1, 'x')]) assert.equal((await call().then(() => null, (e) => e)).code, 'offline');
});

test('frames reach the renderer in deltas, and frame.get fills in the text a snapshot left out', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const s = service(t, dir, file).init();
  const deltas = [];
  s.on('meeting', (p) => p.kind === 'meeting' && deltas.push(p));
  await until(() => s.connected, 'the connection');
  voice.append('meeting.started', M1, { ...started('Review'), screen: 'frames' });
  voice.append('frame', M1, { id: 'f-000412000', at: 412000, width: 1920, height: 1080, text: 'Q3 roadmap' });
  await until(() => deltas.some((d) => d.frames), 'a delta with the frame');
  assert.deepEqual(deltas.find((d) => d.frames).frames.map((f) => [f.id, f.text]), [['f-000412000', 'Q3 roadmap']]);
  // A meeting.get answer with an older frame whose text Voice left out to keep the reply small.
  s.store.applySnapshot(M1, {
    meeting: { id: M1, ...started('Review'), screen: 'frames', state: 'recording', endedAt: null },
    speakers: [],
    passages: [],
    marks: [],
    frames: [
      { id: 'f-000100000', at: 100000, width: 1920, height: 1080, text: '', textOmitted: true },
      { id: 'f-000412000', at: 412000, width: 1920, height: 1080, text: 'Q3 roadmap' }
    ],
    head: voice.head
  });
  assert.equal(s.get(M1).frames[0].textOmitted, true);
  voice.handlers['frame.get'] = (a) => ({ frame: { id: 'f-000100000', at: 100000, width: 1920, height: 1080, text: 'Agenda' }, image: { type: 'image/jpeg', data: '/9j/2Q==' } });
  deltas.length = 0;
  const res = await s.frame(M1, 100000);
  assert.equal(res.image.type, 'image/jpeg');
  assert.deepEqual(voice.commands.at(-1).args, { meeting: M1, at: 100000 });
  assert.equal(s.get(M1).frames[0].text, 'Agenda');
  await until(() => deltas.some((d) => d.frames), 'a delta with the text');
  assert.deepEqual(deltas.find((d) => d.frames).frames.map((f) => [f.id, f.text, f.textOmitted]), [['f-000100000', 'Agenda', undefined]]);
});

test("Voice's screen default reaches the state, and a change of it alone tells the renderer", async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.status = { recording: null, dictating: false, screenDefault: 'video' };
  voice.writeDiscovery();
  const s = service(t, dir, file).init();
  await until(() => s.connected, 'the connection');
  assert.equal(s.state().screenDefault, 'video');
  const states = [];
  s.on('state', (x) => states.push(x));
  voice.append('status', null, { recording: null, dictating: false, screenDefault: 'frames' });
  await until(() => states.some((x) => x.screenDefault === 'frames'), 'the new default');
});

test('two clips and a heartbeat: Rukoo sends one clip.get at a time, nothing times out and the socket stays open', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  // Voice runs a connection's commands one at a time, and a cut takes up to 4 s.
  const log = [];
  const queue = [];
  let busy = false;
  const onText = voice.onText.bind(voice);
  const pump = () => {
    if (busy || !queue.length) return;
    busy = true;
    const { peer, text, msg } = queue.shift();
    log.push({ got: msg.command, at: Date.now() });
    if (msg.command !== 'clip.get') {
      onText(peer, text);
      busy = false;
      return pump();
    }
    setTimeout(() => {
      peer.send({ type: 'reply', id: msg.id, ok: true, result: { clips: [] } });
      log.push({ answered: 'clip.get', at: Date.now() });
      busy = false;
      pump();
    }, 4000);
  };
  voice.onText = (peer, text) => {
    const msg = JSON.parse(text);
    if (!peer.hello || msg.type !== 'command') return onText(peer, text);
    log.push({ arrived: msg.command, at: Date.now() });
    queue.push({ peer, text, msg });
    pump();
  };
  const s = service(t, dir, file, { bridge: { ...FAST, heartbeatMs: 1000 } }).init();
  await until(() => s.connected, 'the connection');
  const closed = [];
  s.bridge.on('closed', () => closed.push(Date.now()));
  const both = await Promise.all([s.clip(M1, 0, 60000), s.clip(M1, 60000, 120000)]);
  assert.deepEqual(both, [{ clips: [] }, { clips: [] }]);
  const clips = log.filter((x) => x.arrived === 'clip.get');
  const firstAnswer = log.find((x) => x.answered === 'clip.get');
  assert.equal(clips.length, 2);
  assert.ok(clips[1].at >= firstAnswer.at, 'the second clip.get reached Voice only after the first answer');
  assert.ok(log.filter((x) => x.got === 'status.get').length >= 3, 'the heartbeats went out and were answered meanwhile');
  assert.deepEqual(closed, [], 'no heartbeat timed out');
  assert.equal(voice.hellos.length, 1, 'no reconnect');
  assert.equal(s.connected, true);
});

test('while the discovery file names a Voice that runs, Rukoo tries again within 2 s; otherwise it keeps the backoff', async (t) => {
  const { file, voice } = await setup(t);
  voice.writeDiscovery();
  let running = true;
  const bridge = new VoiceBridge({ file, backoffMs: [30000], liveRetryMs: 100, pollMs: 60000, heartbeatMs: 0, alive: () => running }).start();
  t.after(() => bridge.close());
  await until(() => bridge.connected, 'the connection');
  assert.equal(new VoiceBridge({ file }).liveRetryMs, 2000);
  // The socket closes and Voice keeps running: a Voice restarting its side, or one that stopped answering a while.
  voice.drop();
  await until(() => voice.hellos.length === 2 && bridge.connected, 'the second connection', 1500);
  assert.equal(bridge.retryWait, 100);
  // Voice refused the connection: asking sooner won't change its answer.
  const token = voice.token;
  voice.token = 'y'.repeat(43);
  voice.drop();
  await until(() => voice.hellos.length === 3, 'the refused hello');
  await until(() => bridge.retryWait === 30000, 'the backoff after the refusal');
  // The file names a process that is gone: the backoff too.
  bridge.close();
  voice.token = token;
  const next = new VoiceBridge({ file, backoffMs: [30000], liveRetryMs: 100, pollMs: 60000, heartbeatMs: 0, alive: () => running }).start();
  t.after(() => next.close());
  voice.writeDiscovery();
  await until(() => next.connected, 'the connection');
  running = false;
  voice.drop();
  await until(() => next.retryWait !== null, 'the retry');
  assert.equal(next.retryWait, 30000);
});

test('speakers come back in one order, the microphone and then the call by number, whatever order Voice sent them in', (t) => {
  const dir = tempDir('rukoo-voice-order-');
  const s = service(t, dir, null).init();
  const now = Date.now();
  s.onEvent({ type: 'event', seq: 1, event: 'meeting.started', meeting: M1, createdAt: now, data: started('Three voices', now) });
  let seq = 1;
  // Voice sends a call speaker right before its first passage, so call-10 can come before call-2.
  for (const [id, track] of [['call-1', 'call'], ['call-10', 'call'], ['mic', 'mic'], ['call-2', 'call']]) {
    s.onEvent({ type: 'event', seq: ++seq, event: 'speaker', meeting: M1, createdAt: now, data: { id, track, label: track === 'mic' ? 'You' : `Speaker ${id.slice(5)}` } });
  }
  assert.deepEqual(s.speakers(M1).map((x) => x.id), ['mic', 'call-1', 'call-2', 'call-10']);
  assert.deepEqual(s.get(M1).speakers.map((x) => x.id), ['mic', 'call-1', 'call-2', 'call-10'], 'meetingGet gives the same order');
  assert.deepEqual(s.setSpeaker(M1, 'call-10', { name: 'Joris' }).map((x) => [x.id, x.name]), [['mic', null], ['call-1', null], ['call-2', null], ['call-10', 'Joris']]);
});

test("a 1:1's name Rukoo filled in stays when a second voice turns up, gets no second guess, and asks the user to check it", async (t) => {
  const dir = tempDir('rukoo-voice-autocheck-');
  const now = Date.now();
  const event = { id: 'acc:cal:e1', title: 'Sanne / Demo', start: now - 60000, end: now + 1740000, meeting: 'https://meet.example.com/x', attendees: [{ name: 'Demo User', address: 'demo@example.com', response: 'accepted', self: true }, { name: 'Sanne', address: 'sanne@example.com', response: 'accepted' }] };
  const calendars = { read: async () => ({ events: [event], failed: [] }), event: (id) => (id === event.id ? event : null) };
  const s = service(t, dir, null, { calendars }).init();
  const deltas = [];
  s.on('meeting', (p) => p.kind === 'meeting' && p.speakers && deltas.push(p.speakers));
  s.onEvent({ type: 'event', seq: 1, event: 'meeting.started', meeting: M1, createdAt: now, data: started('', now) });
  await until(() => s.get(M1).meeting.eventId === event.id, 'the link');
  s.onEvent({ type: 'event', seq: 2, event: 'speaker', meeting: M1, createdAt: now, data: { id: 'call-1', track: 'call', label: 'Speaker 1' } });
  const one = s.speakers(M1)[0];
  assert.deepEqual([one.name, one.from, one.autoCheck], ['Sanne', 'auto', false], 'one voice: no doubt yet');
  s.onEvent({ type: 'event', seq: 3, event: 'speaker', meeting: M1, createdAt: now, data: { id: 'call-2', track: 'call', label: 'Speaker 2' } });
  const [first, second] = s.speakers(M1);
  assert.deepEqual([first.id, first.name, first.from, first.autoCheck], ['call-1', 'Sanne', 'auto', true]);
  assert.deepEqual([second.id, second.name, second.from, second.autoCheck], ['call-2', null, null, false], 'the second voice gets no name by itself');
  assert.deepEqual(second.suggestions.map((c) => [c.name, c.takenBy]), [['Sanne', ['call-1']]]);
  // The renderer hears it too.
  await until(() => deltas.some((list) => list.some((x) => x.id === 'call-2')), 'the delta with both voices');
  assert.equal(deltas.at(-1).find((x) => x.id === 'call-1').autoCheck, true);
  // The user confirms the guess: it is theirs now, and nothing asks any more.
  const confirmed = s.setSpeaker(M1, 'call-1', { name: 'Sanne', address: 'sanne@example.com' });
  assert.deepEqual([confirmed[0].name, confirmed[0].from, confirmed[0].autoCheck], ['Sanne', 'user', false]);
});

test('media.open and media.read reach Voice with the protocol\'s arguments and a 10 s limit, and are checked before they leave', async (t) => {
  const { dir, file, voice } = await setup(t);
  voice.writeDiscovery();
  const MEDIA = 'md-a1B2c3D4e5F6g7H8i9J0kL';
  voice.handlers['media.open'] = (args) => ({ media: MEDIA, ready: true, type: 'audio/wav', size: 19200044, start: args.start, end: args.end === undefined ? args.start : args.end });
  voice.handlers['media.read'] = (args) => ({ offset: args.offset, data: 'UklGRiQAAABXQVZF' });
  const s = service(t, dir, file).init();
  await until(() => s.connected, 'the connection');
  const limits = [];
  const command = s.bridge.command.bind(s.bridge);
  s.bridge.command = (name, args, opts) => {
    limits.push([name, opts && opts.timeoutMs]);
    return command(name, args, opts);
  };

  assert.deepEqual(await s.mediaOpen({ meeting: M1, kind: 'audio', start: 600000, end: 1200000 }), { media: MEDIA, ready: true, type: 'audio/wav', size: 19200044, start: 600000, end: 1200000 });
  assert.deepEqual(voice.commands.at(-1).args, { meeting: M1, kind: 'audio', start: 600000, end: 1200000 });
  // A frame has no end: the argument is left out, not sent as null.
  await s.mediaOpen({ meeting: M1, kind: 'frame', start: 412500, end: undefined });
  assert.deepEqual(voice.commands.at(-1).args, { meeting: M1, kind: 'frame', start: 412500 });
  assert.deepEqual(await s.mediaRead(MEDIA, 1048576, 1048576), { offset: 1048576, data: 'UklGRiQAAABXQVZF' });
  assert.deepEqual(voice.commands.at(-1).args, { media: MEDIA, offset: 1048576, length: 1048576 });
  assert.deepEqual(limits, [['media.open', 10000], ['media.open', 10000], ['media.read', 10000]]);

  // What the protocol refuses never leaves Rukoo.
  const sent = voice.commands.length;
  for (const call of [
    () => s.mediaOpen({ meeting: M1, kind: 'transcript', start: 0 }),
    () => s.mediaOpen({ meeting: M1, kind: 'audio', start: 5000, end: 4000 }),
    () => s.mediaOpen({ meeting: M1, kind: 'audio', start: 0, end: 3600001 }),
    () => s.mediaRead('md-short', 0, 1024),
    () => s.mediaRead(MEDIA, 0, 0),
    () => s.mediaRead(MEDIA, 0, 1048577),
    () => s.mediaRead(MEDIA, -1, 1024)
  ]) {
    const err = await call().then(() => null, (e) => e);
    assert.equal(err && err.code, 'bad_request');
  }
  assert.equal(voice.commands.length, sent);

  // Voice's own answers come through with their code, and a read Voice never answers ends at the limit.
  voice.handlers['media.open'] = () => ({ error: { code: 'not_available', message: "This meeting didn't record video." } });
  const missing = await s.mediaOpen({ meeting: M1, kind: 'video', start: 0, end: 300000 }).then(() => null, (e) => e);
  assert.equal(missing.code, 'not_available');
  s.mediaTimeoutMs = 80;
  voice.silent.add('media.read');
  const slow = await s.mediaRead(MEDIA, 0, 1024).then(() => null, (e) => e);
  assert.equal(slow.code, 'timeout');
  assert.match(slow.message, /within 80 ms/);
});
