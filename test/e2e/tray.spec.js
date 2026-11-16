// End-to-end tests for the tray: closing and minimizing to the tray, the tray menu, quitting, and both settings off.
// Test mode (SEM_HIDDEN) puts no icon in the notification area: the tests click main's stand-in, global.__semTray,
// which has the real menu. The real icon is checked by hand.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;
let running;

// On a Mac the close button always hides the main window, closing the last window never quits, the two settings
// are about the menu bar and session-end (a Windows event) never comes. macos.spec.js covers the Mac behavior, so
// the cases below that check the Windows and Linux behavior skip there.
const MAC = process.platform === 'darwin';
const NOT_ON_MAC = 'Windows and Linux behavior; macos.spec.js covers the Mac';

async function launch() {
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.'], cwd: path.join(__dirname, '..', '..'), env });
  running = true;
  app.process().once('exit', () => (running = false));
  win = page(await app.firstWindow());
  await app.evaluate(({ BrowserWindow }) => {
    const w = BrowserWindow.getAllWindows()[0];
    w.setSize(1500, 950);
    // isVisible() is false for a minimized window too; only a hidden one raises 'hide'.
    global.__hidden = false;
    w.on('hide', () => (global.__hidden = true));
    w.on('show', () => (global.__hidden = false));
  });
}

// The question about unsaved changes comes from beforeunload, which Electron answers by itself (it keeps the
// window). Without a listener of its own, Playwright tries to answer it too, and fails.
function page(p) {
  p.on('dialog', () => {});
  return p;
}

test.beforeEach(async () => {
  dataDir = tempDir('sem-e2e-');
  fs.mkdirSync(path.join(dataDir, 'data'), { recursive: true });
  fs.writeFileSync(path.join(dataDir, 'data', 'settings.json'), JSON.stringify({ closeToTray: true }));
  await launch();
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
});

test.afterEach(async () => {
  if (running) await closeApp(app);
  removeDir(dataDir);
});

// The main window as main sees it, or null once it is gone. hidden: in the tray, not on the taskbar.
const main = () =>
  app.evaluate(({ BrowserWindow }) => {
    const w = BrowserWindow.getAllWindows().find((x) => x.webContents.getURL().endsWith('index.html'));
    return w ? { visible: w.isVisible(), minimized: w.isMinimized(), hidden: global.__hidden, bounds: w.getBounds() } : null;
  });
const visible = async () => (await main())?.visible;
const windowCount = () => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length);
// What the close button does.
const closeMain = () =>
  app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find((x) => x.webContents.getURL().endsWith('index.html')).close());

const tray = () =>
  app.evaluate(() => {
    const t = global.__semTray;
    return t && { tooltip: t.tray.tooltip, dot: t.dot, labels: t.menu.items.filter((i) => i.type !== 'separator').map((i) => i.label) };
  });
const trayClick = () => app.evaluate(() => global.__semTray.tray.emit('click'));
const trayMenu = (id) => app.evaluate((_electron, id) => global.__semTray.menu.getMenuItemById(id).click(), id);

const saved = () => JSON.parse(fs.readFileSync(path.join(dataDir, 'data', 'settings.json'), 'utf8'));
const unread = () => app.evaluate(() => global.__semEngine.unreadCount());

// Notifications are recorded instead of shown, so the tests put no toasts on screen.
const recordNotifications = () =>
  app.evaluate(({ Notification, BrowserWindow }) => {
    // Simulate a background app explicitly: Linux window managers can focus an off-screen window.
    BrowserWindow.getAllWindows()[0].isFocused = () => false;
    global.__shown = [];
    Notification.prototype.show = function () {
      global.__shown.push(this);
    };
  });
const shown = () => app.evaluate(() => global.__shown.map((n) => n.title));

// New mail on the demo server. Only a sync brings it into Rukoo.
const deliver = (subject) =>
  app.evaluate(async (_electron, subject) => {
    const engine = global.__semEngine;
    const raw = [
      'From: Sanne de Vries <sanne@example.com>',
      'To: demo@example.com',
      `Subject: ${subject}`,
      `Date: ${new Date().toUTCString()}`,
      `Message-ID: <${Date.now()}.tray@example.com>`,
      '',
      'Hello'
    ].join('\r\n');
    await engine.session(engine.accounts[0]).append('INBOX', raw);
  }, subject);
const hasMessage = (subject) => app.evaluate((_electron, subject) => global.__semEngine.listMessages({}).some((m) => m.subject === subject), subject);

// Sync every 1.2 seconds, through the same call Settings makes, so main sets its timer again.
const syncOften = () => win.evaluate(() => window.mail.call('updateSettings', { syncInterval: 0.02 }));

// Writes what happens during a quit to a file, because the app is gone afterwards.
const quitLog = path.join(os.tmpdir(), `rukoo-tray-quit-${process.pid}.log`);
async function watchQuit() {
  fs.rmSync(quitLog, { force: true });
  await app.evaluate(({ app }, file) => {
    const fs = process.mainModule.require('fs');
    const log = (what) => fs.appendFileSync(file, `${what}\n`);
    const wrap = (object, method, name) => {
      const orig = object[method].bind(object);
      object[method] = (...args) => {
        log(name);
        return orig(...args);
      };
    };
    app.on('before-quit', () => log('before-quit'));
    app.on('will-quit', () => log('will-quit'));
    wrap(global.__semEngine, 'flush', 'engine flushed');
    wrap(global.__semAgents, 'dispose', 'agents closed');
    if (global.__semTray) wrap(global.__semTray, 'destroy', 'tray removed');
  }, quitLog);
}
// Answers the question about a draft with a button that lets the quit go on. Don't save quits at once, so the
// window can close before Playwright's click has returned, which it reports as a closed page. quitEvents() then
// checks that the quit really happened.
async function answerAndQuit(where, button) {
  await where.click(`.scrim .buttons button:text-is("${button}")`).catch((err) => {
    if (!/closed/i.test(String(err && err.message))) throw err;
  });
}
async function quitEvents() {
  // will-quit waits up to 5 s for the agents; a busy machine adds to that.
  await expect.poll(() => running, { timeout: 30000 }).toBe(false);
  const lines = fs.readFileSync(quitLog, 'utf8').trim().split('\n');
  fs.rmSync(quitLog, { force: true });
  return lines;
}

async function inlineDraft(subject, text) {
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  await win.fill('.composer [data-field="subject"]', subject);
  await win.click('.composer .editor');
  await win.keyboard.type(text);
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'true');
}

test('closing the window hides it in the tray, where mail keeps syncing and notifying', async () => {
  const before = await main();
  const count = await unread();
  expect(await tray()).toEqual({
    tooltip: `Rukoo Mail: ${count} unread emails`,
    dot: false,
    labels: ['Open Rukoo', 'New message', 'Sync now', 'Quit']
  });
  await inlineDraft('Half written', 'Still here');
  await recordNotifications();
  await syncOften();

  await closeMain();
  await expect.poll(main).toMatchObject({ hidden: true, visible: false });
  // Hidden, not closed: the window and the unsaved draft in it are still there, and nothing asked about it.
  expect(await windowCount()).toBe(1);
  await expect(win.locator('.scrim')).toHaveCount(0);
  // The agents' tools that need the window still reach it.
  const draft = await app.evaluate(() => global.__semAgents.ui('getDraft'));
  expect(draft.subject).toBe('Half written');

  // The sync timer picks up new mail and notifies, with the window hidden.
  await deliver('Arrived while hidden');
  await expect.poll(shown, { timeout: 10000 }).toContain('Sanne de Vries');
  expect(await hasMessage('Arrived while hidden')).toBe(true);
  // The tray counts it, and its dot stands in for the taskbar badge.
  await expect.poll(tray).toMatchObject({ tooltip: `Rukoo Mail: ${count + 1} unread emails`, dot: true });

  // The tray icon brings the window back where it was, with the draft as it was.
  await trayClick();
  await expect.poll(visible).toBe(true);
  expect((await main()).bounds).toEqual(before.bounds);
  await expect(win.locator('.composer [data-field="subject"]')).toHaveValue('Half written');
  await expect(win.locator('.composer .editor')).toContainText('Still here');
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'true');
});

// Real nativeImages: the icons on this platform are built as before, and the macOS menu bar images (built here
// too, since macOS CI only runs macos.spec.js) have both representations, each with the dot.
test('the tray icon is built as before here, and the macOS menu bar icon is 18 points with a 2x copy', async () => {
  const icons = await app.evaluate(({ app, nativeImage, screen }) => {
    const path = process.mainModule.require('path');
    const { trayImages, withDot } = process.mainModule.require(path.join(app.getAppPath(), 'src', 'main', 'tray.js'));
    const icon = path.join(app.getAppPath(), 'build', 'icon.png');
    const shape = (image) => ({ size: image.getSize(), scales: image.getScaleFactors() });
    const { plain, dot } = global.__semTray.images;
    const size = Math.round(16 * screen.getPrimaryDisplay().scaleFactor);
    // The tray images as main made them before the menu bar change.
    const before = nativeImage.createFromPath(icon).resize({ width: size, height: size, quality: 'best' });
    const beforeDot = nativeImage.createFromBitmap(withDot(before.toBitmap(), size), { width: size, height: size });
    // Shapes first: asking a resized image for another scale factor adds that representation.
    const here = { plain: shape(plain), dot: shape(dot), size };
    // Linux's tray asks for higher scale factors on HiDPI screens; the resized image draws those from the source.
    here.scales = [1, 1.5, 2].map((scaleFactor) => {
      const [a, b, c, d] = [plain, before, dot, beforeDot].map((image) => image.toBitmap({ scaleFactor }));
      return { scaleFactor, bytes: a.length, plain: a.equals(b), dot: c.equals(d) };
    });
    const mac = trayImages(icon, 'darwin', 1);
    return {
      here,
      mac: { plain: shape(mac.plain), dot: shape(mac.dot) },
      bitmaps: [1, 2].map((scaleFactor) => {
        const [a, b] = [mac.plain, mac.dot].map((image) => image.toBitmap({ scaleFactor }));
        const side = 18 * scaleFactor;
        const centre = (Math.floor(side * 0.78) * side + Math.floor(side * 0.78)) * 4;
        const top = Math.floor(side / 3) * side * 4;
        let opaque = 0;
        for (let i = 3; i < a.length; i += 4) if (a[i] > 0) opaque++;
        return { bytes: a.length, opaque, topSame: a.subarray(0, top).equals(b.subarray(0, top)), dot: [...b.subarray(centre, centre + 4)] };
      })
    };
  });
  if (process.platform !== 'darwin') {
    const { size, scales } = icons.here;
    for (const image of [icons.here.plain, icons.here.dot]) expect(image).toEqual({ size: { width: size, height: size }, scales: [1] });
    for (const { scaleFactor, plain, dot } of scales) expect({ scaleFactor, plain, dot }).toEqual({ scaleFactor, plain: true, dot: true });
    // At 2x the plain icon has twice the pixels each way, not the 1x bitmap again.
    expect(scales.find((s) => s.scaleFactor === 2).bytes).toBe(2 * size * 2 * size * 4);
  }
  for (const image of [icons.mac.plain, icons.mac.dot]) expect(image).toEqual({ size: { width: 18, height: 18 }, scales: [1, 2] });
  for (const [i, scaleFactor] of [1, 2].entries()) {
    const side = 18 * scaleFactor;
    const { bytes, opaque, topSame, dot } = icons.bitmaps[i];
    expect(bytes).toBe(side * side * 4);
    // The pigeon is there at this size, the dot leaves its top third alone and is the badge colour, in BGRA.
    expect(opaque).toBeGreaterThan((side * side) / 4);
    expect(topSame).toBe(true);
    expect(dot).toEqual([0x6c, 0x33, 0xd6, 255]);
  }
});

test('clicking a notification brings the hidden window back with the new email open', async () => {
  await recordNotifications();
  await closeMain();
  await expect.poll(main).toMatchObject({ hidden: true });
  await deliver('Open me from the notification');
  await trayMenu('sync');
  await expect.poll(shown).toContain('Sanne de Vries');
  await app.evaluate(() => global.__shown.find((n) => n.body.startsWith('Open me from the notification')).emit('click'));
  await expect.poll(main).toMatchObject({ hidden: false, visible: true });
  await expect(win.locator('.reader-subject')).toHaveText('Open me from the notification');
});

test('the tray menu opens Rukoo, starts a new message, syncs and follows the language', async () => {
  await closeMain();
  await expect.poll(visible).toBe(false);

  // New message opens a compose window; the main window stays in the tray.
  const [compose] = await Promise.all([app.waitForEvent('window'), trayMenu('compose')]);
  page(compose);
  await compose.waitForSelector('.compose .editor');
  expect(await visible()).toBe(false);

  // Open Rukoo shows the main window. Closing it to the tray again leaves the compose window alone.
  await trayMenu('open');
  await expect.poll(visible).toBe(true);
  await closeMain();
  await expect.poll(visible).toBe(false);
  expect(await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().endsWith('compose.html')).isVisible())).toBe(true);

  // Sync now fetches mail that arrived since the last sync.
  await deliver('Fetched from the tray');
  expect(await hasMessage('Fetched from the tray')).toBe(false);
  await trayMenu('sync');
  await expect.poll(() => hasMessage('Fetched from the tray')).toBe(true);

  // The menu and the tooltip switch language with the app.
  await win.evaluate(() => window.mail.call('updateSettings', { language: 'nl' }));
  const count = await unread();
  await expect.poll(tray).toMatchObject({
    tooltip: `Rukoo Mail: ${count} ongelezen e-mails`,
    labels: ['Rukoo openen', 'Nieuw bericht', 'Nu synchroniseren', 'Afsluiten']
  });
});

test('with minimize to the tray on, minimizing hides the window and the tray brings it back where it was', async () => {
  test.skip(MAC, NOT_ON_MAC);
  await win.click('[data-action="settings"]');
  const row = win.locator('.settings [data-a="toggle:minimizeToTray"]');
  await expect(row).toContainText('Minimize to the tray');
  await expect(row.locator('.switch')).not.toHaveClass(/\bon\b/);
  await expect(win.locator('.settings [data-a="toggle:closeToTray"] .switch')).toHaveClass(/\bon\b/);
  await row.click();
  await expect(row.locator('.switch')).toHaveClass(/\bon\b/);
  expect(saved().minimizeToTray).toBe(true);
  await win.click('.settings [data-a="back"]');

  const before = await main();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].minimize());
  await expect.poll(main).toMatchObject({ hidden: true, minimized: true });
  await trayClick();
  await expect.poll(main).toMatchObject({ hidden: false, visible: true, minimized: false });
  expect((await main()).bounds).toEqual(before.bounds);
});

test('Quit in the tray menu goes through before-quit and will-quit', async () => {
  await closeMain();
  await expect.poll(visible).toBe(false);
  await watchQuit();
  await trayMenu('quit');
  // before-quit flushes the engine; will-quit removes the tray icon and closes the agents.
  expect(await quitEvents()).toEqual(['engine flushed', 'before-quit', 'tray removed', 'agents closed', 'will-quit']);
});

// The drafts on the demo server, read from its file, so this works after Rukoo has quit.
const savedDrafts = () => JSON.parse(fs.readFileSync(path.join(dataDir, 'data', 'demo-server.json'), 'utf8')).boxes.Drafts.messages.map((m) => m.subject);
// The quit asks once, holds while the question is open, and asks again once it is answered.
const QUIT_AFTER_QUESTION = ['engine flushed', 'before-quit', 'engine flushed', 'before-quit', 'tray removed', 'agents closed', 'will-quit'];

// A compose window from the tray menu with an unsaved subject; the main window goes to the tray.
async function composeDraft(subject) {
  const [compose] = await Promise.all([app.waitForEvent('window'), trayMenu('compose')]);
  page(compose);
  await compose.waitForSelector('.compose .editor');
  await compose.fill('[data-field="subject"]', subject);
  await expect(compose.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  await closeMain();
  await expect.poll(visible).toBe(false);
  return compose;
}

test('Cancel in the question about a draft in the reading pane calls off a quit from the tray', async () => {
  await recordNotifications();
  await syncOften();
  await inlineDraft('Not saved yet', 'Some words');
  await closeMain();
  await expect.poll(visible).toBe(false);

  await trayMenu('quit');
  // The window comes out of the tray to ask about the draft.
  await expect.poll(visible).toBe(true);
  await expect(win.locator('.scrim')).toContainText('Save draft?');
  await win.click('.scrim .buttons button:text-is("Cancel")');
  await expect(win.locator('.composer .editor')).toContainText('Some words');

  // Not quitting any more: sync runs, and closing goes to the tray again.
  await deliver('After the cancelled quit');
  await expect.poll(shown, { timeout: 10000 }).toContain('Sanne de Vries');
  await closeMain();
  await expect.poll(visible).toBe(false);
  expect(await windowCount()).toBe(1);
  expect(await tray()).not.toBeNull();
  expect(running).toBe(true);
});

for (const [button, kept] of [["Don't save", false], ['Save', true]]) {
  test(`${button} in the question about a draft in the reading pane finishes a quit from the tray`, async () => {
    const subject = `Reading pane, ${button}`;
    await inlineDraft(subject, 'Some words');
    await closeMain();
    await expect.poll(visible).toBe(false);
    await watchQuit();

    await trayMenu('quit');
    await expect.poll(visible).toBe(true);
    await expect(win.locator('.scrim')).toContainText('Save draft?');
    await answerAndQuit(win, button);
    expect(await quitEvents()).toEqual(QUIT_AFTER_QUESTION);
    expect(savedDrafts().includes(subject)).toBe(kept);
  });
}

// A downloaded update (main's installer.js) whose installer only writes to the quit log.
async function updateReady(version = '9.9.9') {
  await app.evaluate(({}, { file, version }) => {
    const fs = process.mainModule.require('fs');
    const tag = `v${version}`;
    global.__semUpdates.available = { version, name: tag, url: `https://github.com/MajesteitBart/rukoo/releases/tag/${tag}`, prerelease: version.includes('-'), feed: `https://github.com/MajesteitBart/rukoo/releases/download/${tag}/` };
    Object.assign(global.__semInstaller, {
      kind: 'nsis',
      status: 'ready',
      version,
      updater: { install: (silent, restart) => (fs.appendFileSync(file, `install ${silent ? 'silent' : 'asking'}${restart ? ' restart' : ''}\n`), true) }
    });
  }, { file: quitLog, version });
}

test('Restart to update asks about a draft first: Cancel installs nothing, Save installs after every window has closed', async () => {
  await inlineDraft('Before the update', 'Some words');
  await watchQuit();
  await updateReady();
  await win.evaluate(() => window.mail.call('updatesInstall'));
  await expect(win.locator('.scrim')).toContainText('Save draft?');
  await win.click('.scrim .buttons button:text-is("Cancel")');
  await expect(win.locator('.composer .editor')).toContainText('Some words');
  await win.waitForTimeout(1000);
  expect(running).toBe(true);
  expect(fs.readFileSync(quitLog, 'utf8')).not.toContain('install');

  // Restart again, and keep the draft this time.
  fs.rmSync(quitLog, { force: true });
  await win.evaluate(() => window.mail.call('updatesInstall'));
  await expect(win.locator('.scrim')).toContainText('Save draft?');
  await answerAndQuit(win, 'Save');
  expect(await quitEvents()).toEqual([...QUIT_AFTER_QUESTION, 'install silent restart']);
  expect(savedDrafts()).toContain('Before the update');
});

test('a plain quit installs a downloaded update without restarting; switched back to Stable, it installs nothing', async () => {
  await watchQuit();
  await win.evaluate(() => window.mail.call('updateSettings', { updateChannel: 'nightly' }));
  await updateReady('9.9.9-nightly.20261030.1');
  // Back on Stable, the downloaded nightly isn't offered any more, before any check: this one can't reach GitHub,
  // and a failed check keeps what it found before.
  await win.evaluate(() => window.mail.call('updateSettings', { updateChannel: 'stable' }));
  expect(await app.evaluate(() => global.__semUpdates.available)).toBeNull();
  await trayMenu('quit');
  expect(await quitEvents()).not.toContain('install silent');
  await launch();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await watchQuit();
  await updateReady();
  await trayMenu('quit');
  expect((await quitEvents()).at(-1)).toBe('install silent');
});

test('with the automatic check off, Restart to update still installs what Check now downloaded, and restarts', async () => {
  await watchQuit();
  await win.evaluate(() => window.mail.call('updateSettings', { checkUpdates: false }));
  await updateReady();
  expect(await win.evaluate(() => window.mail.call('updatesInstall'))).toBe(true);
  expect((await quitEvents()).at(-1)).toBe('install silent restart');
});

test('a save that fails during a quit keeps Rukoo running with the draft', async () => {
  await inlineDraft('Cannot be saved', 'Some words');
  await app.evaluate(() => {
    global.__semEngine.saveDraft = async () => {
      throw new Error('Connection lost');
    };
  });
  await closeMain();
  await expect.poll(visible).toBe(false);

  await trayMenu('quit');
  await expect(win.locator('.scrim')).toContainText('Save draft?');
  await win.click('.scrim .buttons button:text-is("Save")');
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  await expect(win.locator('.composer .editor')).toContainText('Some words');
  // Called off: closing goes to the tray again instead of quitting.
  await closeMain();
  await expect.poll(visible).toBe(false);
  expect(running).toBe(true);
});

test('Cancel in a compose window calls off a quit; a main window comes back hidden in the tray', async () => {
  const compose = await composeDraft('Popped out');
  await trayMenu('quit');
  // The main window had nothing to ask and closed; the compose window asks.
  await expect(compose.locator('.scrim')).toContainText('Save draft?');
  await expect.poll(main).toBeNull();
  const [fresh] = await Promise.all([app.waitForEvent('window'), compose.click('.scrim .buttons button:text-is("Cancel")')]);
  win = page(fresh);
  await expect(compose.locator('.scrim')).toHaveCount(0);
  await expect(compose.locator('[data-field="subject"]')).toHaveValue('Popped out');
  // Rukoo is running with a main window in the tray, as after any close.
  await expect.poll(main).toMatchObject({ visible: false });
  expect(running).toBe(true);

  // The tray shows it, and closing it goes to the tray again.
  await trayClick();
  await expect.poll(visible).toBe(true);
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await closeMain();
  await expect.poll(visible).toBe(false);
  expect(running).toBe(true);
});

test('after a quit called off in a compose window, the end of the Windows session still cleans up once that window closes', async () => {
  const compose = await composeDraft('Closed later');
  await trayMenu('quit');
  await expect(compose.locator('.scrim')).toContainText('Save draft?');
  await Promise.all([app.waitForEvent('window'), compose.click('.scrim .buttons button:text-is("Cancel")')]);
  await expect(compose.locator('.scrim')).toHaveCount(0);
  // Closing the compose window now asks again, outside a quit; Don't save lets it go.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().endsWith('compose.html')).close());
  await expect(compose.locator('.scrim')).toContainText('Save draft?');
  await Promise.all([compose.waitForEvent('close'), compose.click(`.scrim .buttons button:text-is("Don't save")`)]);
  expect(await windowCount()).toBe(1);
  expect(running).toBe(true);

  // The hidden main window is still there to hear that Windows signs out.
  await watchQuit();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].emit('session-end'));
  const lines = () => (fs.existsSync(quitLog) ? fs.readFileSync(quitLog, 'utf8').trim().split(/\r?\n/) : []);
  await expect.poll(lines).toEqual(['engine flushed', 'tray removed', 'agents closed']);
  fs.rmSync(quitLog, { force: true });
});

for (const [button, kept] of [["Don't save", false], ['Save', true]]) {
  test(`${button} in a compose window finishes a quit from the tray`, async () => {
    const subject = `Compose window, ${button}`;
    const compose = await composeDraft(subject);
    await watchQuit();
    await trayMenu('quit');
    await expect(compose.locator('.scrim')).toContainText('Save draft?');
    await answerAndQuit(compose, button);
    expect(await quitEvents()).toEqual(QUIT_AFTER_QUESTION);
    expect(savedDrafts().includes(subject)).toBe(kept);
  });
}

// A crashed page can't answer the question, so it can't hold up the quit either.
for (const where of ['reading pane', 'compose window']) {
  test(`a quit from the tray finishes when the ${where} crashes while it asks`, async () => {
    let asking = win;
    if (where === 'reading pane') {
      await inlineDraft('Lost in a crash', 'Some words');
      await closeMain();
      await expect.poll(visible).toBe(false);
    } else asking = await composeDraft('Lost in a crash');
    await watchQuit();
    await trayMenu('quit');
    await expect(asking.locator('.scrim')).toContainText('Save draft?');
    const page = where === 'reading pane' ? 'index.html' : 'compose.html';
    await app.evaluate(({ BrowserWindow }, page) => BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().endsWith(page)).webContents.forcefullyCrashRenderer(), page);
    expect(await quitEvents()).toEqual(QUIT_AFTER_QUESTION);
  });
}

test('the end of the Windows session cleans up while only a compose window asks', async () => {
  const compose = await composeDraft('Asking at sign-out');
  await trayMenu('quit');
  await expect(compose.locator('.scrim')).toContainText('Save draft?');
  await expect.poll(main).toBeNull();
  await watchQuit();
  // Only the compose window is left to hear it.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].emit('session-end'));
  const lines = () => (fs.existsSync(quitLog) ? fs.readFileSync(quitLog, 'utf8').trim().split(/\r?\n/) : []);
  await expect.poll(lines).toEqual(['engine flushed', 'tray removed', 'agents closed']);
  expect(await tray()).toBeNull();
  fs.rmSync(quitLog, { force: true });
});

test('with both settings off there is no tray icon, and closing the window quits as before', async () => {
  test.skip(MAC, NOT_ON_MAC);
  await win.click('[data-action="settings"]');
  const row = win.locator('.settings [data-a="toggle:closeToTray"]');
  await expect(row).toContainText('Close to the tray');
  await row.click();
  await expect(row.locator('.switch')).not.toHaveClass(/\bon\b/);
  expect(saved()).toMatchObject({ closeToTray: false, minimizeToTray: false });
  expect(await tray()).toBeNull();
  await win.click('.settings [data-a="back"]');

  // Minimizing leaves the window on the taskbar.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].minimize());
  await expect.poll(main).toMatchObject({ hidden: false, minimized: true });

  await watchQuit();
  await closeMain();
  expect(await quitEvents()).toEqual(['engine flushed', 'before-quit', 'agents closed', 'will-quit']);
});

test('only minimize to the tray: closing quits, minimizing hides', async () => {
  test.skip(MAC, NOT_ON_MAC);
  await win.evaluate(() => window.mail.call('updateSettings', { closeToTray: false, minimizeToTray: true }));
  expect(await tray()).not.toBeNull();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].minimize());
  await expect.poll(main).toMatchObject({ hidden: true, minimized: true });
  await trayClick();
  await expect.poll(main).toMatchObject({ hidden: false, visible: true, minimized: false });
  await watchQuit();
  await closeMain();
  expect(await quitEvents()).toEqual(['engine flushed', 'before-quit', 'tray removed', 'agents closed', 'will-quit']);
});

test('starting Rukoo again shows the hidden window, and the end of the Windows session lets it close', async () => {
  test.skip(MAC, NOT_ON_MAC);
  await closeMain();
  await expect.poll(visible).toBe(false);
  // What a second start sends to the running app (tests run without the single-instance lock).
  await app.evaluate(({ app }) => app.emit('second-instance', {}, [], process.cwd()));
  await expect.poll(visible).toBe(true);

  // Windows signs out: no before-quit follows, so main saves now, removes the tray icon and lets the window close.
  await watchQuit();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].emit('session-end'));
  expect(await tray()).toBeNull();
  await closeMain();
  const events = await quitEvents();
  expect(events.slice(0, 3)).toEqual(['engine flushed', 'tray removed', 'agents closed']);
  expect(events).toContain('will-quit');
});

test('a quit after the end of the Windows session still waits for the agents to close', async () => {
  test.skip(MAC, NOT_ON_MAC);
  await watchQuit();
  // Closing the agents takes a while, as a stop request to Hermes or the kill of a process tree can.
  await app.evaluate((_electron, file) => {
    const fs = process.mainModule.require('fs');
    const log = (what) => fs.appendFileSync(file, `${what}\n`);
    const calendars = global.__semCalendars;
    const flush = calendars.flush.bind(calendars);
    calendars.flush = () => {
      log('calendars flushed');
      return flush();
    };
    const hub = global.__semAgents;
    const dispose = hub.dispose.bind(hub);
    hub.dispose = () => dispose().then(() => new Promise((resolve) => setTimeout(resolve, 1200))).then(() => log('agents done'));
  }, quitLog);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].emit('session-end'));
  // Windows lets the window close, and the quit that follows waits for the agents session-end started closing.
  await closeMain();
  const events = await quitEvents();
  expect(events.slice(0, 4)).toEqual(['engine flushed', 'calendars flushed', 'tray removed', 'agents closed']);
  expect(events.slice(-2)).toEqual(['will-quit', 'agents done']);
});

test('the close button keeps the window and its question in view while a quit asks about a draft', async () => {
  await inlineDraft('Still asking', 'Some words');
  await closeMain();
  await expect.poll(visible).toBe(false);
  await trayMenu('quit');
  await expect.poll(visible).toBe(true);
  await expect(win.locator('.scrim')).toContainText('Save draft?');

  await closeMain();
  await win.waitForTimeout(500);
  expect(await visible()).toBe(true);
  await expect(win.locator('.scrim')).toContainText('Save draft?');

  // The question still decides the quit.
  await watchQuit();
  await win.click('.scrim .buttons button:text-is("Save")');
  expect(await quitEvents()).toEqual(['engine flushed', 'before-quit', 'tray removed', 'agents closed', 'will-quit']);
  expect(savedDrafts()).toContain('Still asking');
});

// Sending takes a while, as it does over a slow connection.
const slowSend = (fail = false) =>
  app.evaluate((_electron, fail) => {
    const engine = global.__semEngine;
    const send = engine.send.bind(engine);
    engine.send = async (payload) => {
      await new Promise((resolve) => setTimeout(resolve, 1500));
      if (fail) throw new Error('Connection lost');
      return send(payload);
    };
  }, fail);
const inboxSubjects = () => JSON.parse(fs.readFileSync(path.join(dataDir, 'data', 'demo-server.json'), 'utf8')).boxes.INBOX.messages.map((m) => m.subject);

async function inlineMessage(subject) {
  await inlineDraft(subject, 'Some words');
  await win.fill('.composer [data-rinput="to"]', 'demo@example.com');
  await win.keyboard.press('Enter');
  await expect(win.locator('.composer [data-rfield="to"] .recipient')).toHaveCount(1);
}

test('a quit from the tray during a send waits for it and goes on once the mail is sent', async () => {
  await inlineMessage('Sent while quitting');
  await slowSend();
  await win.click('.composer [data-c="send"]');
  await expect(win.locator('.composer [data-c="send"]')).toBeDisabled();
  await watchQuit();
  await trayMenu('quit');
  expect(await quitEvents()).toEqual(QUIT_AFTER_QUESTION);
  expect(inboxSubjects()).toContain('Sent while quitting');
});

test('a send that fails during a quit calls the quit off and keeps the message', async () => {
  await inlineMessage('Not sent while quitting');
  await slowSend(true);
  await win.click('.composer [data-c="send"]');
  await expect(win.locator('.composer [data-c="send"]')).toBeDisabled();
  await trayMenu('quit');
  await expect(win.locator('.scrim')).toContainText('Connection lost');
  await win.click('.scrim .buttons button');
  await expect(win.locator('.composer [data-c="send"]')).toBeEnabled();
  await expect(win.locator('.composer .editor')).toContainText('Some words');
  // Called off: closing goes to the tray again instead of quitting.
  await closeMain();
  await expect.poll(visible).toBe(false);
  expect(running).toBe(true);
});
