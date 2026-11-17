// New mail reaching the list by itself while Rukoo runs, over IMAP. Two accounts on test mailboxes that take new mail
// while the app runs (imapMailbox in test/fixtures/imap-server.js), so "All accounts" exists; swipe actions and close
// to the tray on. Each test files an email on a server and runs the sync the way main's timer runs it, then checks
// the row shows without anyone touching the window: with the reader open, after minimize and restore, after hiding to
// the tray and showing, and above a scrolled list. Then the cases that held the list back: a refresh that failed, an
// email whose Date header no parser reads, and a touch on a row whose pointerup never came. Last, the sync when the
// window comes forward.
//
// On Linux a hidden or minimized window still counts as visible to the page; CI's Windows shards run this where it
// is really hidden.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');
const { imapMailbox } = require('../fixtures/imap-server');

const ROOT = path.join(__dirname, '..', '..');
const HOUR = 3600000;
const ACCOUNTS = [
  { id: 'aaaa1111-0000-4000-8000-000000000001', name: 'Alpha', email: 'anna@alpha.test', color: '#6f9cf2' },
  { id: 'bbbb2222-0000-4000-8000-000000000002', name: 'Beta', email: 'bob@beta.test', color: '#e0795b' }
];

let servers;
let dataDir;
let app;
let win;
let pageErrors;

function seed(server, tag, offset) {
  const now = Date.now();
  // Oldest first, so uids grow with the date like a real inbox; the newest is two hours old.
  for (let i = 39; i >= 0; i--) {
    server.put('INBOX', { subject: `${tag} message ${40 - i}`, from: [`${tag} Sender ${i % 5}`, `s${i % 5}@${tag.toLowerCase()}.test`], date: now - 2 * HOUR - offset - i * 30 * 60000, flags: i === 3 ? [] : ['\\Seen'] });
  }
  for (let i = 0; i < 3; i++) server.put('Sent', { subject: `${tag} sent ${i}`, from: [tag, server.email], date: now - 3 * HOUR - i * HOUR, flags: ['\\Seen'] });
}

function writeProfile(settings = {}) {
  const dir = path.join(dataDir, 'data');
  fs.mkdirSync(dir, { recursive: true });
  const accounts = ACCOUNTS.map((a, i) => ({
    id: a.id,
    type: 'imap',
    provider: 'other',
    email: a.email,
    name: a.name,
    label: a.email,
    color: a.color,
    signature: null,
    imap: { host: '127.0.0.1', port: servers[i].port, secure: false, user: a.email },
    smtp: { host: '127.0.0.1', port: 9, secure: false, user: a.email },
    secret: `plain:${Buffer.from('test-password').toString('base64')}`,
    createdAt: Date.now() - 86400000
  }));
  fs.writeFileSync(path.join(dir, 'accounts.json'), JSON.stringify(accounts));
  fs.writeFileSync(
    path.join(dir, 'settings.json'),
    JSON.stringify({ language: 'en', swipeActions: true, swipeRight: 'read', swipeLeft: 'archive', closeToTray: true, sort: 'date-desc', notifications: true, syncInterval: 5, defaultAccountId: ACCOUNTS[0].id, signatureOptIn: true, ...settings })
  );
}

const syncsDone = () =>
  app.evaluate(() => {
    const e = global.__semEngine;
    return e.syncs.size === 0 && e.accounts.every((a) => e.caches.get(a.id) && e.caches.get(a.id).lastSync);
  });

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-new-mail-sync-e2e-');
  servers = [await imapMailbox({ email: ACCOUNTS[0].email }), await imapMailbox({ email: ACCOUNTS[1].email })];
  seed(servers[0], 'Alpha', 0);
  seed(servers[1], 'Beta', 13 * 60000);
  writeProfile();
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1', RUKOO_ACCOUNT_SERVER: 'http://127.0.0.1:9' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.'], cwd: ROOT, env });
  win = await app.firstWindow();
  pageErrors = [];
  win.on('pageerror', (err) => pageErrors.push(String(err && err.message)));
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await expect.poll(syncsDone, { timeout: 30000 }).toBe(true);
  await expect(win.locator('.list-scroll .item').first()).toBeVisible({ timeout: 15000 });
  // The start's own sync has reached the list.
  await expect(win.locator('.list-scroll .item', { hasText: 'Beta message 40' })).toBeVisible();
});

test.afterEach(async () => {
  await closeApp(app);
  for (const s of servers) await s.close();
  removeDir(dataDir);
});

let delivered = 0;
// An email lands in an account's inbox on its server; the app knows nothing of it until a sync.
function deliver(account, label, extra = {}) {
  delivered++;
  const subject = `${label} ${delivered}`;
  servers[account].put('INBOX', { subject, from: ['New Sender', 'new@sender.test'], date: Date.now(), ...extra });
  return subject;
}

// What main's sync timer runs.
const timerSync = () => app.evaluate(() => global.__semEngine.syncAll().catch((err) => err.message));
const row = (subject) => win.locator('.list-scroll .item', { hasText: subject });
const readerSubject = () => win.locator('.reader-subject');
const log = () => {
  const file = path.join(dataDir, 'data', 'logs', 'rukoo.log');
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '';
};
// A touch on a row (the third) that goes down and never comes up, as a pointerup the window never got.
const stuckTouch = () =>
  win.locator('.list-scroll .item').nth(2).evaluate((item) => {
    const r = item.getBoundingClientRect();
    item.dispatchEvent(new PointerEvent('pointerdown', { pointerType: 'touch', pointerId: 9, isPrimary: true, bubbles: true, cancelable: true, button: 0, buttons: 1, clientX: r.left + 50, clientY: r.top + 10 }));
  });

test('with an email open in the reader, new mail shows in the list by itself, and the log has the sync', async () => {
  await win.locator('.list-scroll .item').nth(2).click();
  await expect(readerSubject()).toBeVisible();
  const open = await readerSubject().textContent();
  const subject = deliver(0, 'Lunch on Friday?');
  expect(await timerSync()).toBeUndefined();
  await expect(row(subject)).toBeVisible();
  await expect(readerSubject()).toHaveText(open);

  // One line per account per sync, with the email it found, and nothing that names the account or the email.
  const lines = log().split('\n');
  expect(lines.filter((l) => / INFO  sync: account=aaaa1111 other\/password start=\S+ ms=\d+ folders=5 new=1$/.test(l))).toHaveLength(1);
  expect(lines.filter((l) => / INFO  sync: account=bbbb2222 other\/password .* new=0$/.test(l)).length).toBeGreaterThanOrEqual(2);
  expect(log()).not.toMatch(/alpha\.test|beta\.test|Lunch|Sender/);
  expect(pageErrors).toEqual([]);
});

test('new mail that came while the window was minimized shows when it is restored', async () => {
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].minimize());
  const subject = deliver(1, 'While minimized');
  await timerSync();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].restore());
  await expect(row(subject)).toBeVisible();
});

test('new mail that came while the window was in the tray shows when it comes back', async () => {
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].close());
  await expect.poll(() => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].isVisible())).toBe(false);
  const subject = deliver(0, 'While in the tray');
  await timerSync();
  // What the tray's Open does in test mode (showWindow()).
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].showInactive());
  await expect(row(subject)).toBeVisible();
});

test('above a list scrolled down, new mail gets the pill that counts it', async () => {
  const scroll = win.locator('.list-scroll');
  await scroll.evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await expect.poll(() => scroll.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  const subject = deliver(0, 'Above the fold');
  await timerSync();
  await expect(win.locator('.new-pill')).toHaveText('1 new email');
  await win.locator('.new-pill').click();
  await expect(row(subject)).toBeInViewport();
});

test('a refresh that fails still runs the one asked for meanwhile, and goes in the log', async () => {
  // The next list the window asks for waits for a gate, then fails; the one after works.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const list = engine.listMessages;
    global.__calls = 0;
    global.__gate = new Promise((resolve) => (global.__open = resolve));
    engine.listMessages = function () {
      global.__calls++;
      engine.listMessages = list;
      return global.__gate.then(() => {
        throw new Error('The list could not be read this time');
      });
    };
    engine.emit('updated');
  });
  await expect.poll(() => app.evaluate(() => global.__calls)).toBe(1);
  // The sync's news reaches the window while that pass still waits.
  const subject = deliver(0, 'Asked for meanwhile');
  await timerSync();
  await win.waitForTimeout(500);
  await app.evaluate(() => global.__open());
  await expect(row(subject)).toBeVisible();
  await expect.poll(log).toMatch(/WARN  window main: refresh failures=1 The list could not be read this time stack: /);
  expect(pageErrors).toEqual([]);
});

test('a refresh that keeps failing is tried again now and then, never in a loop, and draws once it works', async () => {
  await app.evaluate(() => {
    const engine = global.__semEngine;
    global.__list = engine.listMessages;
    global.__calls = 0;
    engine.listMessages = function () {
      global.__calls++;
      throw new Error('The list keeps failing');
    };
  });
  const subject = deliver(0, 'After the trouble');
  await timerSync();
  await win.waitForTimeout(6000);
  const calls = await app.evaluate(() => global.__calls);
  expect(calls).toBeGreaterThanOrEqual(2);
  expect(calls).toBeLessThanOrEqual(12);
  await expect(row(subject)).toHaveCount(0);
  await app.evaluate(() => {
    global.__semEngine.listMessages = global.__list;
  });
  // The next try, at most 10 s on.
  await expect(row(subject)).toBeVisible({ timeout: 15000 });
  await expect.poll(log).toMatch(/window main: refresh reason=recovered failures=\d+/);
  // Logged once, not once per try.
  expect(log().split('\n').filter((l) => l.includes('The list keeps failing'))).toHaveLength(1);
});

test('an email whose Date header no parser reads shows with the date the server received it, and the list goes on drawing', async () => {
  const odd = deliver(0, 'Odd date', { dateHeader: 'Tue, 31 Febtember 2026 99:99:99 +9999' });
  await timerSync();
  await expect(row(odd)).toBeVisible();
  await expect(row(odd).locator('time')).toHaveText(/^\d\d:\d\d$/);
  const next = deliver(1, 'Right after it');
  await timerSync();
  await expect(row(next)).toBeVisible();
  expect(await app.evaluate((_e, s) => global.__semEngine.listMessages({ scope: 'all', view: 'inbox' }).find((m) => m.subject === s).date, odd)).toBeGreaterThan(Date.now() - 60000);
  expect(pageErrors).toEqual([]);
});

test('a touch on a row whose pointerup never came holds the list for a few seconds at most', async () => {
  await stuckTouch();
  const subject = deliver(0, 'Under a stuck finger');
  await timerSync();
  await expect(row(subject)).toBeVisible({ timeout: 6000 });
  await expect.poll(log).toMatch(/INFO  window main: list reason=swipe-stale ms=\d+/);

  // Another pointer going down lets go of it at once.
  await stuckTouch();
  const next = deliver(1, 'After a click');
  await timerSync();
  await win.waitForTimeout(300);
  await expect(row(next)).toHaveCount(0);
  await win.locator('.list-title').click();
  await expect(row(next)).toBeVisible({ timeout: 1000 });

  // A pen that went back to hovering lets go at its next move.
  await win.locator('.list-scroll .item').nth(2).evaluate((item) => {
    const r = item.getBoundingClientRect();
    item.dispatchEvent(new PointerEvent('pointerdown', { pointerType: 'pen', pointerId: 11, isPrimary: true, bubbles: true, cancelable: true, button: 0, buttons: 1, clientX: r.left + 50, clientY: r.top + 10 }));
    window.__pen = item;
  });
  const hovered = deliver(0, 'After the pen lifted');
  await timerSync();
  await win.waitForTimeout(300);
  await expect(row(hovered)).toHaveCount(0);
  await win.evaluate(() => {
    const r = window.__pen.getBoundingClientRect();
    window.__pen.dispatchEvent(new PointerEvent('pointermove', { pointerType: 'pen', pointerId: 11, isPrimary: true, bubbles: true, buttons: 0, clientX: r.left + 60, clientY: r.top + 12 }));
  });
  await expect(row(hovered)).toBeVisible({ timeout: 1000 });

  // A finger that keeps moving a little, without its pointerup ever coming, holds the list 10 s at most.
  await win.locator('.list-scroll .item').nth(2).evaluate((item) => {
    const r = item.getBoundingClientRect();
    const at = { pointerType: 'touch', pointerId: 12, isPrimary: true, bubbles: true, cancelable: true, button: 0, buttons: 1, clientY: r.top + 10 };
    item.dispatchEvent(new PointerEvent('pointerdown', { ...at, clientX: r.left + 50 }));
    let n = 0;
    window.__wiggle = setInterval(() => item.dispatchEvent(new PointerEvent('pointermove', { ...at, clientX: r.left + 50 + (n++ % 2) * 4 })), 400);
  });
  const wiggled = deliver(1, 'Under a moving finger');
  await timerSync();
  // Past the 3 s a still pointer gets: the moves keep it alive.
  await win.waitForTimeout(4000);
  await expect(row(wiggled)).toHaveCount(0);
  await expect(row(wiggled)).toBeVisible({ timeout: 10000 });
  await win.evaluate(() => clearInterval(window.__wiggle));
  await expect.poll(log).toMatch(/INFO  window main: list reason=swipe-held ms=\d+/);
});

test('bringing the window forward syncs an account that last synced over a minute ago, once, and not with manual sync', async () => {
  const subject = deliver(0, 'Waiting on the server');
  // Both accounts last synced more than a minute ago.
  const backdate = () =>
    app.evaluate(() => {
      const e = global.__semEngine;
      for (const a of e.accounts) e.syncStarts.set(a.id, Date.now() - 61000);
    });
  await backdate();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].emit('focus'));
  await expect(row(subject)).toBeVisible();
  await expect.poll(syncsDone).toBe(true);
  // Within the minute, coming forward again (shown, restored, focused) starts no sync.
  const comeForward = () =>
    app.evaluate(({ BrowserWindow }) => {
      const w = BrowserWindow.getAllWindows()[0];
      const e = global.__semEngine;
      const before = [...e.syncStarts.values()];
      for (const event of ['show', 'restore', 'focus']) w.emit(event);
      return { syncing: e.syncs.size, same: JSON.stringify([...e.syncStarts.values()]) === JSON.stringify(before) };
    });
  expect(await comeForward()).toEqual({ syncing: 0, same: true });

  // With the schedule on manual, never.
  await app.evaluate(() => global.__semEngine.updateSettings({ syncInterval: 0 }));
  await backdate();
  const later = deliver(0, 'Still on the server');
  expect(await comeForward()).toEqual({ syncing: 0, same: true });
  await win.waitForTimeout(1000);
  await expect(row(later)).toHaveCount(0);
});
