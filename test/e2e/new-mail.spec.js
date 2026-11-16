// New mail while Rukoo runs (#98): the pill over a list scrolled away from it, the toast for mail the list on screen
// doesn't show, the sound for new mail, and the test notification in Settings. The real app with a throwaway profile
// and the demo account. Desktop notifications are recorded instead of shown and the sound is counted instead of played,
// so nothing reaches the screen or the speakers.
const { test, expect, _electron: electron } = require('@playwright/test');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-new-mail-e2e-');
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.'], cwd: path.join(__dirname, '..', '..'), env });
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await app.evaluate(({ Notification }) => {
    global.__shown = [];
    Notification.prototype.show = function () {
      global.__shown.push({ title: this.title, body: this.body, silent: this.silent });
    };
  });
  await win.evaluate(() => {
    window.__coos = 0;
    HTMLMediaElement.prototype.play = function () {
      window.__coos++;
      return Promise.resolve();
    };
    window.__newMail = [];
    window.mail.on(({ type, payload }) => type === 'new-mail' && window.__newMail.push(payload));
  });
});

test.afterEach(async () => {
  await closeApp(app);
  removeDir(dataDir);
});

const item = (text) => win.locator('.item', { hasText: text }).first();
const toastEl = () => win.locator('#toast');
const settings = () => win.locator('.page.settings');
// Whether the main window counts as in front, where the desktop notification stays away.
const focused = (on) =>
  app.evaluate(({ BrowserWindow }, on) => {
    BrowserWindow.getAllWindows()[0].isFocused = () => on;
  }, on);
// What main told the window about new mail, one entry per batch.
const heard = () => win.evaluate(() => window.__newMail);
const coos = () => win.evaluate(() => window.__coos);

// An email from Sanne lands in the demo inbox, and a sync finds it.
const arrive = (subject) =>
  app.evaluate(async (_e, subject) => {
    const engine = global.__semEngine;
    const raw = [
      'From: Sanne de Vries <sanne@example.com>',
      'To: demo@example.com',
      `Subject: ${subject}`,
      `Date: ${new Date().toUTCString()}`,
      `Message-ID: <${Date.now()}.${Math.random().toString(36).slice(2)}@example.com>`,
      '',
      'Hallo'
    ].join('\r\n');
    await engine.session(engine.accounts[0]).append('INBOX', raw);
    await engine.syncAll();
  }, subject);

test('new mail above a list scrolled down gets a pill that counts it and scrolls to it', async () => {
  await focused(false);
  // Short enough that the inbox scrolls.
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 600));
  const scroll = win.locator('.list-scroll');
  await scroll.evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await expect.poll(() => scroll.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  const pill = win.locator('.new-pill');
  await expect(pill).toBeHidden();

  await arrive('Lunch on Friday?');
  await expect(pill).toBeVisible();
  await expect(pill).toHaveText('1 new email');
  await expect(item('Lunch on Friday?')).not.toBeInViewport();
  await arrive('Tickets for Sunday');
  await expect(pill).toHaveText('2 new emails');
  // The list itself is the cue here: no toast, and in the background the desktop notifications as before.
  await expect(toastEl()).not.toContainText('New email');
  expect((await app.evaluate(() => global.__shown)).map((n) => n.title)).toEqual(['Sanne de Vries', 'Sanne de Vries']);

  await pill.click();
  await expect(pill).toBeHidden();
  await expect(item('Tickets for Sunday')).toBeInViewport();
  await expect(item('Lunch on Friday?')).toBeInViewport();

  // Scrolled to by hand, the pill goes, and stays away once the list scrolls down again.
  await scroll.evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await arrive('Coffee next week');
  await expect(pill).toHaveText('1 new email');
  await scroll.evaluate((el) => el.scrollTo(0, 0));
  await expect(pill).toBeHidden();
  await scroll.evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await expect(pill).toBeHidden();
  // Another folder drops the count.
  await arrive('One more thing');
  await expect(pill).toHaveText('1 new email');
  await win.click('[data-view="sent"]');
  await expect(win.locator('.list-title h1')).toHaveText('Sent');
  await expect(pill).toBeHidden();
});

test('new mail the list on screen does not show gets a toast while the window is in front, which opens it', async () => {
  await win.click('[data-view="sent"]');
  await expect(win.locator('.list-title h1')).toHaveText('Sent');
  await focused(true);
  await arrive('Lunch on Friday?');
  await expect(toastEl()).toContainText('New email from Sanne de Vries: Lunch on Friday?');
  await expect(toastEl().locator('button')).toHaveText('Open');
  // In front, the toast stands in for the desktop notification.
  expect(await app.evaluate(() => global.__shown)).toEqual([]);
  // Ctrl+Z undoes things; it doesn't open the email, and the toast stays.
  await win.locator('.list-scroll').focus();
  await win.keyboard.press(process.platform === 'darwin' ? 'Meta+z' : 'Control+z');
  await expect(toastEl()).toHaveClass(/show/);
  await expect(toastEl().locator('button')).toHaveText('Open');
  await toastEl().locator('button').click();
  await expect(win.locator('.list-title h1')).toHaveText('Inbox');
  await expect(win.locator('.reader-subject')).toHaveText('Lunch on Friday?');

  // Several emails: one toast that counts them, and Show goes to Inbox.
  await win.click('[data-view="sent"]');
  await app.evaluate(async () => {
    const engine = global.__semEngine;
    const s = engine.session(engine.accounts[0]);
    for (const [n, subject] of ['Tickets for Sunday', 'Coffee next week'].entries()) {
      await s.append('INBOX', ['From: Sanne de Vries <sanne@example.com>', 'To: demo@example.com', `Subject: ${subject}`, `Date: ${new Date().toUTCString()}`, `Message-ID: <batch.${n}.${Date.now()}@example.com>`, '', 'Hallo'].join('\r\n'));
    }
    await engine.syncAll();
  });
  await expect(toastEl()).toContainText('2 new emails');
  await toastEl().locator('button', { hasText: 'Show' }).click();
  await expect(win.locator('.list-title h1')).toHaveText('Inbox');
  await expect(item('Coffee next week')).toBeVisible();

  // In the background the desktop notification comes instead, and the toast doesn't.
  await win.click('[data-view="sent"]');
  await focused(false);
  await arrive('One more thing');
  await expect.poll(() => app.evaluate(() => global.__shown.map((n) => n.body.split('\n')[0]))).toEqual(['One more thing']);
  expect((await heard()).at(-1)).toMatchObject({ mails: [{ from: 'Sanne de Vries', subject: 'One more thing' }], toast: false, sound: false });
  // The sound is off, so the system's own plays.
  expect((await app.evaluate(() => global.__shown))[0].silent).toBe(false);
});

test('a toast with Undo stays up when new mail comes in, and the new-mail toast follows what Undo says', async () => {
  await focused(true);
  await item('Bencompare').click();
  await win.click('[data-reader="delete"]');
  await expect(toastEl().locator('button')).toHaveText('Undo');
  await win.click('[data-view="sent"]');
  await expect(win.locator('.list-title h1')).toHaveText('Sent');
  await arrive('Lunch on Friday?');
  await expect.poll(async () => (await heard()).length).toBe(1);
  await expect(win.locator('.nav-item[data-view="inbox"] .count')).toHaveText(/\d+/);
  await expect(toastEl().locator('button')).toHaveText('Undo');
  await toastEl().locator('button').click();
  await expect(toastEl()).toContainText('Restored');
  await expect(toastEl()).toContainText('New email from Sanne de Vries: Lunch on Friday?', { timeout: 8000 });
});

test('the sound for new mail is off by default, a switch in Settings, and plays once for mail that comes in together', async () => {
  await focused(false);
  await arrive('Lunch on Friday?');
  await expect.poll(async () => (await heard()).length).toBe(1);
  expect((await heard())[0].sound).toBe(false);
  expect(await coos()).toBe(0);

  await win.click('[data-action="settings"]');
  const sound = settings().locator('[data-a="toggle:newMailSound"]');
  await expect(sound).toContainText('Sound for new mail');
  await expect(sound).toContainText('Play a short coo when new emails arrive.');
  await expect(sound.locator('.switch')).toHaveAttribute('aria-checked', 'false');
  await sound.click();
  await expect(sound.locator('.switch')).toHaveAttribute('aria-checked', 'true');
  await settings().locator('[data-a="back"]').click();
  await expect(settings()).toHaveCount(0);

  await arrive('Tickets for Sunday');
  await expect.poll(coos).toBe(1);
  expect((await heard()).at(-1).sound).toBe(true);
  // Rukoo's sound plays instead of the system's.
  await expect.poll(() => app.evaluate(() => global.__shown.length)).toBe(2);
  expect((await app.evaluate(() => global.__shown)).map((n) => n.silent)).toEqual([false, true]);

  // Two accounts' mail in one sync arrives as two batches right after each other: one sound at most.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const m = (id) => ({ id, subject: 'Same sync', from: { name: 'Demo', address: 'demo@example.com' } });
    engine.emit('new-mail', [m('demo:together-1')]);
    engine.emit('new-mail', [m('demo:together-2')]);
  });
  await expect.poll(async () => (await heard()).length).toBe(4);
  expect((await heard())[3].sound).toBe(false);
});

test('Settings sends a test notification, also with the window in front, and says what came of it', async () => {
  await focused(true);
  await win.click('[data-action="settings"]');
  const row = settings().locator('[data-a="notify-test"]');
  await expect(row.locator('.title')).toHaveText('Send a test notification');
  await expect(row.locator('.desc')).toHaveText('Shows one now, also while Rukoo is in front, so you can check that notifications reach you.');

  await app.evaluate(({ Notification }) => {
    global.__tests = [];
    Notification.prototype.show = function () {
      global.__tests.push({ title: this.title, body: this.body });
      this.emit('show');
    };
  });
  await row.click();
  await expect(row.locator('.desc')).toHaveText("Sent. Didn't see it? Check that your system allows notifications from Rukoo Mail.");
  expect(await app.evaluate(() => global.__tests)).toEqual([{ title: 'Test notification from Rukoo Mail', body: 'This is how new mail shows up.' }]);

  // Windows refuses it and says why.
  await app.evaluate(({ Notification }) => {
    Notification.prototype.show = function () {
      this.emit('failed', {}, 'Notifications are turned off for Rukoo Mail');
    };
  });
  await row.click();
  await expect(row.locator('.desc')).toHaveText("The system didn't show it (Notifications are turned off for Rukoo Mail). Check that your system allows notifications from Rukoo Mail.");

  // A system without notifications says so before and after a test.
  await app.evaluate(({ Notification }) => {
    Notification.isSupported = () => false;
  });
  await settings().locator('[data-a="back"]').click();
  await win.click('[data-action="settings"]');
  await expect(row.locator('.desc')).toHaveText("This computer doesn't offer notifications to Rukoo.");
  await row.click();
  await expect(row.locator('.desc')).toHaveText("This computer doesn't offer notifications to Rukoo.");
});
