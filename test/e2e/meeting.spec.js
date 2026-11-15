// End-to-end tests for the meeting view: the real app with the demo account, the fake agents (SEM_AGENT_FAKE=1) and a
// fake Rukoo Voice (test/fixtures/voice-server.js), paired through <SEM_DATA_DIR>/voice-bridge.json as voice/PROTOCOL.md
// says for a test profile. Nothing here reaches an installed Voice. Screenshots for review go to tmp/e2e-shots/meeting
// in the checkout, which Git ignores; each run replaces them.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');
const { FakeVoice, startVoiceServer, syntheticMedia } = require('../fixtures/voice-server');

const ROOT = path.join(__dirname, '..', '..');
const SHOTS = path.join(ROOT, 'tmp', 'e2e-shots', 'meeting');
// Electron hides windows from capture only on Windows and macOS (main.js protectWindows).
const CAN_HIDE = process.platform === 'win32' || process.platform === 'darwin';

let app;
let win;
let dataDir;
let voice;

async function launch(size = [1500, 950]) {
  // Media links live 4 s instead of 5 minutes, so a test sees one expire.
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1', SEM_AGENT_FAKE: '1', SEM_AGENT_FAKE_SCALE: '0.5', SEM_MEDIA_LINK_TTL_MS: '4000' };
  delete env.ELECTRON_RUN_AS_NODE;
  // The pairing goes through the profile, never a path from the shell that started the tests.
  delete env.RUKOO_VOICE_BRIDGE;
  app = await electron.launch({ args: ['.'], cwd: ROOT, env });
  win = await app.firstWindow();
  await setSize(...size);
}

const setSize = (w, h) => app.evaluate(({ BrowserWindow }, [w, h]) => BrowserWindow.getAllWindows()[0].setSize(w, h), [w, h]);
const connected = () => app.evaluate(() => global.__semVoice.connected);
const isProtected = () => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().map((w) => w.isContentProtected()));

// ---------- demo screen frames ----------

// A small PNG encoder, so the fake Voice can send frame images without a file in the repository.
const CRC = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function png(width, height, paint) {
  const stride = width * 3 + 1;
  const raw = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) raw.set(paint(x, y), y * stride + 1 + x * 3);
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(crc32(body), body.length + 4);
    return out;
  };
  const head = Buffer.alloc(13);
  head.writeUInt32BE(width, 0);
  head.writeUInt32BE(height, 4);
  head[8] = 8;
  head[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', head), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]).toString('base64');
}
// A demo slide in one accent colour: a title band, three lines of "text" and a bar chart. Nothing real.
function slide([r, g, b], bars) {
  const inside = (x, y, x0, y0, x1, y1) => x >= x0 && x < x1 && y >= y0 && y < y1;
  return png(480, 270, (x, y) => {
    if (y < 48) return [r, g, b];
    if (inside(x, y, 32, 78, 290, 90) || inside(x, y, 32, 108, 250, 120) || inside(x, y, 32, 138, 270, 150)) return [96, 102, 116];
    for (let i = 0; i < bars.length; i++) if (inside(x, y, 320 + i * 34, 230 - bars[i], 344 + i * 34, 230)) return [r, g, b];
    if (inside(x, y, 312, 230, 456, 232)) return [190, 194, 204];
    return [250, 250, 252];
  });
}
const SLIDES = [slide([52, 92, 196], [40, 70, 110, 150]), slide([24, 140, 104], [120, 90, 60, 100]), slide([196, 98, 40], [60, 140, 80, 120])];
const frameId = (at) => `f-${String(at).padStart(9, '0')}`;

// Voice 0.1 has no pause handler in the fixture; these answer as Voice does.
function voiceHandlers(v) {
  v.handlers['screen.get'] = function (args) {
    const started = this.events.find((e) => e.event === 'meeting.started' && e.meeting === args.meeting)?.data;
    const mode = this.events.filter((e) => e.event === 'mark' && e.meeting === args.meeting && e.data.kind === 'screen_mode').at(-1)?.data.label || started?.screen || 'off';
    return { screen: mode, state: mode === 'off' ? 'off' : 'recording', frames: mode !== 'off', video: mode === 'video', monitor: 'demo', monitors: [{ id: 'demo', name: 'Demo monitor', width: 1920, height: 1080, primary: true }] };
  };
  v.handlers['screen.set'] = function (args) {
    const at = this.events.filter((e) => e.meeting === args.meeting).reduce((at, e) => Math.max(at, e.data.at || e.data.end || 0), 0) + 1000;
    this.append('mark', args.meeting, { id: `screen-mode-${at}`, kind: 'screen_mode', at, until: null, track: 'screen', detail: 'user', label: args.screen });
    return { screen: args.screen };
  };
  v.handlers['meeting.pause'] = function (args) {
    if (this.status.recording !== args.meeting) return { error: { code: 'not_recording', message: 'Not recording.' } };
    this.append('meeting.state', args.meeting, { state: 'paused', at: 5000 });
    return {};
  };
  v.handlers['meeting.resume'] = function (args) {
    if (this.status.recording !== args.meeting) return { error: { code: 'not_recording', message: 'Not recording.' } };
    this.append('meeting.state', args.meeting, { state: 'recording', at: 6000 });
    return {};
  };
  // The frame at or before at, as Voice answers frame.get, with a demo slide for its image.
  v.handlers['frame.get'] = function (args) {
    const frames = this.events.filter((e) => e.event === 'frame' && e.meeting === args.meeting).map((e) => e.data);
    const f = frames.filter((x) => x.at <= args.at).sort((a, b) => b.at - a.at)[0];
    if (!f) return { error: { code: 'not_found', message: 'No frame at or before that time.' } };
    const n = frames.sort((a, b) => a.at - b.at).indexOf(f);
    return { frame: f, image: { type: 'image/png', data: SLIDES[n % SLIDES.length] } };
  };
}

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-meeting-e2e-');
  voice = await startVoiceServer({ file: path.join(dataDir, 'voice-bridge.json') });
  voiceHandlers(voice);
  voice.writeDiscovery();
  await launch();
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await expect.poll(connected, { timeout: 10000 }).toBe(true);
});

test.afterEach(async () => {
  await closeApp(app);
  await voice.close();
  removeDir(dataDir);
});

// ---------- a scripted meeting ----------

const pid = (track, start) => `${track}-${String(start).padStart(9, '0')}`;

const MIC = { id: 'mic', track: 'mic', label: 'You' };
const CALL_1 = { id: 'call-1', track: 'call', label: 'Speaker 1' };
// A call voice as Voice's speaker labels send it: call-N, "Speaker N".
const callVoice = (n) => ({ id: `call-${n}`, track: 'call', label: `Speaker ${n}` });

// A meeting Voice starts on its own, as from its pill: the events Voice sends. screen: what it records of the screen.
// speakers: the voices it sends at the start; without speaker labels that is you and the whole call as one voice.
function voiceStarts({ title = '', startedAt = Date.now(), context = null, screen = 'off', speakers = [MIC, CALL_1] } = {}) {
  const id = voice.meetingId();
  voice.append('meeting.started', id, { title, startedAt, tracks: ['mic', 'call'], screen, startedBy: 'voice', context });
  voice.append('meeting.state', id, { state: 'recording', at: 0 });
  voice.append('status', null, { recording: id, dictating: false });
  for (const s of speakers) voice.append('speaker', id, s);
  return id;
}

// One passage. push: false keeps it in Voice's log without sending it, as while Rukoo is away.
function say(id, track, start, text, { push = true, ...more } = {}) {
  return voice.append('passage', id, { id: pid(track, start), track, speaker: track === 'mic' ? 'mic' : 'call-1', start, end: start + 3000, text, revision: 1, dictation: false, language: 'en', ...more }, { push });
}

// A moment the screen changed, with the text Voice read on it.
function frameAt(id, at, text, { width = 1920, height = 1080, push = true } = {}) {
  return voice.append('frame', id, { id: frameId(at), at, width, height, text }, { push });
}

// The at of every frame.get Rukoo sent, in order.
const frameGets = () => voice.commands.filter((c) => c.command === 'frame.get').map((c) => c.args.at);

// The stand-up the demo calendar has every weekday at 09:30: today's while it isn't long past, else the next one. A
// meeting at that time stays within the 12 hours in which Rukoo links a meeting Voice started to its event.
function standupAt(now = new Date()) {
  const d = new Date(now);
  d.setHours(9, 30, 0, 0);
  const weekday = (x) => x.getDay() >= 1 && x.getDay() <= 5;
  if (!(weekday(d) && now.getHours() < 15)) {
    do d.setDate(d.getDate() + 1);
    while (!weekday(d));
  }
  return d.getTime();
}

const view = () => win.locator('.meeting');
const rows = () => win.locator('.meeting .mt-transcript .mt-passage');
const frame = (at) => win.locator(`.meeting .mt-frame[data-id="${frameId(at)}"]`);
const menu = () => win.locator('.menu');
const indicator = () => win.locator('.titlebar [data-meeting-indicator]');
const person = (id) => win.locator(`.meeting .mt-person[data-speaker="${id}"]`);
const pane = () => win.locator('.agentpane');
const transcript = () => win.locator('.agentpane .ap-transcript');
const input = () => win.locator('.agentpane .bui-pb__ta');
const chip = (label) => win.locator('.agentpane .bui-sugg__chip', { hasText: label });
const settled = () => pane().evaluate((el) => Promise.all(el.getAnimations().map((a) => a.finished)));

async function openMeetings() {
  await win.keyboard.press('Control+3');
  await expect(view()).toBeVisible();
  await expect(win.locator('[data-mode="meeting"]')).toHaveAttribute('aria-selected', 'true');
}

async function setTheme(theme) {
  await win.evaluate((t) => window.mail.call('updateSettings', { theme: t }), theme);
  await expect(win.locator('html')).toHaveClass(theme === 'light' ? /light/ : /^(?!.*light).*$/);
}

async function setLanguage(language) {
  await win.evaluate((l) => window.mail.call('updateSettings', { language: l }), language);
  await expect(win.locator('html')).toHaveAttribute('lang', language);
  await expect(win.locator('.sidebar-mode [data-mode="mail"] .seg-label')).toHaveText(language === 'nl' ? 'E-mail' : 'Mail');
}

async function shot(name) {
  fs.mkdirSync(SHOTS, { recursive: true });
  // The shimmer and the clock keep moving; a moment lets the last change paint.
  await win.waitForTimeout(400);
  await win.screenshot({ path: path.join(SHOTS, `${name}.png`) });
}

// ---------- tests ----------

test('rename keeps a local title across restart, validates length, and deletion keeps the chat', async () => {
  const id = voiceStarts({ title: 'Launch review' });
  say(id, 'call', 1000, 'We agreed to ship on Thursday.');
  await openMeetings();
  await win.keyboard.press('Control+j');
  await chip('Catch me up').click();
  await expect(transcript().locator('.bui-ub')).toHaveText('Catch me up');
  await expect(transcript().locator('[data-type="assistant"]').last()).toBeVisible();
  await win.keyboard.press('Control+j');
  await view().locator('[data-mt="rename"]').click();
  const title = () => view().locator('.mt-title-input');
  await title().fill('Cancelled title');
  await win.keyboard.press('Escape');
  await expect(view().locator('h1')).toHaveText('Launch review');
  await view().locator('[data-mt="rename"]').click();
  await title().fill('x'.repeat(201));
  await win.keyboard.press('Enter');
  await expect(title()).toBeVisible();
  await expect(win.locator('#toast')).toHaveText('Use a title of 1 to 200 characters.');
  await title().fill('Thursday launch');
  await win.keyboard.press('Enter');
  await expect(view().locator('h1')).toHaveText('Thursday launch');
  await expect(win.locator('.sidebar [data-mt-open]', { hasText: 'Thursday launch' })).toBeVisible();
  await view().locator('[data-mt="more"]').click();
  await expect(menu().getByText('Delete meeting')).toHaveCount(0);
  await win.keyboard.press('Escape');
  voice.append('meeting.state', id, { state: 'done', at: 5000, endedAt: Date.now() });
  voice.append('status', null, { recording: null, dictating: false });
  await expect(view().locator('[data-mt="stop"]')).toHaveCount(0);
  await closeApp(app);
  await launch();
  await expect.poll(connected).toBe(true);
  await openMeetings();
  await expect(view().locator('h1')).toHaveText('Thursday launch');
  const row = win.locator('.sidebar [data-mt-open]', { hasText: 'Thursday launch' });
  await row.click({ button: 'right' });
  await menu().getByText('Rename meeting', { exact: true }).click();
  await expect(title()).toBeFocused();
  await win.keyboard.press('Escape');
  await view().locator('[data-mt="more"]').click();
  await menu().getByText('Delete meeting', { exact: true }).click();
  await expect(win.locator('.dialog')).toContainText('Thursday launch');
  await expect(win.locator('.dialog')).toContainText('audio');
  await shot('meeting-delete-confirmation');
  await win.locator('.dialog .buttons button').first().click();
  await expect(view().locator('h1')).toHaveText('Thursday launch');
  await view().locator('[data-mt="more"]').click();
  await menu().getByText('Delete meeting', { exact: true }).click();
  await win.locator('.dialog .buttons .danger').click();
  await expect.poll(() => voice.commands.filter((c) => c.command === 'meeting.delete').length).toBe(1);
  await expect(win.locator('.sidebar [data-mt-open]')).toHaveCount(0);
  const retained = await app.evaluate(() => global.__semAgents.list().filter((c) => c.meeting));
  expect(retained).toHaveLength(1);
  expect(retained[0].meeting).toEqual({ id, title: 'Thursday launch', deleted: true });
  await win.keyboard.press('Control+j');
  await expect(pane().locator('.bui-entity__sub')).toHaveText('Meeting deleted');
  await shot('deleted-meeting-chat');
  await win.keyboard.press('Control+j');
  await closeApp(app);
  await launch();
  await expect.poll(connected).toBe(true);
  await openMeetings();
  await expect(win.locator('.sidebar [data-mt-open]')).toHaveCount(0);
});

test('screenshots: meeting rename, deletion and live screen menu in both languages and themes', async () => {
  const id = voiceStarts({ title: 'Thursday launch', screen: 'off' });
  say(id, 'call', 1000, 'The launch checklist is ready.');
  await openMeetings();
  for (const language of ['en', 'nl']) {
    await setLanguage(language);
    for (const theme of ['dark', 'light']) {
      await setTheme(theme);
      await view().locator('[data-mt="rename"]').click();
      await view().locator('.mt-title-input').fill(language === 'en' ? 'Thursday launch planning' : 'Planning lancering donderdag');
      await shot(`rename-${theme}-${language}`);
      await win.keyboard.press('Escape');
      await view().locator('[data-mt="screen"]').click();
      await expect(menu().locator('button').first()).toHaveAttribute('aria-checked', 'true');
      await shot(`live-screen-off-${theme}-${language}`);
      await win.keyboard.press('Escape');
    }
  }
  await setLanguage('en');
  await setSize(980, 760);
  for (const zoom of [0.8, 2]) {
    await app.evaluate(({ BrowserWindow }, zoom) => BrowserWindow.getAllWindows()[0].webContents.setZoomFactor(zoom), zoom);
    await view().locator('[data-mt="rename"]').click();
    await expect(view().locator('.mt-title-input')).toBeFocused();
    const bounds = await view().locator('.mt-title-input').evaluate((el) => ({ right: el.getBoundingClientRect().right, width: innerWidth }));
    expect(bounds.right).toBeLessThanOrEqual(bounds.width);
    // Electron's DevTools page screenshot can crop at non-default zoom; capture the whole window.
    await win.waitForTimeout(400);
    const bytes = await app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].capturePage()).toPNG().toString('base64'));
    fs.writeFileSync(path.join(SHOTS, `rename-narrow-zoom-${zoom}.png`), Buffer.from(bytes, 'base64'));
    await win.keyboard.press('Escape');
    await view().locator('[data-mt="screen"]').click();
    await expect(menu().locator('button').first()).toHaveAttribute('aria-checked', 'true');
    await win.keyboard.press('Escape');
  }
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].webContents.setZoomFactor(1));
  await setSize(1500, 950);
  voice.append('meeting.state', id, { state: 'done', at: 5000, endedAt: Date.now() });
  voice.append('status', null, { recording: null, dictating: false });
  await expect(view().locator('[data-mt="stop"]')).toHaveCount(0);
  for (const language of ['en', 'nl']) {
    await setLanguage(language);
    for (const theme of ['dark', 'light']) {
      await setTheme(theme);
      await view().locator('[data-mt="more"]').click();
      await menu().getByText(language === 'en' ? 'Delete meeting' : 'Meeting verwijderen', { exact: true }).click();
      await shot(`delete-${theme}-${language}`);
      await win.keyboard.press('Escape');
    }
  }
});

test('screen modes can be enabled from an audio-only meeting, turned off and resumed while paused', async () => {
  const id = voiceStarts({ title: 'Weekly planning', screen: 'off' });
  await openMeetings();
  const state = () => app.evaluate(() => global.__semVoice.screenRecording());
  await expect.poll(state).toBe(false);
  for (const [mode, label] of [['frames', 'Frames'], ['off', 'Off'], ['video', 'Video']]) {
    await view().locator('[data-mt="screen"]').click();
    await menu().getByText(label, { exact: true }).click();
    await expect.poll(() => voice.commands.filter((c) => c.command === 'screen.set').at(-1)?.args.screen).toBe(mode);
    await expect.poll(state).toBe(mode !== 'off');
    if (mode === 'off') {
      await view().locator('[data-mt="pause"]').click();
      await expect(view().locator('[data-mt="resume"]')).toBeVisible();
    }
  }
  await expect(view().locator('.mt-mark .mt-line')).toHaveText(['Screen recording on · Frames', 'Screen recording off', 'Screen recording on · Video']);
  const saved = await win.evaluate((id) => window.mail.call('meetingGet', id), id);
  expect(saved.meeting.initialScreen).toBe('off');
  expect(saved.meeting.screen).toBe('video');
});

for (const [language, theme, zoom] of [['en', 'dark', 0.8], ['en', 'light', 1], ['nl', 'dark', 1], ['nl', 'light', 2]]) {
  test(`screen controls follow Voice after monitor changes and can turn Off after disconnection: ${language} ${theme}`, async () => {
    const first = { id: 'demo-first', name: 'Demo display 1', width: 1920, height: 1080, primary: true };
    const second = { id: 'demo-second', name: 'Demo display 2', width: 1920, height: 1080, primary: false };
    let info = { screen: 'frames', state: 'recording', video: false, frames: true, monitor: first.id, monitors: [first, second] };
    voice.handlers['screen.get'] = () => info;
    const changeMode = voice.handlers['screen.set'];
    voice.handlers['screen.set'] = function (args) {
      if (args.monitor && !info.monitors.some((m) => m.id === args.monitor)) return { error: { code: 'unknown_monitor', message: 'That monitor was disconnected.' } };
      const result = args.screen === info.screen ? { screen: args.screen } : changeMode.call(this, args);
      info = { ...info, screen: args.screen, state: args.screen === 'off' ? 'off' : 'recording', frames: args.screen !== 'off', video: args.screen === 'video', monitor: args.monitor || info.monitor };
      return result;
    };
    const id = voiceStarts({ title: 'Monitor disconnect demo', screen: 'frames' });
    say(id, 'call', 1000, 'The presentation can move to the remaining display.');
    await openMeetings();
    await setLanguage(language);
    await setTheme(theme);
    await setSize(1060, 860);
    await app.evaluate(({ BrowserWindow }, zoom) => BrowserWindow.getAllWindows()[0].webContents.setZoomFactor(zoom), zoom);
    const openScreen = () => view().locator('[data-mt="screen"]').click();
    const monitorRow = (m) => menu().locator('button', { hasText: m.name });
    const lastChange = () => voice.commands.filter((c) => c.command === 'screen.set').at(-1)?.args;
    const recording = () => app.evaluate(() => global.__semVoice.screenRecording());
    const capture = async (suffix) => {
      fs.mkdirSync(SHOTS, { recursive: true });
      await win.waitForTimeout(400);
      const bytes = await app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].capturePage()).toPNG().toString('base64'));
      fs.writeFileSync(path.join(SHOTS, `screen-monitor-${suffix}-${theme}-${language}.png`), Buffer.from(bytes, 'base64'));
    };

    await openScreen();
    await monitorRow(second).click();
    await expect.poll(lastChange).toEqual({ meeting: id, screen: 'frames', monitor: second.id });

    // Voice's pill selects another display while the old cached selection is still connected.
    info = { ...info, monitor: first.id };
    await openScreen();
    await expect(monitorRow(first)).toHaveAttribute('aria-checked', 'true');
    await expect(monitorRow(second)).toHaveAttribute('aria-checked', 'false');
    await menu().getByText('Video', { exact: true }).click();
    await expect.poll(lastChange).toEqual({ meeting: id, screen: 'video', monitor: first.id });
    await expect.poll(() => info.screen).toBe('video');

    // The cached display disappears; mode changes must use Voice's remaining display.
    info = { ...info, monitors: [first] };
    await openScreen();
    await expect(monitorRow(first)).toHaveAttribute('aria-checked', 'true');
    await expect(monitorRow(second)).toHaveCount(0);
    await capture('fallback');
    await menu().getByText(language === 'nl' ? 'Beelden' : 'Frames', { exact: true }).click();
    await expect.poll(lastChange).toEqual({ meeting: id, screen: 'frames', monitor: first.id });
    await expect.poll(() => info.screen).toBe('frames');

    // Unplug the active display after opening the menu. Off must not depend on its old ID.
    await openScreen();
    info = { ...info, monitor: second.id, monitors: [second] };
    await menu().getByText(language === 'nl' ? 'Uit' : 'Off', { exact: true }).click();
    await expect.poll(lastChange).toEqual({ meeting: id, screen: 'off' });
    await expect.poll(recording).toBe(false);
    await openScreen();
    await expect(menu().getByRole('menuitemcheckbox', { name: language === 'nl' ? 'Uit' : 'Off', exact: true })).toHaveAttribute('aria-checked', 'true');
    await expect(monitorRow(second)).toHaveAttribute('aria-checked', 'true');
    await capture('off');
    await win.keyboard.press('Escape');
    info = { ...info, monitor: null, monitors: [first, second] };
    await openScreen();
    await expect(monitorRow(first)).toHaveAttribute('aria-checked', 'false');
    await expect(monitorRow(second)).toHaveAttribute('aria-checked', 'false');
  });
}

test('the Screen menu fills in when a new line redraws the header while Voice is still answering', async () => {
  const display = { id: 'demo-second', name: 'Demo display 2', width: 1920, height: 1080, primary: false };
  const id = voiceStarts({ title: 'Redraw demo', screen: 'frames' });
  await openMeetings();
  // Voice holds its answer to screen.get until the header has been drawn again.
  voice.silent.add('screen.get');
  const asks = () => voice.commands.filter((c) => c.command === 'screen.get');
  await view().locator('[data-mt="screen"]').evaluate((b) => (b.dataset.before = '1'));
  await view().locator('[data-mt="screen"]').click();
  await expect.poll(() => asks().length).toBe(1);
  say(id, 'call', 1000, 'A new line redraws the header.');
  await expect(rows()).toHaveCount(1);
  await expect(view().locator('[data-mt="screen"][data-before]')).toHaveCount(0);
  voice.silent.delete('screen.get');
  const reply = { type: 'reply', id: asks()[0].id, ok: true, result: { screen: 'frames', state: 'recording', frames: true, video: false, monitor: display.id, monitors: [display] } };
  for (const p of voice.peers) if (p.hello) p.send(reply);
  await expect(menu().locator('button', { hasText: display.name })).toHaveAttribute('aria-checked', 'true');
  await shot('screen-menu-after-redraw');
});

test('older Voice hides live screen controls and an unsupported deletion keeps the meeting and notes', async () => {
  const id = voiceStarts({ title: 'Retained notes', screen: 'off' });
  await openMeetings();
  await view().locator('.mt-notes-input').fill('Keep these decisions.');
  await expect.poll(async () => (await win.evaluate((id) => window.mail.call('meetingGet', id), id)).notes.text).toBe('Keep these decisions.');
  voice.protocol = 1;
  voice.writeDiscovery();
  voice.drop();
  await expect.poll(() => app.evaluate(() => global.__semVoice.state().protocol)).toBe(1);
  await expect(view().locator('[data-mt="screen"]')).toHaveCount(0);
  voice.append('meeting.state', id, { state: 'done', at: 5000, endedAt: Date.now() });
  voice.append('status', null, { recording: null, dictating: false });
  voice.handlers['meeting.delete'] = () => ({ error: { code: 'unknown_command', message: 'Unknown command.' } });
  await expect(view().locator('[data-mt="stop"]')).toHaveCount(0);
  await view().locator('[data-mt="more"]').click();
  await menu().getByText('Delete meeting', { exact: true }).click();
  await win.locator('.dialog .buttons .danger').click();
  await expect(win.locator('#toast')).toHaveText('Update Rukoo Voice to delete meetings.');
  await expect(view().locator('h1')).toHaveText('Retained notes');
  expect((await win.evaluate((id) => window.mail.call('meetingGet', id), id)).notes.text).toBe('Keep these decisions.');
});

test('similar screen frames fold across speech, open as a strip, and the Screen toggle persists after restart', async () => {
  const id = voiceStarts({ title: 'Launch review', screen: 'frames' });
  say(id, 'call', 1000, 'We can ship on Thursday.');
  frameAt(id, 2000, 'Google Meet\nLaunch review\nSanne Joris Bart');
  say(id, 'mic', 4000, 'I will tell support.');
  frameAt(id, 5000, 'Google Meet\nLaunch review\nSanne Joris Bart');
  frameAt(id, 8000, 'Google Meet\nLaunch review\nSanne Joris Bart');
  frameAt(id, 10000, 'Release checklist\nDocumentation\nSupport training');
  await openMeetings();
  await expect(win.locator('.mt-transcript > .mt-frame')).toHaveCount(2);
  await expect(frame(2000).locator('.mt-frame-count')).toHaveText('+2');
  // A later frame updates the original group's count without adding another transcript row.
  frameAt(id, 11000, 'Release checklist\nDocumentation\nSupport training');
  await expect(frame(10000).locator('.mt-frame-count')).toHaveText('+1');
  await frame(2000).locator('.mt-frame-count').focus();
  await win.keyboard.press('Enter');
  await expect(frame(2000).locator('.mt-frame-strip')).toBeVisible();
  await expect(frame(5000)).toHaveClass(/has-image/);
  await expect(frame(8000)).toHaveClass(/has-image/);
  frameAt(id, 9000, 'Google Meet\nLaunch review\nSanne Joris Bart');
  await expect(frame(2000).locator('.mt-frame-count')).toHaveText('+3');
  await expect(frame(2000).locator('.mt-frame-count')).toBeFocused();
  await shot('folded-frames-strip');
  await view().locator('[data-mt="show-frames"]').click();
  await expect(view().locator('.mt-frame')).toHaveCount(0);
  await expect(rows()).toHaveCount(2);
  expect(await app.evaluate((_e, id) => global.__semVoice.get(id).frames.length, id)).toBe(6);
  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Launch review');
  await expect(view().locator('[data-mt="show-frames"]')).toHaveAttribute('aria-pressed', 'false');
  await expect(view().locator('.mt-frame')).toHaveCount(0);
  await view().locator('[data-mt="show-frames"]').click();
  await expect(win.locator('.mt-transcript > .mt-frame')).toHaveCount(2);
});

test('notes share composer formatting, save Markdown and survive a restart, with full width notes and layout captures', async () => {
  test.setTimeout(120000);
  // The mail editor sets Chromium's document-wide CSS formatting flag; notes must reset it.
  await win.locator('[data-action="compose"]').click();
  await win.locator('.composer [data-c="close"]').click();
  const id = voiceStarts({ title: 'Launch review', screen: 'frames' });
  say(id, 'call', 1000, 'The release moves to Thursday.');
  say(id, 'mic', 5000, 'Support needs the checklist by Monday.');
  frameAt(id, 2500, 'Launch review\nRelease checklist\nSupport on Monday');
  frameAt(id, 5500, 'Launch review\nRelease checklist\nSupport on Monday');
  await openMeetings();
  const notes = () => view().locator('.mt-notes-input');
  await notes().fill('Launch decisions');
  await notes().press('Control+a');
  await view().locator('[data-cmd="formatBlock"]').click();
  await expect(notes().locator('h2')).toHaveText('Launch decisions');
  await notes().press('End');
  await notes().press('Enter');
  await notes().press('Control+b');
  await win.keyboard.type('Thursday');
  await notes().press('Control+b');
  await notes().press('Enter');
  await view().locator('[data-cmd="insertUnorderedList"]').click();
  await win.keyboard.type('Tell support on Monday');
  await notes().press('Enter');
  await win.keyboard.type('Publish the checklist');
  await notes().press('Control+k');
  await win.locator('.dialog input').fill('https://example.com/checklist');
  await win.locator('.dialog .buttons .primary').click();
  await notes().blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  const stored = await app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, id);
  expect(stored).toContain('## Launch decisions');
  expect(stored).toContain('**Thursday**');
  expect(stored).toContain('- Tell support on Monday');
  expect(stored).toContain('https://example.com/checklist');
  const result = await app.evaluate((_e, id) => global.__semAgents.callTool({ agent: 'claude', remote: false }, 'read_meeting_notes', { meeting_id: id }), id);
  expect(result.isError).toBeFalsy();
  expect(JSON.stringify(result.structuredContent)).toContain('## Launch decisions');
  // Compare a wide side editor with the full width Notes tab using the same populated meeting.
  for (const language of ['en', 'nl']) for (const theme of ['dark', 'light']) {
    await setLanguage(language);
    await setTheme(theme);
    await view().locator('[data-mt-tab="transcript"]').click();
    await shot(`notes-side-${theme}-${language}`);
    await view().locator('[data-mt-tab="notes"]').click();
    await expect(view().locator('.mt-transcript-pane')).toBeHidden();
    await shot(`notes-tab-${theme}-${language}`);
  }
  await setSize(860, 900);
  for (const language of ['en', 'nl']) for (const theme of ['dark', 'light']) {
    await setLanguage(language);
    await setTheme(theme);
    await shot(`notes-narrow-${theme}-${language}`);
    await view().locator('[data-mt-tab="transcript"]').click();
    await shot(`folded-frames-narrow-${theme}-${language}`);
    await view().locator('[data-mt-tab="notes"]').click();
  }
  await app.evaluate(({ webContents }) => webContents.getAllWebContents().find((w) => w.getType() === 'window').setZoomFactor(2));
  await win.waitForTimeout(400);
  // Electron's native capture includes the full window at non-default zoom; CDP can crop it to CSS pixels.
  fs.writeFileSync(path.join(SHOTS, 'notes-narrow-zoom-200.png'), Buffer.from(await app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].capturePage()).toPNG().toString('base64')), 'base64'));
  await expect(notes()).toBeInViewport();
  expect(await notes().evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await app.evaluate(({ webContents }) => webContents.getAllWebContents().find((w) => w.getType() === 'window').setZoomFactor(0.8));
  await setSize(1500, 950);
  await win.waitForTimeout(400);
  fs.writeFileSync(path.join(SHOTS, 'notes-wide-zoom-80.png'), Buffer.from(await app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].capturePage()).toPNG().toString('base64')), 'base64'));
  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Launch review');
  await expect(notes().locator('h2')).toHaveText('Launch decisions');
  await expect(notes().locator('b, strong')).toHaveText('Thursday');
  await expect(notes().locator('li')).toHaveCount(2);
  await expect(notes().locator('a')).toHaveAttribute('href', 'https://example.com/checklist');
  expect(await app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, id)).toBe(stored);
});

test('notes Markdown round trips editor structures and sanitizes pasted controls and links', async () => {
  const result = await win.evaluate(async () => {
    const { notesToMarkdown, markdownToNotes } = await import('./notes-markdown.js');
    const source = '<h2>Decisions</h2><div><b>Ship</b> on <i>Thursday</i>.</div><ul><li>Support<ul><li>Training</li></ul></li><li><a href="https://example.com/plan?q=a&amp;b=c">Plan</a></li></ul><div><u>Underlined</u> and *literal* [brackets] &lt;angle&gt;</div><pre><code>x &lt; y</code></pre>';
    const editor = document.createElement('div');
    editor.innerHTML = source;
    const markdown = notesToMarkdown(editor);
    editor.innerHTML = markdownToNotes(markdown);
    const again = notesToMarkdown(editor);
    const nested = editor.querySelector('li ul li')?.textContent;
    editor.innerHTML = '<div onclick="alert(1)">Safe<script>alert(1)</script><a href="javascript:alert(1)">label</a><img src="https://example.com/track"><button data-mt="stop">Stop</button></div>';
    const clean = markdownToNotes(notesToMarkdown(editor));
    const variants = [
      '<div><b>Bold <i>and italic</i></b></div>',
      '<div><code>a ` b</code></div><pre><code>```example```</code></pre>',
      '<div>Literal `ticks`</div>',
      '<ol><li>First<br>More on the first item</li><li>Second</li></ol>',
      '<div>1. Literal numbering</div><div># Literal heading</div><div></div><div>Last line</div>',
      '<div><a href="https://example.com"><code>a*b</code> and literal * [brackets]</a></div>',
      '<div><u><code>value</code> and literal * [brackets]</u></div>',
      '<div><b>First<br>Second</b></div>',
      '<div><i>First<br>Second</i></div>',
      '<div><s>First<br>Second</s></div>',
      '<div><u>First<br>Second</u></div>',
      '<div><a href="https://example.com">First<br>Second</a></div>',
      '<div><b><u>First<br><br>Second</u></b></div>',
      '<div><code>First\nSecond</code></div>',
      '<h2>First<br>Second</h2>',
      '<ul><li>First<br><br>Second</li><li>Third</li></ul>',
      '<ol><li>First<br><br>Second</li><li>Third</li></ol>',
      '<ul><li>Parent<ul><li>Nested<br><br>Continuation</li></ul>Back at parent</li><li>Sibling</li></ul>'
    ].map((html) => {
      editor.innerHTML = html;
      const first = notesToMarkdown(editor);
      editor.innerHTML = markdownToNotes(first);
      return [first, notesToMarkdown(editor)];
    });
    const blocks = ['First<div>Second</div>', 'First<p>Second</p>', '<div>First</div><div>Second</div>', 'First<div><br></div><div>Third</div>'].map((html) => {
      editor.innerHTML = html;
      return notesToMarkdown(editor);
    });
    const labels = ['[a\\*b](https://example.com)', '[a\\*b](<https://example.com>)', '<u>a\\*b</u>'].map((text) => {
      editor.innerHTML = markdownToNotes(text);
      return editor.textContent;
    });
    return { markdown, again, nested, clean, variants, blocks, labels };
  });
  expect(result.again).toBe(result.markdown);
  expect(result.nested).toBe('Training');
  expect(result.clean).toBe('<div>Safelabel</div>');
  for (const [first, second] of result.variants) expect(second).toBe(first);
  expect(result.blocks).toEqual(['First\nSecond', 'First\nSecond', 'First\nSecond', 'First\n\nThird']);
  expect(result.labels).toEqual(['a*b', 'a*b', 'a*b']);
});

test('blank list continuations and long escaped notes survive editing and restart without loss', async () => {
  test.setTimeout(120000);
  const id = voiceStarts({ title: 'Notes at their limits' });
  await openMeetings();
  const notes = () => view().locator('.mt-notes-input');
  await notes().evaluate((el) => {
    el.innerHTML = '<ol><li>First<br><br>Second</li><li>Third</li></ol>';
    el.dispatchEvent(new InputEvent('input', { bubbles: true }));
  });
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  await setLanguage('nl');
  await expect(notes().locator('ol')).toHaveCount(1);
  await expect(notes().locator('li')).toHaveCount(2);
  expect(await notes().locator('li').first().innerText()).toBe('First\n\nSecond');
  await shot('notes-blank-list-preserved');
  // Valid persisted Markdown can expand beyond the mail sanitizer's 200 KB HTML budget.
  const original = '&'.repeat(50000) + '\nEnd marker';
  await app.evaluate((_e, { id, original }) => {
    const v = global.__semVoice;
    v.saveNotes(id, original, v.get(id).notes.rev);
  }, { id, original });
  await expect.poll(() => notes().evaluate((el) => el.textContent.length)).toBe(50010);
  expect(await notes().evaluate((el) => el.textContent.endsWith('End marker'))).toBe(true);
  await notes().press('Control+End');
  await win.keyboard.type(' revised');
  await notes().blur();
  const expected = original + ' revised';
  await expect.poll(() => app.evaluate((_e, { id, expected }) => global.__semVoice.get(id).notes.text === expected, { id, expected })).toBe(true);
  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Notes at their limits');
  expect(await notes().evaluate((el) => el.textContent.length)).toBe(50018);
  expect(await notes().evaluate((el) => el.textContent.endsWith('End marker revised'))).toBe(true);
});

test('notes sanitize complete valid content while mail keeps its default HTML budget', async () => {
  const result = await win.evaluate(async () => {
    const { notesToMarkdown, markdownToNotes } = await import('./notes-markdown.js');
    const { sanitizeAgentHtml } = await import('./agent/richtext.js');
    const editor = document.createElement('div');
    const cases = ['&'.repeat(50000) + '\nEnd marker', '"'.repeat(99990) + '\nLast line'];
    const complete = cases.map((text) => {
      editor.innerHTML = markdownToNotes(text);
      return notesToMarkdown(editor) === text;
    });
    const html = '<div>' + '&amp;'.repeat(50000) + '<script>alert(1)</script><a href="javascript:alert(1)">End marker</a></div>';
    editor.innerHTML = html;
    const serialized = notesToMarkdown(editor);
    return { complete, serializedLength: serialized.length, tail: serialized.slice(-10), defaultTail: sanitizeAgentHtml(html).includes('End marker'), unsafe: serialized.includes('alert') };
  });
  expect(result.complete).toEqual([true, true]);
  expect(result.serializedLength).toBe(50010);
  expect(result.tail).toBe('End marker');
  expect(result.defaultTail).toBe(false);
  expect(result.unsafe).toBe(false);
});

test('escaped notes over the storage limit keep their saved original and a recoverable editor draft', async () => {
  test.setTimeout(120000);
  const id = voiceStarts({ title: 'Oversized notes' });
  const other = voice.meetingId();
  voice.append('meeting.started', other, { title: 'Another meeting', startedAt: Date.now() - 3600000, tracks: ['mic', 'call'], screen: 'off', startedBy: 'voice', context: null });
  voice.append('meeting.state', other, { state: 'done', at: 1000, endedAt: Date.now() - 3599000 });
  await openMeetings();
  const notes = () => view().locator('.mt-notes-input');
  const original = '_'.repeat(60000) + '\nEnd marker';
  await app.evaluate((_e, { id, original }) => global.__semVoice.saveNotes(id, original, 0), { id, original });
  await expect.poll(() => notes().evaluate((el) => el.textContent.length)).toBe(60010);
  await notes().press('Control+End');
  await win.keyboard.type(' revised');
  await notes().blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Not saved: shorten the notes');
  expect(await app.evaluate((_e, { id, original }) => global.__semVoice.get(id).notes.text === original && global.__semVoice.get(id).notes.rev === 1, { id, original })).toBe(true);
  // A permanently rejected draft must not prevent another meeting's notes from saving.
  await win.locator('.sidebar [data-mt-open]', { hasText: 'Another meeting' }).click();
  await notes().fill('These notes can save.');
  await notes().blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  await win.locator('.sidebar [data-mt-open]', { hasText: 'Oversized notes' }).click();
  expect(await notes().evaluate((el) => el.textContent.endsWith('End marker revised'))).toBe(true);
  await expect(view().locator('.mt-notes-status')).toHaveText('Not saved: shorten the notes');
  for (const language of ['en', 'nl']) for (const theme of ['dark', 'light']) {
    await setLanguage(language);
    await setTheme(theme);
    await view().locator('[data-mt-tab="notes"]').click();
    await setSize(860, 900);
    await shot(`notes-too-long-${theme}-${language}`);
  }
  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Oversized notes');
  expect(await notes().evaluate((el) => el.textContent.endsWith('End marker'))).toBe(true);
  expect(await app.evaluate((_e, { id, original }) => global.__semVoice.get(id).notes.text === original, { id, original })).toBe(true);
  await notes().fill('Short enough.');
  await notes().blur();
  await expect.poll(() => app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, id)).toBe('Short enough.');
});

test('soft breaks preserve note formatting and linked labels after restart', async () => {
  const id = voiceStarts({ title: 'Soft breaks in notes' });
  await openMeetings();
  const notes = () => view().locator('.mt-notes-input');
  await notes().fill('First bold line');
  await notes().press('Control+a');
  await notes().press('Control+b');
  await notes().press('End');
  await notes().press('Shift+Enter');
  await win.keyboard.type('Second bold line');
  await notes().blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  const typed = await app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, id);
  expect(typed).toBe('**First bold line**\n**Second bold line**');
  await notes().evaluate((el) => {
    el.insertAdjacentHTML('beforeend', '<div><u>First underlined line<br>Second underlined line</u></div><div><a href="https://example.com">First linked line<br>Second linked line</a></div>');
    el.dispatchEvent(new InputEvent('input', { bubbles: true }));
  });
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Soft breaks in notes');
  await expect(notes().locator('b, strong')).toHaveText(['First bold line', 'Second bold line']);
  await expect(notes().locator('u')).toHaveText(['First underlined line', 'Second underlined line']);
  await expect(notes().locator('a')).toHaveText(['First linked line', 'Second linked line']);
  await expect(notes()).not.toContainText('**');
  await shot('notes-soft-breaks-reopened');
});

test('typed note lines and formatted link labels survive saving, view changes and restart', async () => {
  test.setTimeout(120000);
  const id = voiceStarts({ title: 'Notes preservation' });
  say(id, 'call', 1000, 'Please keep each decision on its own line.');
  await openMeetings();
  const notes = () => view().locator('.mt-notes-input');
  await notes().fill('First decision');
  await notes().press('End');
  await notes().press('Enter');
  await win.keyboard.type('Second decision');
  await notes().blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  await expect.poll(() => app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, id)).toBe('First decision\nSecond decision');
  await notes().evaluate((el) => {
    el.insertAdjacentHTML('beforeend', '<div><a href="https://example.com"><code>plan</code> * [review]</a></div><div><u>Keep * and [brackets]</u></div>');
    el.dispatchEvent(new InputEvent('input', { bubbles: true }));
  });
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  const stored = await app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, id);
  for (const language of ['en', 'nl']) for (const theme of ['dark', 'light']) {
    await setLanguage(language);
    await setTheme(theme);
    await expect(notes().locator('a')).toHaveText('plan * [review]');
    await expect(notes().locator('a code')).toHaveText('plan');
    await expect(notes().locator('u')).toHaveText('Keep * and [brackets]');
    await shot(`notes-preserved-${theme}-${language}`);
  }
  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Notes preservation');
  expect(await notes().innerText()).toContain('First decision\nSecond decision');
  await expect(notes().locator('a')).toHaveText('plan * [review]');
  await expect(notes().locator('u')).toHaveText('Keep * and [brackets]');
  expect(await app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, id)).toBe(stored);
});

test('records the Stand-up from its card: Rukoo hides first, the transcript streams in, and Stop ends it', async () => {
  // The card offers recording from 15 minutes before an event until it ends, so the window's clock is set to five
  // minutes before the Stand-up. Voice's clock follows it, so the meeting's own clock reads right.
  const standup = standupAt();
  const offset = standup - 5 * 60 * 1000 - Date.now();
  await win.clock.install({ time: new Date(standup - 5 * 60 * 1000) });
  voice.handlers['meeting.start'] = function (args) {
    const id = this.meetingId();
    this.append('meeting.started', id, { title: args.title || '', startedAt: Date.now() + offset, tracks: ['mic', 'call'], screen: 'off', startedBy: 'rukoo', context: args.context || null });
    this.append('meeting.state', id, { state: 'recording', at: 0 });
    this.append('status', null, { recording: id, dictating: false });
    return { meeting: id };
  };
  voice.handlers['meeting.stop'] = function (args) {
    if (this.status.recording !== args.meeting) return { error: { code: 'not_recording', message: 'Not recording.' } };
    this.append('meeting.state', args.meeting, { state: 'processing', at: 15000, endedAt: Date.now() + offset, reason: 'rukoo' });
    this.append('status', null, { recording: null, dictating: false });
    return {};
  };
  // Every window's protection, at the moment Rukoo sends meeting.start.
  await app.evaluate(({ BrowserWindow }) => {
    const v = global.__semVoice;
    const command = v.bridge.command.bind(v.bridge);
    global.__protectedAtStart = null;
    v.bridge.command = (name, args) => {
      if (name === 'meeting.start') global.__protectedAtStart = BrowserWindow.getAllWindows().map((w) => w.isContentProtected());
      return command(name, args);
    };
  });

  await win.keyboard.press('Control+2');
  await win.locator('[data-cal-view="day"]').click();
  await win.locator('[data-cal="today"]').click();
  await win.locator('.calendar [data-ev]', { hasText: 'Stand-up' }).first().click();
  const card = win.locator('.cal-pop');
  await expect(card.locator('h2')).toHaveText('Stand-up');
  await expect(card.locator('[data-pop="record"]')).toHaveText('Record with Rukoo Voice');
  await shot('card-record');
  await card.locator('[data-pop="record"]').click();

  // The meeting opens in its own mode, with the event linked and the guests to name the call after.
  await expect(win.locator('[data-mode="meeting"]')).toHaveAttribute('aria-selected', 'true');
  await expect(view().locator('.mt-title')).toHaveText('Stand-up');
  await expect(view().locator('.mt-state')).toHaveText('Recording');
  await expect(view().locator('.mt-event-title')).toHaveText('Stand-up');
  const start = voice.commands.find((c) => c.command === 'meeting.start');
  expect(start.args.title).toBe('Stand-up');
  expect(start.args.context.calendarEvent).toMatch(/standup/);
  if (CAN_HIDE) {
    expect(await app.evaluate(() => global.__protectedAtStart)).toEqual([true]);
    expect(await isProtected()).toEqual([true]);
    await expect(view().locator('.mt-hidden')).toBeVisible();
  }
  await expect(indicator()).toBeVisible();
  await expect(indicator().locator('.rec-time')).toHaveText(/^\d:\d\d$/);
  const id = start && voice.status.recording;

  // Passages arrive while Voice records, in time order, with the voice's name once per stretch.
  voice.append('speaker', id, { id: 'mic', track: 'mic', label: 'You' });
  voice.append('speaker', id, { id: 'call-1', track: 'call', label: 'Speaker 1' });
  say(id, 'call', 2000, 'Morning all. Quick round: what did everyone do yesterday?');
  await expect(rows()).toHaveCount(1);
  await expect(rows().first().locator('.mt-name')).toHaveText('Speaker 1');
  say(id, 'mic', 6000, 'I finished the pricing page and sent it to Joris.');
  say(id, 'call', 10000, 'Thanks. Joris, can you review it before lunch?');
  say(id, 'call', 13500, 'Sure, I will have comments by twelve.');
  await expect(rows()).toHaveCount(4);
  await expect(rows().nth(1).locator('.mt-name')).toHaveText('You');
  await expect(rows().nth(3)).toHaveClass(/cont/);
  // A later revision replaces the passage in place.
  say(id, 'call', 2000, 'Morning, all. Quick round: what did everyone do yesterday?', { revision: 2 });
  await expect(rows().first().locator('.mt-text')).toHaveText('Morning, all. Quick round: what did everyone do yesterday?');
  await expect(rows()).toHaveCount(4);
  await expect(view().locator('.mt-live')).toHaveText('Listening…');

  // The indicator shows in Mail too, and brings the meeting back.
  await win.keyboard.press('Control+1');
  await expect(view()).toBeHidden();
  await expect(indicator()).toBeVisible();
  await indicator().click();
  await expect(view().locator('.mt-title')).toHaveText('Stand-up');

  // Pause and Resume, then Stop in two steps.
  await view().locator('[data-mt="pause"]').click();
  await expect(view().locator('.mt-state')).toHaveText('Paused');
  await expect(indicator()).toHaveClass(/is-paused/);
  await view().locator('[data-mt="resume"]').click();
  await expect(view().locator('.mt-state')).toHaveText('Recording');
  await view().locator('[data-mt="stop"]').click();
  await expect(view().locator('[data-mt="stop"]')).toHaveText('Stop recording?');
  expect(voice.commands.some((c) => c.command === 'meeting.stop')).toBe(false);
  await view().locator('[data-mt="stop"]').click();
  await expect(view().locator('.mt-state')).toHaveText('Finishing the transcript');
  expect(voice.commands.find((c) => c.command === 'meeting.stop').args.meeting).toBe(id);
  await expect(indicator()).toBeHidden();
  if (CAN_HIDE) await expect.poll(isProtected).toEqual([false]);
  voice.append('meeting.state', id, { state: 'done', at: 16000, endedAt: Date.now() + offset });
  await expect(view().locator('.mt-state')).toHaveCount(0);
  await expect(view().locator('[data-mt="stop"]')).toHaveCount(0);
});

test('a meeting Rukoo Voice starts links the Stand-up, hides Rukoo, and the names picked from its guests survive a restart', async () => {
  test.setTimeout(120000);
  await openMeetings();
  await expect(view().locator('.mt-empty .t')).toHaveText('Record a meeting');
  const id = voiceStarts({ startedAt: standupAt() + 60 * 1000 });
  say(id, 'call', 1500, 'Shall we start? Joris is still on his way.');
  say(id, 'mic', 5200, 'Yes, let us go through the launch first.');

  // The view opens the new recording, and Rukoo links the one event that fits.
  await expect(view().locator('.mt-state')).toHaveText('Recording');
  await expect(view().locator('.mt-event-title')).toHaveText('Stand-up', { timeout: 10000 });
  await expect(view().locator('.mt-title')).toHaveText('Stand-up');
  if (CAN_HIDE) await expect.poll(isProtected).toEqual([true]);
  await expect(indicator()).toBeVisible();

  // Two guests can be the voice on the call, so Rukoo asks instead of guessing.
  await expect(person('call-1').locator('.mt-person-name')).toHaveText('Speaker 1');
  await expect(person('call-1').locator('.mt-person-sub')).toHaveText('Choose who this is');
  await person('call-1').click();
  const menu = win.locator('.menu');
  await expect(menu.locator('.menu-heading')).toHaveText('Guests of Stand-up');
  await expect(menu.locator('button')).toHaveText([/Sanne de Vries/, /Joris Bakker/, /Someone else/]);
  await menu.locator('button', { hasText: 'Sanne de Vries' }).click();
  await expect(person('call-1').locator('.mt-person-name')).toHaveText('Sanne de Vries');
  await expect(person('call-1').locator('.mt-person-sub')).toHaveText('sanne@example.com');
  await expect(rows().first().locator('.mt-name')).toHaveText('Sanne de Vries');
  await expect(rows().nth(1).locator('.mt-name')).toHaveText('You');

  // A correction and notes are Rukoo's own too.
  await rows().first().locator('.mt-text').dblclick();
  await view().locator('.mt-edit').fill('Shall we start? Joris is on his way.');
  await view().locator('.mt-edit').press('Enter');
  await expect(rows().first().locator('.mt-text')).toHaveText('Shall we start? Joris is on his way.');
  await expect(rows().first().locator('.mt-corrected')).toHaveText('Corrected by you');
  await view().locator('.mt-notes-input').fill('Launch first, then pricing.');
  await view().locator('.mt-notes-input').blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  // The keyboard path: the transcript is one tab stop, arrows move, Enter corrects and Escape leaves it as it was.
  await view().locator('.mt-transcript').focus();
  await win.keyboard.press('ArrowDown');
  await expect(rows().first()).toBeFocused();
  await win.keyboard.press('ArrowDown');
  await expect(rows().nth(1)).toBeFocused();
  await win.keyboard.press('Enter');
  await expect(view().locator('.mt-edit')).toBeFocused();
  await win.keyboard.type(' Typed, then dropped.');
  await win.keyboard.press('Escape');
  await expect(rows().nth(1).locator('.mt-text')).toHaveText('Yes, let us go through the launch first.');
  await expect(rows().nth(1)).toBeFocused();

  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Stand-up', { timeout: 20000 });
  await expect(person('call-1').locator('.mt-person-name')).toHaveText('Sanne de Vries');
  await expect(rows().first().locator('.mt-name')).toHaveText('Sanne de Vries');
  await expect(rows().first().locator('.mt-text')).toHaveText('Shall we start? Joris is on his way.');
  await expect(view().locator('.mt-notes-input')).toHaveText('Launch first, then pricing.');
  // Voice still records: after the restart Rukoo hides again and shows the clock.
  await expect.poll(connected, { timeout: 10000 }).toBe(true);
  if (CAN_HIDE) await expect.poll(isProtected).toEqual([true]);
  await expect(indicator()).toBeVisible();
});

test('three voices on the call keep their names', async () => {
  test.setTimeout(180000);
  await openMeetings();
  // With speaker labels Voice sends a call voice right before its first passage: at the start there is only you.
  const id = voiceStarts({ startedAt: standupAt() + 60 * 1000, speakers: [MIC] });
  await expect(view().locator('.mt-event-title')).toHaveText('Stand-up', { timeout: 10000 });
  voice.append('speaker', id, callVoice(1));
  say(id, 'call', 1500, 'Shall we start with the numbers?');
  say(id, 'mic', 4000, 'Yes, go ahead.');
  // Numbers follow the order Voice first heard each voice, but call-10's first passage can arrive before call-2's.
  voice.append('speaker', id, callVoice(10));
  say(id, 'call', 7000, 'Sorry, I joined late.', { speaker: 'call-10' });
  voice.append('speaker', id, callVoice(2));
  say(id, 'call', 10000, 'The numbers are in.', { speaker: 'call-2' });
  const names = () => view().locator('.mt-people .mt-person-name');
  await expect(names()).toHaveText(['You', 'Speaker 1', 'Speaker 2', 'Speaker 10']);
  // An unnamed voice's monogram is its number: Speaker 1 and Speaker 10 would both be "S1".
  await expect(view().locator('.mt-people button.mt-person .mt-av')).toHaveText(['1', '2', '10']);
  await expect(rows()).toHaveCount(4);
  await expect(person('call-1').locator('.mt-person-sub')).toHaveText('Choose who this is');

  // call-2 gets a name from the Stand-up's guests.
  const menu = win.locator('.menu');
  await person('call-2').click();
  await expect(menu.locator('button')).toHaveText([/Sanne de Vries/, /Joris Bakker/, /Someone else/]);
  await menu.locator('button', { hasText: 'Sanne de Vries' }).click();
  await expect(person('call-2').locator('.mt-person-name')).toHaveText('Sanne de Vries');
  await expect(rows().nth(3).locator('.mt-name')).toHaveText('Sanne de Vries');

  // Another voice's picker offers Sanne after Joris, marked, and she can still be picked.
  const sanne = menu.locator('button', { hasText: 'Sanne de Vries' });
  await person('call-10').click();
  await expect(menu.locator('button')).toHaveText([/Joris Bakker/, /Sanne de Vries/, /Someone else/]);
  await expect(sanne.locator('.ms')).toHaveText('Already chosen for Sanne de Vries');
  await expect(sanne).toBeEnabled();
  await expect(menu.locator('button', { hasText: 'Joris Bakker' }).locator('.ms')).toHaveText('joris@example.com');
  await win.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);

  // A passage Voice couldn't place yet shows Someone; its revision with the voice takes Sanne's name.
  say(id, 'call', 14000, "Okay, let's look at them.", { speaker: null });
  await expect(rows()).toHaveCount(5);
  await expect(rows().nth(4).locator('.mt-name')).toHaveText('Someone');
  say(id, 'call', 14000, "Okay, let's look at them.", { speaker: 'call-2', revision: 2 });
  await expect(rows().nth(4).locator('.mt-name')).toHaveText('Sanne de Vries');
  await expect(rows()).toHaveCount(5);

  // The People card and call-10's open picker, in both themes and languages, wide and narrow.
  for (const [width, height, size] of [[1500, 950, 'wide'], [760, 900, 'narrow']]) {
    await setSize(width, height);
    for (const language of ['en', 'nl']) {
      await setLanguage(language);
      for (const theme of ['dark', 'light']) {
        await setTheme(theme);
        if (size === 'narrow') await view().locator('[data-mt-tab="people"]').click();
        await expect(person('call-10')).toBeVisible();
        if (language === 'nl') await expect(names()).toHaveText(['Jij', 'Spreker 1', 'Sanne de Vries', 'Spreker 10']);
        await shot(`speakers-${size}-${theme}-${language}`);
        await person('call-10').click();
        await expect(sanne.locator('.ms')).toHaveText(language === 'nl' ? 'Al gekozen voor Sanne de Vries' : 'Already chosen for Sanne de Vries');
        await shot(`speakers-menu-${size}-${theme}-${language}`);
        await win.keyboard.press('Escape');
        await expect(menu).toHaveCount(0);
      }
    }
  }
  await setLanguage('en');

  // After a restart the names are still on their voices.
  await closeApp(app);
  await launch();
  await expect(view().locator('.mt-title')).toHaveText('Stand-up', { timeout: 20000 });
  await expect(names()).toHaveText(['You', 'Speaker 1', 'Sanne de Vries', 'Speaker 10']);
  await expect(rows()).toHaveCount(5);
  await expect(rows().nth(3).locator('.mt-name')).toHaveText('Sanne de Vries');
  await expect(rows().nth(4).locator('.mt-name')).toHaveText('Sanne de Vries');
  await expect(rows().nth(2).locator('.mt-name')).toHaveText('Speaker 10');
});

test("a 1:1's guessed name stays when a second voice joins, and the People card asks to check it", async () => {
  await openMeetings();
  // The Stand-up as if only Sanne were invited: one voice on the call and one guest, so Rukoo fills in her name.
  await app.evaluate(() => {
    const v = global.__semVoice;
    const event = v.calendarEvent.bind(v);
    v.calendarEvent = (eventId) => {
      const e = event(eventId);
      return e && /standup/.test(e.id) ? { ...e, attendees: (e.attendees || []).filter((a) => a.self || /Sanne/.test(a.name || '')) } : e;
    };
  });
  const id = voiceStarts({ startedAt: standupAt() + 60 * 1000 });
  say(id, 'call', 1500, 'Hi, just the two of us today.');
  await expect(view().locator('.mt-event-title')).toHaveText('Stand-up', { timeout: 10000 });
  await expect(person('call-1').locator('.mt-person-name')).toHaveText('Sanne de Vries');
  await expect(person('call-1').locator('.mt-person-sub')).toHaveText('sanne@example.com');
  // Someone else joins the call: the guess stays, and the card asks to check it.
  voice.append('speaker', id, callVoice(2));
  say(id, 'call', 6000, 'Sorry, Joris here, I joined after all.', { speaker: 'call-2' });
  await expect(person('call-1').locator('.mt-person-sub')).toHaveText('Guessed from the invitation; check it');
  await expect(person('call-1').locator('.mt-person-sub')).toHaveClass(/is-check/);
  await expect(person('call-2').locator('.mt-person-name')).toHaveText('Speaker 2');
  for (const theme of ['dark', 'light']) {
    await setTheme(theme);
    await shot(`speakers-autocheck-${theme}-en`);
  }
  await setLanguage('nl');
  await expect(person('call-1').locator('.mt-person-sub')).toHaveText('Geraden uit de uitnodiging; controleer het');
  // The longer Dutch sentence wraps rather than losing "controleer het" to an ellipsis.
  expect(await person('call-1').locator('.mt-person-sub').evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  await shot('speakers-autocheck-light-nl');
  await setLanguage('en');
  // Picking the name confirms it.
  await person('call-1').click();
  await win.locator('.menu button', { hasText: 'Sanne de Vries' }).click();
  await expect(person('call-1').locator('.mt-person-sub')).toHaveText('sanne@example.com');
});

test('the chat follows the meeting: the fake agent reads the transcript, and its second turn gets only what is new', async () => {
  // Leave an unrelated email selected before opening Meet, as in the reported context bug.
  await win.locator('.item').first().click();
  const id = voiceStarts({ title: 'Design review' });
  say(id, 'call', 1000, 'The new onboarding has three steps instead of five.');
  say(id, 'mic', 4500, 'Good. Can we keep the skip button?');
  say(id, 'call', 8000, 'Yes, it stays on every step.');
  await openMeetings();
  await expect(rows()).toHaveCount(3);
  await win.keyboard.press('Control+j');
  await expect(pane()).toBeVisible();
  await expect(win.locator('.agentpane .bui-rec__title')).toHaveText('What should Hermes do with this meeting?');
  await expect(win.locator('.agentpane .bui-entity__name')).toHaveText('Design review');
  await expect(win.locator('.agentpane .bui-sugg__chip')).toHaveText(['Catch me up', 'What did they ask me?', 'Write the notes', 'Draft the follow-up email']);
  await expect(input()).toHaveAttribute('data-placeholder', 'Ask Hermes about this meeting...');

  await chip('Catch me up').click();
  await expect(transcript().locator('.bui-ub')).toHaveText('Catch me up');
  const first = transcript().locator('[data-type="assistant"]').last();
  await expect(first).toContainText('From Design review, 3 lines', { timeout: 10000 });
  await expect(first).toContainText('The new onboarding has three steps instead of five.');
  await expect(transcript().locator('.bui-tool', { hasText: 'Read the meeting' })).toBeVisible();
  const chat = await app.evaluate(() => global.__semAgents.list()[0]);
  expect(chat.meeting).toEqual({ id, title: 'Design review' });
  const context = await app.evaluate((_e, cid) => global.__semAgents.callTool({ agent: 'clark', remote: false, conversationId: cid }, 'get_context', {}), chat.id);
  expect(context.structuredContent.meeting.id).toBe(id);
  expect(context.structuredContent.open_message).toBeNull();
  await shot('meeting-agent-context');

  say(id, 'mic', 12000, 'Then I will tell support on Monday.');
  say(id, 'call', 15500, 'Thanks, that is everything.');
  await expect(rows()).toHaveCount(5);
  await input().fill('What is new in the meeting?');
  await input().press('Enter');
  const second = transcript().locator('[data-type="assistant"]').last();
  await expect(second).toContainText('From Design review, 2 lines', { timeout: 10000 });
  await expect(second).toContainText('Then I will tell support on Monday.');
  await expect(second).not.toContainText('three steps instead of five');

  // In Mail the panel follows the email again; back in the meeting it finds this chat.
  await win.keyboard.press('Control+1');
  await expect(win.locator('.agentpane .bui-entity__name')).not.toHaveText('Design review');
  await win.keyboard.press('Control+3');
  await expect(transcript()).toContainText('From Design review, 2 lines');
});

test("an agent gets a link to the meeting's audio that stops working", async () => {
  const id = voiceStarts({ title: 'Design review', startedAt: Date.now() - 60 * 1000 });
  say(id, 'call', 1000, 'Can we hear that part again?');
  await openMeetings();
  await expect(rows()).toHaveCount(1);
  await win.keyboard.press('Control+j');
  await expect(pane()).toBeVisible();
  await input().fill('Get me a media link for the first seconds');
  await input().press('Enter');

  // The fake agent calls get_meeting_media_link for 0 to 5 s and answers with the link.
  await expect(transcript().locator('.bui-tool', { hasText: "Got a link to the meeting's audio or screen" })).toBeVisible({ timeout: 10000 });
  const answer = transcript().locator('[data-type="assistant"]').last();
  await expect(answer).toContainText('-audio-0-5.wav', { timeout: 10000 });
  const url = await answer.locator('a').first().getAttribute('href');
  expect(url.startsWith('http://127.0.0.1:')).toBe(true);
  expect(url).toMatch(/\/media\/v1\/[A-Za-z0-9_-]+\.[A-Za-z0-9_-]{43}\/m-[0-9a-f-]+-audio-0-5\.wav$/);
  expect(url.endsWith(`/${id}-audio-0-5.wav`)).toBe(true);
  expect(voice.commands.find((c) => c.command === 'media.open').args).toEqual({ meeting: id, kind: 'audio', start: 0, end: 5000 });

  // Fetched from here, as an agent on this computer would: the mix Voice prepared, and a range of it.
  const audio = syntheticMedia({ kind: 'audio', start: 0, end: 5000 });
  const wav = audio.bytes(0, audio.size);
  const res = await fetch(url);
  expect(res.status).toBe(200);
  expect(res.headers.get('content-type')).toBe('audio/wav');
  expect(Buffer.from(await res.arrayBuffer()).equals(wav)).toBe(true);
  const head = await fetch(url, { headers: { range: 'bytes=0-43' } });
  expect(head.status).toBe(206);
  expect(Buffer.from(await head.arrayBuffer()).equals(wav.subarray(0, 44))).toBe(true);
  await setTheme('dark');
  await shot('media-link-dark');
  await setTheme('light');
  await shot('media-link-light');
  await setLanguage('nl');
  await expect(transcript().locator('.bui-tool', { hasText: 'Haalde een link naar geluid of scherm' })).toBeVisible();
  await shot('media-link-light-nl');
  await setTheme('dark');
  await shot('media-link-dark-nl');

  // Four seconds after it was made, the same link is refused.
  await expect.poll(async () => (await fetch(url)).status, { timeout: 10000, intervals: [500] }).toBe(410);
});

test('a dropped connection and a Rukoo Voice on a new port replay without showing anything twice', async () => {
  const id = voiceStarts({ title: 'Sync with Joris' });
  say(id, 'call', 1000, 'Can you hear me?');
  say(id, 'mic', 3000, 'Yes, loud and clear.');
  await openMeetings();
  await expect(rows()).toHaveCount(2);

  // The connection dies halfway through a passage: it comes back and the replay brings it once.
  const cut = say(id, 'call', 6000, 'Let me share my screen.', { push: false });
  voice.drop(cut);
  await expect(rows()).toHaveCount(3, { timeout: 15000 });

  // Voice restarts on another port with the same data folder; what it said meanwhile arrives once too.
  const old = voice;
  const next = new FakeVoice({ file: old.file, instance: old.instance, token: old.token });
  Object.assign(next, { events: old.events, head: old.head, status: old.status, nextMeeting: old.nextMeeting });
  voiceHandlers(next);
  await old.close();
  voice = next;
  say(id, 'mic', 9000, 'I can see it now.');
  say(id, 'call', 12000, 'Good, the dashboard is on the left.');
  await voice.listen();
  voice.writeDiscovery();
  await expect(rows()).toHaveCount(5, { timeout: 15000 });
  const keys = await rows().evaluateAll((els) => els.map((el) => el.dataset.key));
  expect(new Set(keys).size).toBe(5);
  await expect(rows().last().locator('.mt-text')).toHaveText('Good, the dashboard is on the left.');
  // The new Voice was asked only for what came after the last event Rukoo had.
  const hello = voice.hellos[voice.hellos.length - 1];
  expect(hello.instance).toBe(old.instance);
  expect(hello.after).toBeGreaterThan(0);
});

test('a meeting Rukoo Voice deletes keeps your notes, marked as deleted; one without notes leaves the list', async () => {
  const ended = (id, title, ago) => {
    voice.append('meeting.started', id, { title, startedAt: Date.now() - ago, tracks: ['mic', 'call'], screen: 'off', startedBy: 'voice', context: null });
    voice.append('meeting.state', id, { state: 'done', at: 60000, endedAt: Date.now() - ago + 60000 });
    say(id, 'call', 1000, `A line from ${title}.`);
  };
  const kept = voice.meetingId();
  const gone = voice.meetingId();
  ended(kept, 'Kept for its notes', 3 * 60 * 60 * 1000);
  ended(gone, 'Gone entirely', 2 * 60 * 60 * 1000);
  await openMeetings();
  const item = (title) => win.locator('.sidebar [data-mt-open]', { hasText: title });
  await expect(win.locator('.sidebar [data-mt-open]')).toHaveCount(2);
  await item('Kept for its notes').click();
  await expect(item('Kept for its notes')).toHaveClass(/active/);
  await expect(view().locator('.mt-title')).toHaveText('Kept for its notes');
  await expect(rows()).toHaveCount(1);
  await view().locator('.mt-notes-input').fill('Follow up with the supplier.');
  await view().locator('.mt-notes-input').blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');

  voice.append('meeting.removed', kept, {});
  await expect(view().locator('.mt-state')).toHaveText('Deleted in Rukoo Voice');
  await expect(view().locator('.mt-notice')).toHaveText('Rukoo Voice deleted this meeting and its transcript. Your notes stay here.');
  await expect(rows()).toHaveCount(0);
  await expect(view().locator('.mt-notes-input')).toHaveText('Follow up with the supplier.');
  voice.append('meeting.removed', gone, {});
  await expect(item('Gone entirely')).toHaveCount(0);
  await expect(item('Kept for its notes')).toHaveCount(1);
});

test('notes typed while a save is still on its way are kept when another meeting opens', async () => {
  const ended = (id, title, ago) => {
    voice.append('meeting.started', id, { title, startedAt: Date.now() - ago, tracks: ['mic', 'call'], screen: 'off', startedBy: 'voice', context: null });
    voice.append('meeting.state', id, { state: 'done', at: 60000, endedAt: Date.now() - ago + 60000 });
    say(id, 'call', 1000, `A line from ${title}.`);
  };
  const first = voice.meetingId();
  const second = voice.meetingId();
  ended(first, 'First meeting', 3 * 60 * 60 * 1000);
  ended(second, 'Second meeting', 2 * 60 * 60 * 1000);
  await openMeetings();
  const item = (title) => win.locator('.sidebar [data-mt-open]', { hasText: title });
  await item('First meeting').click();
  await expect(view().locator('.mt-title')).toHaveText('First meeting');
  // Every save takes a while in main, so a second one has to wait for the first.
  await app.evaluate(() => {
    const v = global.__semVoice;
    const save = v.saveNotes.bind(v);
    v.saveNotes = (...args) => new Promise((resolve) => setTimeout(() => resolve(save(...args)), 800));
  });
  const input = view().locator('.mt-notes-input');
  await input.fill('First line.');
  await input.blur();
  await input.fill('First line. Second line.');
  await item('Second meeting').click();
  await expect(view().locator('.mt-title')).toHaveText('Second meeting');
  await expect.poll(() => app.evaluate((_e, id) => global.__semVoice.get(id).notes.text, first), { timeout: 10000 }).toBe('First line. Second line.');
  await item('First meeting').click();
  await expect(view().locator('.mt-notes-input')).toHaveText('First line. Second line.');
});

test('without Rukoo Voice the view says to start it, and Settings > Meetings has both switches', async () => {
  await voice.close();
  voice.removeDiscovery();
  await expect.poll(connected).toBe(false);
  await openMeetings();
  await expect(view().locator('.mt-empty .t')).toHaveText('Start Rukoo Voice to record meetings');
  await expect(win.locator('.sidebar .sync-status .t')).toHaveText('Voice not running');
  await win.locator('.sidebar [data-mt-side="record"]').click();
  await expect(win.locator('#toast')).toHaveText('Start Rukoo Voice to record meetings.');
  await shot('voice-absent');

  // The sidebar's status line leads to Settings > Meetings.
  await win.locator('.sidebar .sync-status').click();
  const page = win.locator('.page.settings');
  await expect(page.locator('h1')).toHaveText('Meetings');
  await expect(page.locator('.mtset-pill')).toHaveText('Not running');
  const hide = page.locator('[data-a="toggle:meetingHideWindow"]');
  const agents = page.locator('[data-a="toggle:meetingAgentsCanRead"]');
  if (CAN_HIDE) {
    await expect(hide.locator('.switch')).toHaveClass(/on/);
    await hide.click();
    await expect(page.locator('[data-a="toggle:meetingHideWindow"] .switch')).not.toHaveClass(/on/);
    expect(await win.evaluate(() => window.mail.call('state').then((s) => s.settings.meetingHideWindow))).toBe(false);
  } else {
    await expect(hide).toHaveCount(0);
  }
  await expect(agents.locator('.switch')).toHaveClass(/on/);
  await agents.click();
  await expect(page.locator('[data-a="toggle:meetingAgentsCanRead"] .switch')).not.toHaveClass(/on/);
  expect(await win.evaluate(() => window.mail.call('state').then((s) => s.settings.meetingAgentsCanRead))).toBe(false);
  await shot('settings-meetings-off');
});

// ---------- the screen (RUK-108) ----------

test('records with a screen choice from the Stand-up card, and the header says what Voice records of the screen', async () => {
  const standup = standupAt();
  await win.clock.install({ time: new Date(standup - 5 * 60 * 1000) });
  // Every window's protection, at the moment Rukoo sends meeting.start.
  await app.evaluate(({ BrowserWindow }) => {
    const v = global.__semVoice;
    const command = v.bridge.command.bind(v.bridge);
    global.__protectedAtStart = null;
    v.bridge.command = (name, args, opts) => {
      if (name === 'meeting.start') global.__protectedAtStart = BrowserWindow.getAllWindows().map((w) => w.isContentProtected());
      return command(name, args, opts);
    };
  });
  await win.keyboard.press('Control+2');
  await win.locator('[data-cal-view="day"]').click();
  await win.locator('[data-cal="today"]').click();
  await win.locator('.calendar [data-ev]', { hasText: 'Stand-up' }).first().click();
  const card = win.locator('.cal-pop');
  await expect(card.locator('[data-pop="record"]')).toHaveText('Record with Rukoo Voice');
  await card.locator('[data-pop="record-menu"]').click();
  // Voice didn't say what its own setting is, so the first choice names no mode.
  await expect(menu().locator('.menu-heading')).toHaveText('Choose what to record');
  await expect(menu().locator('button')).toHaveText(["Voice's default", 'Audio only', 'Audio and screen frames', 'Audio and screen video']);
  await expect(card).toBeVisible();
  await shot('card-record-menu');
  await menu().locator('button', { hasText: 'Audio and screen frames' }).click();
  await expect(card).toBeHidden();

  await expect(win.locator('[data-mode="meeting"]')).toHaveAttribute('aria-selected', 'true');
  await expect(view().locator('.mt-title')).toHaveText('Stand-up');
  const start = voice.commands.find((c) => c.command === 'meeting.start');
  expect(start.args.screen).toBe('frames');
  expect(start.args.context.calendarEvent).toMatch(/standup/);
  if (CAN_HIDE) expect(await app.evaluate(() => global.__protectedAtStart)).toEqual([true]);
  await expect(view().locator('.mt-screen')).toHaveText('Screen: frames');
  await expect(view().locator('[data-mt="screen"]')).toBeVisible();
  // Hidden from capture, Rukoo can't show in the frames.
  await expect(view().locator('.mt-visible')).toHaveCount(0);
});

test("Voice's default shows in the record menu, and Record itself leaves the choice to Voice", async () => {
  voice.handlers['meeting.start'] = function (args) {
    const id = this.meetingId();
    this.append('meeting.started', id, { title: args.title || '', startedAt: Date.now(), tracks: ['mic', 'call'], screen: args.screen || this.status.screenDefault || 'off', startedBy: 'rukoo', context: args.context || null });
    this.append('meeting.state', id, { state: 'recording', at: 0 });
    this.append('status', null, { recording: id, dictating: false, screenDefault: this.status.screenDefault });
    return { meeting: id };
  };
  voice.append('status', null, { recording: null, dictating: false, screenDefault: 'video' });
  await openMeetings();
  await expect(view().locator('.mt-empty .t')).toHaveText('Record a meeting');
  await view().locator('[data-mt="record-menu"]').click();
  await expect(menu().locator('button').first()).toHaveText("Voice's default (video)");
  await expect(menu().locator('button').first()).toHaveAttribute('aria-checked', 'true');
  await shot('empty-record-menu');
  await win.keyboard.press('Escape');
  await expect(menu()).toHaveCount(0);
  await expect(view().locator('[data-mt="record-menu"]')).toBeFocused();

  // The sidebar's arrow opens the same choice, under the whole button.
  await win.locator('.sidebar [data-mt-side="record-menu"]').click();
  await expect(menu().locator('button').first()).toHaveText("Voice's default (video)");
  const [split, box] = await Promise.all([win.locator('.sidebar .mt-record-split').boundingBox(), menu().boundingBox()]);
  expect(Math.abs(box.x - split.x)).toBeLessThan(2);
  await shot('sidebar-record-menu');
  await win.keyboard.press('Escape');

  await win.locator('.sidebar [data-mt-side="record"]').click();
  await expect.poll(() => voice.commands.filter((c) => c.command === 'meeting.start').length).toBe(1);
  expect(voice.commands.find((c) => c.command === 'meeting.start').args).not.toHaveProperty('screen');
  await expect(view().locator('.mt-screen')).toHaveText('Screen: video');
});

test('frames sit between the lines at their moments, load as they come into view, open to their text, and say why when Voice is gone', async () => {
  test.setTimeout(90000);
  voice.handlers['frame.get'] = ((own) =>
    function (args) {
      // Voice removed this frame's image with the meeting's audio: the frame keeps its text, without an image.
      if (args.at === 9000) return { frame: { id: 'f-000009000', at: 9000, width: 1920, height: 1080, text: 'Agenda' }, image: null };
      return own.call(this, args);
    })(voice.handlers['frame.get']);
  const id = voiceStarts({ title: 'Roadmap review', screen: 'frames' });
  for (let i = 0; i < 60; i++) say(id, i % 3 ? 'call' : 'mic', 2000 + i * 5000, `Line ${i + 1} of the roadmap review, about the launch and the pricing page.`);
  frameAt(id, 4000, 'Q3 roadmap\nLaunch in September\nPricing page first');
  frameAt(id, 9000, 'Agenda');
  frameAt(id, 152500, 'Support plan\nTell support on Monday');
  frameAt(id, 297500, 'Slide 4\nNext steps\nWrite the follow-up');
  await openMeetings();
  await expect(rows()).toHaveCount(60);

  // Each frame sits after what was said before it, and before the next line.
  const keys = await win.locator('.meeting .mt-transcript > li').evaluateAll((els) => els.map((el) => el.dataset.key));
  expect(keys.slice(0, 4)).toEqual([`p:${pid('mic', 2000)}`, `f:${frameId(4000)}`, `p:${pid('call', 7000)}`, `f:${frameId(9000)}`]);
  expect(keys.indexOf(`f:${frameId(297500)}`)).toBe(keys.indexOf(`p:${pid('call', 297000)}`) + 1);

  // The view opens at the end: only the frame there asks Voice for its image.
  await expect(frame(297500)).toHaveClass(/has-image/);
  await expect(frame(297500).locator('.mt-shot img')).toHaveAttribute('src', /^data:image\/png;base64,/);
  await expect(frame(297500).locator('.mt-shot img')).toHaveAttribute('alt', 'The screen at 4:57');
  expect(frameGets()).toEqual([297500]);

  // The first line shows; the rest opens with the toggle, and Enter on the row closes it again.
  await expect(frame(297500).locator('.mt-frame-line')).toHaveText('Slide 4');
  await expect(frame(297500).locator('.mt-frame-full')).toBeHidden();
  await frame(297500).locator('.mt-frame-toggle').click();
  await expect(frame(297500)).toHaveClass(/is-open/);
  await expect(frame(297500).locator('.mt-frame-full')).toBeVisible();
  await expect(frame(297500).locator('.mt-frame-full')).toHaveText('Next steps\nWrite the follow-up');
  // The row draws again as it opens: measure the image once it is laid out at its new width.
  await expect.poll(async () => (await frame(297500).locator('.mt-shot').boundingBox())?.width ?? 0).toBeGreaterThan(300);
  const wide = await frame(297500).locator('.mt-shot').boundingBox();
  expect(wide.width).toBeGreaterThan(300);
  // The row was the last one in view; the opened image scrolled into view instead of growing below it.
  const port = await view().locator('.mt-scroll').boundingBox();
  expect(wide.y + wide.height).toBeLessThanOrEqual(port.y + port.height + 1);
  await shot('frame-open');
  await frame(297500).focus();
  await win.keyboard.press('Enter');
  await expect(frame(297500)).not.toHaveClass(/is-open/);
  await expect(frame(297500).locator('.mt-frame-full')).toBeHidden();

  // A frame that arrives while the meeting records takes its place at once; without text it says so.
  frameAt(id, 299000, '');
  await expect(frame(299000)).toHaveCount(1);
  await expect(frame(299000).locator('.mt-frame-line')).toHaveText('No text read from the screen');
  await expect(frame(299000)).toHaveClass(/has-image/);

  // At the top: the first frame's image arrives, and the one Voice removed says so.
  await view().locator('.mt-scroll').evaluate((el) => (el.scrollTop = 0));
  await expect(frame(4000)).toHaveClass(/has-image/);
  await expect(frame(9000).locator('.mt-frame-note')).toHaveText('Rukoo Voice no longer has this image.');
  await expect(frame(9000).locator('.mt-frame-line')).toHaveText('Agenda');
  expect(frameGets()).not.toContain(152500);
  await shot('frame-gone');

  // Without Voice, images Rukoo has stay, the others say why they're missing, and the text is still there.
  await voice.close();
  voice.removeDiscovery();
  await expect.poll(connected).toBe(false);
  await frame(152500).scrollIntoViewIfNeeded();
  await expect(frame(152500).locator('.mt-frame-note')).toHaveText('The image shows while Rukoo Voice runs.');
  await expect(frame(152500).locator('.mt-frame-line')).toHaveText('Support plan');
  await expect(frame(152500)).not.toHaveClass(/has-image/);
  await expect(frame(4000)).toHaveClass(/has-image/);
  await shot('frames-offline');
});

test("the view keeps at most the cached frames' images, and a frame scrolled back to loads its image again", async () => {
  test.setTimeout(120000);
  const id = voiceStarts({ title: 'Long deck', screen: 'frames' });
  const ats = Array.from({ length: 75 }, (_, i) => 1000 + i * 4000);
  for (const [i, at] of ats.entries()) frameAt(id, at, `Slide ${i + 1}`);
  await openMeetings();
  const last = ats[ats.length - 1];
  await expect(frame(last)).toHaveClass(/has-image/);
  // Up to the top a step at a time, so each frame comes into view and loads.
  const scroll = view().locator('.mt-scroll');
  for (;;) {
    const top = await scroll.evaluate((el) => (el.scrollTop = Math.max(0, el.scrollTop - el.clientHeight / 2)));
    await expect
      .poll(() => win.locator('.meeting .mt-frame').evaluateAll((els, h) => els.filter((el) => { const b = el.getBoundingClientRect(); return b.bottom > 0 && b.top < h && !el.classList.contains('has-image'); }).length, 950))
      .toBe(0);
    if (top === 0) break;
  }
  await expect(frame(ats[0])).toHaveClass(/has-image/);
  // Every frame was loaded once, but the rows keep at most the cache's images and the ones near the view. Which
  // rows let go depends on the order the first images loaded in, so the test asks the page.
  expect(new Set(frameGets()).size).toBe(ats.length);
  const holding = await win.locator('.meeting .mt-frame.has-image').count();
  expect(holding).toBeLessThan(ats.length);
  expect(holding).toBeLessThanOrEqual(70);
  const released = await win.locator('.meeting .mt-frame:not(.has-image)').evaluateAll((els) => els.map((el) => Number(el.dataset.at)));
  expect(released.length).toBeGreaterThan(0);
  const back = released[released.length - 1];
  expect(await frame(back).locator('.mt-shot img').getAttribute('src')).toBeNull();
  // Scrolled back to, a released frame asks Voice again and shows its image.
  await frame(back).scrollIntoViewIfNeeded();
  await expect(frame(back)).toHaveClass(/has-image/);
  // Asked again at least once; a row near the edge of the view can leave the cache and come back more than once.
  expect(frameGets().filter((at) => at === back).length).toBeGreaterThanOrEqual(2);
});

test('marks about the screen say what happened to it, apart from the audio', async () => {
  const id = voiceStarts({ title: 'Screen marks', screen: 'video' });
  say(id, 'call', 500, 'Let me share my screen.');
  const mark = (data) => voice.append('mark', id, { until: null, track: 'screen', label: null, ...data });
  mark({ id: 'dropped-video-a', kind: 'video_dropped', at: 900, detail: 'encoder' });
  mark({ id: 'dropped-video-b', kind: 'video_dropped', at: 60000, detail: 'disk' });
  mark({ id: 'gap-screen-000070000', kind: 'screen_gap', at: 70000, until: 130000, detail: 'locked' });
  mark({ id: 'monitor-000140000', kind: 'monitor', at: 140000, detail: 'user', label: 'DELL S3220DGF' });
  mark({ id: 'gap-screen-000180000', kind: 'screen_gap', at: 180000, until: 183000, detail: 'crash' });
  mark({ id: 'gap-screen-000200000', kind: 'screen_gap', at: 200000, until: 204000, detail: 'device' });
  mark({ id: 'monitor-000210000', kind: 'monitor', at: 210000, detail: 'lost', label: 'PHL 275V8' });
  mark({ id: 'gap-screen-000220000', kind: 'screen_gap', at: 220000, until: 225000, detail: 'monitor' });
  mark({ id: 'dropped-frames-a', kind: 'dropped', at: 240000, detail: 'disk' });
  mark({ id: 'dropped-video-c', kind: 'video_dropped', at: 245000, detail: 'device' });
  mark({ id: 'dropped-frames-b', kind: 'dropped', at: 250000, detail: 'device' });
  mark({ id: 'dropped-frames-c', kind: 'dropped', at: 260000, detail: 'crash' });
  await openMeetings();
  await expect(view().locator('.mt-mark .mt-line')).toHaveText([
    'Video stopped: no video encoder on this computer worked. Frames go on.',
    'Video stopped to keep recording audio: the disk is almost full. Frames go on.',
    /^Screen: not recorded for 1 min\w* while the computer was locked$/,
    'Screen recording moved to DELL S3220DGF',
    /^Screen: not recorded for 3 sec\w* while the screen recorder restarted$/,
    /^Screen: not recorded for 4 sec\w* while the graphics card restarted$/,
    'The recorded monitor went away; screen recording moved to PHL 275V8',
    /^Screen: not recorded for 5 sec\w* while no monitor was connected$/,
    'Screen recording stopped to keep recording audio: the disk is full',
    "Video stopped: the graphics card couldn't start the video again",
    "Screen recording stopped to keep recording audio: the graphics card couldn't start again",
    'Screen recording stopped: the screen recorder failed too often'
  ]);
  await expect(view().locator('.mt-mark.is-monitor svg')).toHaveCount(2);
  await expect(view().locator('.mt-mark.is-screen_gap')).toHaveCount(4);
  await expect(view().locator('.mt-mark.is-video_dropped')).toHaveCount(3);
  await shot('screen-marks-light-en');
  await setLanguage('nl');
  await setTheme('dark');
  await expect(view().locator('.mt-mark .mt-line').first()).toHaveText('Video gestopt: geen video-encoder op deze computer werkte. Schermbeelden gaan door.');
  await expect(view().locator('.mt-mark.is-video_dropped .mt-line').nth(2)).toHaveText('Video gestopt: de videokaart kon de video niet opnieuw starten');
  await expect(view().locator('.mt-mark.is-screen_gap .mt-line').last()).toHaveText(/^Scherm: 5 sec\S* niet opgenomen terwijl er geen beeldscherm was aangesloten$/);
  await expect(view().locator('.mt-mark .mt-line').last()).toHaveText('Schermopname gestopt: de schermopname viel te vaak uit');
  await shot('screen-marks-dark-nl');
});

test('a silent call mark says where to switch System Audio Recording back on', async () => {
  const id = voiceStarts({ title: 'Silent call' });
  say(id, 'mic', 500, 'Can you hear me?');
  voice.append('mark', id, { id: 'silent-call-30', kind: 'silent', at: 30, until: null, track: 'call', detail: null, label: null });
  await openMeetings();
  await expect(view().locator('.mt-mark.is-silent .mt-line')).toHaveText(
    'Call audio: no sound from here while another app had its audio output open. If people were talking, macOS may not have let Rukoo Voice record it: check that Rukoo Voice is switched on under System Settings › Privacy & Security › Screen & System Audio Recording › System Audio Recording Only.'
  );
  await expect(view().locator('.mt-mark.is-silent svg')).toHaveCount(1);
  await shot('silent-call-light-en');
  await setLanguage('nl');
  await setTheme('dark');
  await expect(view().locator('.mt-mark.is-silent .mt-line')).toHaveText(/^Gespreksaudio: vanaf hier geen geluid/);
  await shot('silent-call-dark-nl');
});

test('frames and speaker labels in one meeting: each frame sits at its moment, and each voice keeps its row and its lines', async () => {
  // With speaker labels Voice sends a call voice right before its first passage; frames come between the passages.
  const id = voiceStarts({ title: 'Design review', screen: 'frames', speakers: [MIC] });
  await openMeetings();
  voice.append('speaker', id, callVoice(1));
  say(id, 'call', 1500, 'Here is the first design.');
  frameAt(id, 3000, 'Design A\nNavigation on the left');
  voice.append('speaker', id, callVoice(2));
  say(id, 'call', 5000, 'I prefer the second one.', { speaker: 'call-2' });
  say(id, 'mic', 8000, "Let's look at it.");
  frameAt(id, 9500, 'Design B\nNavigation on top');
  voice.append('speaker', id, callVoice(3));
  say(id, 'call', 11000, 'Same here.', { speaker: 'call-3' });
  await expect(view().locator('.mt-screen')).toHaveText('Screen: frames');
  await expect(view().locator('.mt-people .mt-person-name')).toHaveText(['You', 'Speaker 1', 'Speaker 2', 'Speaker 3']);
  await expect(rows()).toHaveCount(4);
  await expect(rows().locator('.mt-name')).toHaveText(['Speaker 1', 'Speaker 2', 'You', 'Speaker 3']);
  const keys = await win.locator('.meeting .mt-transcript > li').evaluateAll((els) => els.map((el) => el.dataset.key));
  expect(keys).toEqual([`p:${pid('call', 1500)}`, `f:${frameId(3000)}`, `p:${pid('call', 5000)}`, `p:${pid('mic', 8000)}`, `f:${frameId(9500)}`, `p:${pid('call', 11000)}`]);
  await expect(frame(9500).locator('.mt-frame-line')).toHaveText('Design B');
  // A passage Voice places later moves to its voice; the frames stay where they are.
  say(id, 'call', 13000, 'One more thing.', { speaker: null });
  frameAt(id, 14500, 'Design C');
  await expect(rows().nth(4).locator('.mt-name')).toHaveText('Someone');
  say(id, 'call', 13000, 'One more thing.', { speaker: 'call-2', revision: 2 });
  await expect(rows().nth(4).locator('.mt-name')).toHaveText('Speaker 2');
  await expect(frame(14500)).toHaveCount(1);
  const after = await win.locator('.meeting .mt-transcript > li').evaluateAll((els) => els.map((el) => el.dataset.key));
  expect(after.slice(-2)).toEqual([`p:${pid('call', 13000)}`, `f:${frameId(14500)}`]);
});

test('a meeting Rukoo Voice starts gets Rukoo hidden before Voice captures the screen; a replayed start asks nothing', async () => {
  const acks = [];
  voice.handlers['meeting.protected'] = function (args) {
    acks.push({ at: Date.now(), meeting: args.meeting });
    return {};
  };
  // When main first hides a window.
  await app.evaluate(({ BrowserWindow }) => {
    global.__protectedAt = null;
    for (const w of BrowserWindow.getAllWindows()) {
      const set = w.setContentProtection.bind(w);
      w.setContentProtection = (on) => {
        if (on && !global.__protectedAt) global.__protectedAt = Date.now();
        return set(on);
      };
    }
  });
  // Voice records the audio already and asks before its first frame. Its status hasn't said it records yet.
  const id = voice.meetingId();
  voice.append('meeting.started', id, { title: 'From the pill', startedAt: Date.now(), tracks: ['mic', 'call'], screen: 'frames', startedBy: 'voice', context: null });
  voice.append('meeting.state', id, { state: 'recording', at: 0 });
  voice.append('meeting.starting', id, { screen: 'frames' });
  await expect.poll(() => acks.length).toBe(1);
  expect(acks[0].meeting).toBe(id);
  if (CAN_HIDE) {
    expect(await isProtected()).toEqual([true]);
    const at = await app.evaluate(() => global.__protectedAt);
    expect(acks[0].at - at).toBeGreaterThanOrEqual(90);
  }

  // A start Rukoo missed comes back in the replay after a reconnect: that is history, so no second answer.
  const other = voice.meetingId();
  const missed = voice.append('meeting.starting', other, { screen: 'frames' }, { push: false });
  voice.drop(missed);
  await expect.poll(connected, { timeout: 15000 }).toBe(false);
  await expect.poll(connected, { timeout: 15000 }).toBe(true);
  await win.waitForTimeout(1500);
  expect(acks.length).toBe(1);
  expect(voice.commands.filter((c) => c.command === 'meeting.protected').length).toBe(1);
});

test("a window that doesn't take the protection: Rukoo confirms nothing to Voice, says it shows, and hides it once it can", async () => {
  test.skip(!CAN_HIDE, 'Electron hides windows from capture only on Windows and macOS');
  voice.handlers['meeting.protected'] = () => ({});
  // Every window refuses: setContentProtection does nothing, as when Windows doesn't take it.
  await app.evaluate(({ BrowserWindow }) => {
    global.__refuse = true;
    for (const w of BrowserWindow.getAllWindows()) {
      const set = w.setContentProtection.bind(w);
      w.setContentProtection = (on) => (global.__refuse ? undefined : set(on));
    }
  });
  const id = voice.meetingId();
  voice.append('meeting.started', id, { title: 'From the pill', startedAt: Date.now(), tracks: ['mic', 'call'], screen: 'frames', startedBy: 'voice', context: null });
  voice.append('meeting.state', id, { state: 'recording', at: 0 });
  voice.append('meeting.starting', id, { screen: 'frames' });
  await openMeetings();
  await expect(view().locator('.mt-visible')).toHaveText('Visible in screen recordings');
  // Voice stops waiting after 500 ms.
  await win.waitForTimeout(1000);
  expect(voice.commands.filter((c) => c.command === 'meeting.protected')).toEqual([]);
  expect(await isProtected()).toEqual([false]);
  expect((await win.evaluate(() => window.mail.call('voiceState'))).hidden).toBe(false);

  // The windows take it again: Rukoo's next try hides them, and the header says so.
  await app.evaluate(() => {
    global.__refuse = false;
  });
  await expect.poll(isProtected, { timeout: 5000 }).toEqual([true]);
  await expect(view().locator('.mt-hidden')).toBeVisible();
  await expect(view().locator('.mt-visible')).toHaveCount(0);
  expect((await win.evaluate(() => window.mail.call('voiceState'))).hidden).toBe(true);
  expect(voice.commands.filter((c) => c.command === 'meeting.protected')).toEqual([]);
});

test('the Screen button moves the recording to another monitor; a Voice that can\'t do that loses the button', async () => {
  const DISPLAY1 = '\\\\.\\DISPLAY1';
  const DISPLAY2 = '\\\\.\\DISPLAY2';
  const monitors = [
    { id: DISPLAY1, name: 'DELL S3220DGF', width: 2560, height: 1440, primary: true },
    { id: DISPLAY2, name: 'PHL 275V8', width: 1440, height: 2560, primary: false }
  ];
  let screen = { screen: 'frames', state: 'recording', video: false, frames: true, monitor: DISPLAY1, monitors };
  voice.handlers['screen.get'] = () => screen;
  voice.handlers['screen.set'] = (args) => ({ screen: args.screen });
  const id = voiceStarts({ title: 'Demo for the board', screen: 'frames' });
  say(id, 'call', 1000, 'Can you show the second screen?');
  await openMeetings();
  await view().locator('[data-mt="screen"]').click();
  await expect(menu().locator('.menu-heading')).toHaveText(['Screen recording', 'Record this monitor']);
  await expect(menu().locator('button')).toHaveText(['Off', 'Frames', 'Video', /DELL S3220DGF\s*2560 × 1440\s*Main display/, /PHL 275V8\s*1440 × 2560/]);
  await expect(menu().locator('button', { hasText: 'DELL S3220DGF' })).toHaveAttribute('aria-checked', 'true');
  await expect(menu().locator('button', { hasText: 'PHL 275V8' })).toHaveAttribute('aria-checked', 'false');
  await shot('screen-menu');
  await menu().locator('button', { hasText: 'PHL 275V8' }).click();
  await expect.poll(() => voice.commands.filter((c) => c.command === 'screen.set').map((c) => c.args)).toEqual([{ meeting: id, screen: 'frames', monitor: DISPLAY2 }]);
  voice.append('mark', id, { id: 'monitor-000015000', kind: 'monitor', at: 15000, until: null, track: 'screen', detail: 'user', label: 'PHL 275V8' });
  await expect(view().locator('.mt-mark .mt-line')).toHaveText(['Screen recording moved to PHL 275V8']);

  // Voice stopped recording the screen: the menu says so and offers nothing.
  screen = { ...screen, state: 'stopped', frames: false, monitor: null };
  await view().locator('[data-mt="screen"]').click();
  await expect(menu().locator('button', { hasText: 'Screen recording stopped' })).toBeDisabled();
  await expect(menu().locator('button', { hasText: 'PHL 275V8' })).toBeDisabled();
  await win.keyboard.press('Escape');

  // A Voice from before screen recording doesn't know screen.get: the button goes for that Voice.
  voice.handlers['screen.get'] = () => ({ error: { code: 'unknown_command', message: "Voice doesn't know the command `screen.get`." } });
  await view().locator('[data-mt="screen"]').click();
  await expect(win.locator('#toast')).toHaveText("This Rukoo Voice can't do that yet. Update Rukoo Voice.");
  await expect(view().locator('[data-mt="screen"]')).toHaveCount(0);
  await expect(view().locator('[data-mt="mark"]')).toBeVisible();
});

test('mail notifications wait while a meeting records the screen; the badge still counts', async () => {
  // Nothing reaches the desktop in a test, whatever happens.
  await app.evaluate(({ Notification }) => {
    global.__semShown = [];
    Notification.prototype.show = function () {
      global.__semShown.push(this.title);
    };
  });
  voiceStarts({ title: 'Screen share', screen: 'frames' });
  await expect.poll(() => app.evaluate(() => global.__semVoice.screenRecording())).toBe(true);
  const before = await app.evaluate(() => global.__semHeldNotices || 0);
  await app.evaluate(() => global.__semEngine.emit('new-mail', [{ id: 'demo:held-1', subject: 'Invoice for October', from: { name: 'Demo', address: 'demo@example.com' } }]));
  await expect.poll(() => app.evaluate(() => global.__semHeldNotices || 0)).toBe(before + 1);
  expect(await app.evaluate(() => global.__semShown)).toEqual([]);
});

test('screenshots: screen frames, the record and screen menus and the chips, in both themes and languages, wide and narrow', async () => {
  test.setTimeout(180000);
  voice.handlers['screen.get'] = () => ({
    screen: 'video',
    state: 'recording',
    video: true,
    frames: true,
    monitor: '\\\\.\\DISPLAY1',
    monitors: [
      { id: '\\\\.\\DISPLAY1', name: 'DELL S3220DGF', width: 2560, height: 1440, primary: true },
      { id: '\\\\.\\DISPLAY2', name: 'PHL 275V8', width: 1440, height: 2560, primary: false }
    ]
  });
  const id = voiceStarts({ title: 'Roadmap review', startedAt: Date.now() - 6 * 60 * 1000, screen: 'video' });
  say(id, 'call', 2100, 'Let me share the roadmap. Can everyone see it?');
  say(id, 'mic', 6400, 'Yes, the Q3 slide is up.');
  frameAt(id, 8000, 'Q3 roadmap\nLaunch in September\nPricing page first\nSupport plan in October');
  say(id, 'call', 12500, 'September is the launch. The pricing page goes first.');
  say(id, 'mic', 19000, 'Can we move support to October?');
  voice.append('mark', id, { id: 'monitor-000024000', kind: 'monitor', at: 24000, until: null, track: 'screen', detail: 'user', label: 'PHL 275V8' });
  frameAt(id, 25500, 'Support plan\nTell support on Monday\nTraining in week 41', { width: 1440, height: 2560 });
  say(id, 'call', 28000, 'Yes. I will tell support on Monday.');
  voice.append('mark', id, { id: 'dropped-video', kind: 'video_dropped', at: 33000, until: null, track: 'screen', detail: 'disk', label: null });
  frameAt(id, 36000, 'Next steps\nWrite the follow-up');
  say(id, 'mic', 38000, 'Agreed. I will write the follow-up.');
  await openMeetings();
  await expect(rows()).toHaveCount(6);
  for (const at of [8000, 25500, 36000]) await expect(frame(at)).toHaveClass(/has-image/);
  await frame(8000).locator('.mt-frame-toggle').click();

  for (const language of ['en', 'nl']) {
    await setLanguage(language);
    await frame(8000).locator('.mt-frame-toggle').waitFor();
    if (!(await frame(8000).getAttribute('class')).includes('is-open')) await frame(8000).locator('.mt-frame-toggle').click();
    for (const theme of ['dark', 'light']) {
      await setTheme(theme);
      await shot(`frames-wide-${theme}-${language}`);
      await view().locator('[data-mt="screen"]').click();
      await expect(menu().locator('button')).toHaveCount(5);
      await shot(`screen-menu-${theme}-${language}`);
      await win.keyboard.press('Escape');
    }
  }
  await expect(view().locator('.mt-screen')).toHaveText('Scherm: beelden');

  // Narrow, with Rukoo visible in screen recordings: the chip warns.
  await win.evaluate(() => window.mail.call('updateSettings', { meetingHideWindow: false }));
  if (CAN_HIDE) await expect(view().locator('.mt-visible')).toHaveText('Zichtbaar in schermopnames');
  await setSize(700, 900);
  for (const language of ['nl', 'en']) {
    await setLanguage(language);
    for (const theme of ['dark', 'light']) {
      await setTheme(theme);
      await shot(`frames-narrow-${theme}-${language}`);
    }
  }
  if (CAN_HIDE) await expect(view().locator('.mt-visible')).toHaveText('Visible in screen recordings');

  // The record menus, with nothing recording.
  voice.append('meeting.state', id, { state: 'done', at: 40000, endedAt: Date.now() });
  voice.append('status', null, { recording: null, dictating: false, screenDefault: 'frames' });
  await setSize(1500, 950);
  for (const [theme, language] of [
    ['dark', 'en'],
    ['light', 'nl']
  ]) {
    await setLanguage(language);
    await setTheme(theme);
    await win.locator('.sidebar [data-mt-side="record-menu"]').click();
    await expect(menu().locator('button')).toHaveCount(4);
    await shot(`record-menu-${theme}-${language}`);
    await win.keyboard.press('Escape');
  }
});

test('screenshots: a recording meeting in dark and light, English and Dutch, wide and narrow', async () => {
  test.setTimeout(150000);
  const id = voiceStarts({ title: 'Weekly planning', startedAt: Date.now() - 4 * 60 * 1000 });
  const lines = [
    ['call', 2100, 'Morning. Shall we start with the launch date?'],
    ['mic', 6200, 'Yes. I think Thursday still works if the copy is done by Tuesday.'],
    ['call', 11800, "The copy is close. I'll send the last screens tonight."],
    ['call', 15400, 'Can you check the pricing page before then?'],
    ['mic', 21000, 'I will. Let me note that.'],
    ['mic', 26000, 'Send Joris the pricing numbers', { dictation: true }],
    ['call', 41000, 'Then we move the launch to Thursday and tell support on Monday.'],
    ['mic', 47000, 'Agreed. I will write the follow-up.']
  ];
  for (const [track, start, text, more] of lines) say(id, track, start, text, more || {});
  voice.append('mark', id, { id: 'dict-26000', kind: 'dictation', at: 26000, until: 29500, track: 'mic', detail: null, label: null });
  voice.append('mark', id, { id: 'gap-mic-33000', kind: 'gap', at: 33000, until: 39000, track: 'mic', detail: 'device', label: null });
  voice.append('mark', id, { id: 'dev-mic-39000', kind: 'device', at: 39000, until: null, track: 'mic', detail: 'reconnected', label: 'Jabra Evolve2 65' });
  voice.append('mark', id, { id: 'k-1', kind: 'user', at: 44000, until: null, track: null, detail: null, label: null });
  await openMeetings();
  await expect(rows()).toHaveCount(8);
  // The dictation stretch shows as its tagged line; the gap and the device change as marks.
  await expect(rows().nth(5)).toHaveClass(/is-dict/);
  await expect(rows().nth(5).locator('.mt-tag')).toHaveText('Dictation');
  await expect(view().locator('.mt-mark')).toHaveText([/Microphone: no audio for .+, the device was gone/, /Microphone: Jabra Evolve2 65 is back/, /Marked/]);
  await view().locator('.mt-notes-input').fill('Launch moves to Thursday. Support hears on Monday.');
  await view().locator('.mt-notes-input').blur();
  await expect(view().locator('.mt-notes-status')).toHaveText('Saved');
  await win.keyboard.press('Control+j');
  await expect(pane()).toBeVisible();
  await settled();

  for (const language of ['en', 'nl']) {
    await setLanguage(language);
    for (const theme of ['dark', 'light']) {
      await setTheme(theme);
      await shot(`wide-${theme}-${language}`);
    }
  }
  await expect(view().locator('[data-mt="mark"]')).toHaveText('Markeren');

  // Narrow: the chat closed, the parts as tabs; then the chat over the meeting.
  await win.locator('.agentpane [data-ap="close"]').click();
  await expect(pane()).toBeHidden();
  await setSize(760, 900);
  await expect(view().locator('.mt-tabs')).toBeVisible();
  for (const theme of ['dark', 'light']) {
    await setTheme(theme);
    await view().locator('[data-mt-tab="transcript"]').click();
    await shot(`narrow-${theme}-nl-transcript`);
    await view().locator('[data-mt-tab="notes"]').click();
    await expect(view().locator('.mt-notes-input')).toBeVisible();
    await expect(view().locator('.mt-transcript-pane')).toBeHidden();
    await shot(`narrow-${theme}-nl-notes`);
    await view().locator('[data-mt-tab="people"]').click();
    await expect(person('call-1')).toBeVisible();
    await shot(`narrow-${theme}-nl-details`);
  }
  await setLanguage('en');
  await view().locator('[data-mt-tab="transcript"]').click();
  await shot('narrow-light-en-transcript');
  await win.keyboard.press('Control+j');
  await expect(pane()).toBeVisible();
  await settled();
  await shot('narrow-light-en-chat');
});
