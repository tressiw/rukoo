// Inbox categories: turned on in Settings, the categories under Inbox in the sidebar, moving an email and its sender, and editing,
// reordering and renaming categories, and their notifications. The real app with a throwaway profile, the demo account
// and no Rukoo account.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

// Starts Rukoo on dataDir: the first time with no account yet, again for a restart.
async function launch() {
  // No account server: categories work without one, on this computer.
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1', RUKOO_ACCOUNT_SERVER: 'http://127.0.0.1:9' };
  delete env.ELECTRON_RUN_AS_NODE;
  // The TypeSafe key is stored with safeStorage, which needs the real keyring on Linux (see the IMAP tests).
  const entry = process.platform === 'linux' ? path.join(__dirname, '..', 'fixtures', 'linux-keyring-app.js') : '.';
  app = await electron.launch({ args: [entry], cwd: path.join(__dirname, '..', '..'), env });
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
}

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-categories-e2e-');
  await launch();
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
});

test.afterEach(async () => {
  await closeApp(app);
  removeDir(dataDir);
});

const settings = () => win.locator('.page.settings');
// The categories under Inbox in the sidebar, and Inbox itself for all of it.
const tabs = () => win.locator('.nav-item.nav-sub .label');
const inboxRow = () => win.locator('.nav-item[data-view="inbox"]:not(.nav-sub)');

async function turnOn() {
  await win.click('[data-action="settings"]');
  await expect(settings().locator('[data-a="categories"] .value')).toHaveText('Off');
  await settings().locator('[data-a="categories"]').click();
  await expect(settings().locator('h1')).toHaveText('Inbox categories');
  await settings().locator('[data-a="cat-toggle"]').click();
  await expect(settings().locator('.cat-row')).toHaveCount(5);
}

async function closeSettings() {
  while (await settings().count()) await settings().locator('[data-a="back"]').click();
}

test('turning categories on puts them under Inbox, and each shows its mail', async () => {
  await expect(win.locator('.nav-item.nav-sub')).toHaveCount(0);
  await turnOn();
  await expect(settings().locator('.settings-text').first()).toContainText('These stay on this computer');
  await closeSettings();

  await expect(tabs()).toHaveText(['Needs you', 'To read', 'Money', 'Updates', 'Promotions']);
  await win.locator('.nav-item.nav-sub', { hasText: 'Promotions' }).click();
  await expect(win.locator('.nav-item.nav-sub.active')).toContainText('Promotions');
  await expect(win.locator('.list-title h1')).toHaveText('Promotions');
  const promotions = win.locator('.list-scroll .item');
  await expect(promotions.filter({ hasText: 'ANWB Newsletter' })).toHaveCount(1);
  await expect(promotions.filter({ hasText: 'Forward Future' })).toHaveCount(1);
  await expect(promotions.filter({ hasText: 'Sanne de Vries' })).toHaveCount(0);

  // A selection stays with its category: switching clears it, so no action reaches emails out of sight.
  await win.locator('.list-scroll .item', { hasText: 'Forward Future' }).click({ modifiers: ['Control'] });
  await expect(win.locator('.list-tools.selecting')).toBeVisible();
  await win.locator('.nav-item.nav-sub', { hasText: 'To read' }).click();
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await expect(win.locator('.list-scroll .empty')).toContainText('Nothing in To read');
  await inboxRow().click();
  await expect(win.locator('.list-scroll .item').filter({ hasText: 'Sanne de Vries' })).toHaveCount(1);

  // A category picked during a search ends the search and shows that category's mail.
  await win.fill('#search', 'contract');
  await expect(win.locator('.list-scroll .item').filter({ hasText: 'ANWB Newsletter' })).toHaveCount(0);
  await win.locator('.nav-item.nav-sub', { hasText: 'Promotions' }).click();
  await expect(win.locator('#search')).toHaveValue('');
  await expect(win.locator('.list-title h1')).toHaveText('Promotions');
  await expect(win.locator('.list-scroll .item').filter({ hasText: 'ANWB Newsletter' })).toHaveCount(1);
  await expect(win.locator('.list-scroll .item').filter({ hasText: 'Sanne de Vries' })).toHaveCount(0);

  // Nothing moved on the server: the demo inbox holds the same messages.
  const inbox = await win.evaluate(() => window.mail.call('list', { scope: 'all', view: 'inbox' }).then((l) => l.length));
  expect(inbox).toBeGreaterThan(10);
});

test('Inbox shows each email its category, and a category tab wears its color', async () => {
  const pill = (sender) => win.locator('.list-scroll .item', { hasText: sender }).first().locator('.pill.cat');
  // Off: no categories, so nothing to show.
  await expect(win.locator('.list-scroll .pill.cat')).toHaveCount(0);
  await turnOn();
  await closeSettings();

  await expect(pill('ANWB Newsletter')).toHaveText('Promotions');
  await expect(pill('Sanne de Vries')).toHaveText('Needs you');
  await expect(pill('ANWB Newsletter')).toHaveAttribute('style', '--cat:#c56cf0');
  await expect(win.locator('.list-head')).not.toHaveClass(/cat-head/);

  // In its own tab the header says which category it is, so the rows leave it out.
  await win.locator('.nav-item.nav-sub', { hasText: 'Promotions' }).click();
  await expect(win.locator('.list-head')).toHaveClass(/cat-head/);
  await expect(win.locator('.list-head')).toHaveCSS('--cat', '#c56cf0');
  await expect(win.locator('.list-scroll .item').first()).toBeVisible();
  await expect(win.locator('.list-scroll .pill.cat')).toHaveCount(0);

  // Back in Inbox the header is plain again.
  await inboxRow().click();
  await expect(win.locator('.list-head')).not.toHaveClass(/cat-head/);
  await expect(win.locator('.list-head')).toHaveCSS('--cat', '');
  await expect(pill('ANWB Newsletter')).toHaveText('Promotions');
});

test('with several accounts the account is a dot on the picture, and a narrow list with folder and category labels still shows the subject', async () => {
  await turnOn();
  await closeSettings();
  // A long name, so the category's label needs all the room it may take.
  await win.evaluate(() => window.mail.call('categoriesSave', { id: 'needs-you', name: 'Needs you today or tomorrow' }));
  // A second account, on a server these tests never reach, so a search over everything shows the account too.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const server = { host: '127.0.0.1', port: 1, secure: false, user: 'werk@example.com' };
    engine.accounts.push({ id: 'w1', type: 'imap', provider: 'other', email: 'werk@example.com', name: 'Werk with a long display name', color: '#ff8a3d', signature: null, secret: 'plain:', imap: server, smtp: server, createdAt: Date.now() });
    engine.caches.set('w1', { folders: [], boxes: {}, lastSync: Date.now() });
  });
  await win.reload();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await win.fill('#search', 'contract');
  await win.click('.search-scope');
  await win.click('.menu button:has-text("All accounts")');
  const row = win.locator('.list-scroll .item', { hasText: 'Vandebron' }).first();
  await expect(row.locator('.pill.cat')).toHaveText('Needs you today or tomorrow');
  // The account isn't a label beside the category's, but a dot in its color on the sender's picture.
  const dot = row.locator('.lead .acc-dot');
  await expect(dot).toHaveAttribute('aria-label', 'Account: demo');
  await expect(dot).toHaveAttribute('title', 'demo@example.com');
  await expect(row.locator('.pill.acc')).toHaveCount(0);
  await expect(row.locator('.flags .pill')).toHaveCount(2);

  // The narrowest list there is, made by keyboard on the divider as a user would, so a later layout pass keeps it.
  await win.locator('.divider').focus();
  for (let i = 0; i < 5; i++) await win.keyboard.press('ArrowLeft');
  await expect.poll(() => win.locator('.listpane').evaluate((el) => el.getBoundingClientRect().width)).toBe(300);
  const box = await row.evaluate((el) => {
    const line = el.querySelector('.line2').getBoundingClientRect();
    return {
      subject: el.querySelector('.subject').getBoundingClientRect().width,
      flagsEnd: el.querySelector('.flags').getBoundingClientRect().right,
      lineEnd: line.right
    };
  });
  expect(box.subject).toBeGreaterThan(40);
  expect(box.flagsEnd).toBeLessThanOrEqual(box.lineEnd + 0.5);
});

test('an email moves to another category, and its sender follows on request', async () => {
  await turnOn();
  await closeSettings();
  await win.locator('.nav-item.nav-sub', { hasText: 'Promotions' }).click();
  await win.locator('.list-scroll .item', { hasText: 'ANWB Newsletter' }).click({ button: 'right' });
  await win.locator('.menu button', { hasText: 'Move to category' }).click();
  await win.locator('.scrim .radio-row', { hasText: 'To read' }).click();
  await expect(win.locator('#toast')).toContainText('Moved to To read');
  await expect(win.locator('.list-scroll .item', { hasText: 'ANWB Newsletter' })).toHaveCount(0);
  // Ctrl+Z undoes things elsewhere; it doesn't send the sender's future mail there too.
  await win.locator('.list-scroll').focus();
  await win.keyboard.press(process.platform === 'darwin' ? 'Meta+z' : 'Control+z');
  await expect(win.locator('#toast')).toContainText('Moved to To read');
  await expect(win.locator('#toast')).toHaveClass(/show/);
  await win.locator('#toast button', { hasText: 'Also future mail' }).click();
  await expect(win.locator('#toast')).toContainText('Mail from ANWB Newsletter goes to To read from now on');

  await win.locator('.nav-item.nav-sub', { hasText: 'To read' }).click();
  await expect(win.locator('.list-scroll .item', { hasText: 'ANWB Newsletter' })).toHaveCount(1);

  // Several selected and moved away: the selection doesn't keep them out of sight.
  await win.locator('.nav-item.nav-sub', { hasText: 'Promotions' }).click();
  await win.locator('.list-scroll .item', { hasText: 'Forward Future' }).click({ modifiers: ['Control'] });
  await win.locator('[data-bulk="more"]').click();
  await win.locator('.menu button', { hasText: 'Move to category' }).click();
  await win.locator('.scrim .radio-row', { hasText: 'Updates' }).click();
  await expect(win.locator('.list-scroll .item', { hasText: 'Forward Future' })).toHaveCount(0);
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await win.locator('.nav-item.nav-sub', { hasText: 'To read' }).click();

  await win.click('[data-action="settings"]');
  await settings().locator('[data-a="categories"]').click();
  const rule = settings().locator('.row', { hasText: 'newsletter@anwb.nl' });
  await expect(rule).toContainText('Goes to To read');
  await rule.click();
  await win.locator('.scrim .buttons button', { hasText: 'Delete' }).click();
  await expect(settings().locator('.row', { hasText: 'newsletter@anwb.nl' })).toHaveCount(0);
});

test('moving a selection to another category clears it from the reader too', async () => {
  await turnOn();
  await closeSettings();
  await win.locator('.nav-item.nav-sub', { hasText: 'Promotions' }).click();
  await win.locator('.list-scroll .item', { hasText: 'ANWB Newsletter' }).click({ modifiers: ['Control'] });
  await win.locator('.list-scroll .item', { hasText: 'Forward Future' }).click({ modifiers: ['Control'] });
  await expect(win.locator('.reader .multi')).toBeVisible();
  await win.locator('[data-bulk="more"]').click();
  await win.locator('.menu button', { hasText: 'Move to category' }).click();
  await win.locator('.scrim .radio-row', { hasText: 'Updates' }).click();
  await expect(win.locator('.list-scroll .empty')).toContainText('Nothing in Promotions');
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await expect(win.locator('.reader .multi')).toHaveCount(0);
});

test('categories can be renamed, added, moved up and kept after a restart', async () => {
  await turnOn();
  await settings().locator('[data-a="cat-edit:money"]').click();
  const dialog = win.locator('.scrim .dialog');
  await dialog.locator('[name="name"]').fill('Finance');
  await dialog.locator('.cat-swatch').nth(4).click();
  await dialog.locator('.buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-edit:money"]')).toContainText('Finance');

  await settings().locator('[data-a="cat-add"]').click();
  await dialog.locator('[name="name"]').fill('Kids');
  await dialog.locator('[name="description"]').fill('School, sports clubs and birthday parties');
  await dialog.locator('.buttons button', { hasText: 'Add' }).click();
  await expect(settings().locator('.cat-row')).toHaveCount(6);
  await expect(settings().locator('.cat-row').last()).toContainText('Kids');

  await settings().locator('[data-a="cat-up:promotions"]').click();
  await expect(settings().locator('.cat-row').nth(3)).toContainText('Promotions');

  await settings().locator('[data-a="cat-unsure"]').click();
  await win.locator('.scrim .radio-row', { hasText: 'Kids' }).click();
  await expect(settings().locator('[data-a="cat-unsure"] .value')).toHaveText('Kids');
  await closeSettings();
  await expect(tabs()).toHaveText(['Needs you', 'To read', 'Finance', 'Promotions', 'Updates', 'Kids']);

  await win.reload();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await expect(tabs()).toHaveText(['Needs you', 'To read', 'Finance', 'Promotions', 'Updates', 'Kids']);
});

test("a category's notifications turn off on this computer and stay off after a restart", async () => {
  await turnOn();
  // The window after the restart is another one.
  const dialog = () => win.locator('.scrim .dialog');
  const notify = () => dialog().locator('.cat-notify');
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  await expect(notify()).toHaveAttribute('aria-checked', 'true');
  await expect(dialog()).toContainText('This choice stays on this computer.');
  await notify().click();
  await expect(notify()).toHaveAttribute('aria-checked', 'false');
  await dialog().locator('.buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-edit:promotions"] .cat-muted')).toHaveAttribute('aria-label', 'Notifications off');
  await expect(settings().locator('.cat-muted')).toHaveCount(1);

  // A new category can start with them off.
  await settings().locator('[data-a="cat-add"]').click();
  await dialog().locator('[name="name"]').fill('Kids');
  await notify().click();
  await dialog().locator('.buttons button', { hasText: 'Add' }).click();
  await expect(settings().locator('.cat-row').last()).toContainText('Kids');
  await expect(settings().locator('.cat-row').last().locator('.cat-muted')).toHaveCount(1);
  // Kept on this computer, beside the Jev key, not in the categories' synced records.
  const local = () => JSON.parse(fs.readFileSync(path.join(dataDir, 'data', 'categories.json'), 'utf8')).muted;
  expect(local()).toHaveLength(2);
  expect(local()).toContain('promotions');

  await closeApp(app);
  await launch();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await win.click('[data-action="settings"]');
  await settings().locator('[data-a="categories"]').click();
  await expect(settings().locator('.cat-muted')).toHaveCount(2);
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  await expect(notify()).toHaveAttribute('aria-checked', 'false');
  await notify().click();
  await dialog().locator('.buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-edit:promotions"] .cat-muted')).toHaveCount(0);
  expect(local()).toHaveLength(1);

  // "Use Rukoo's text" keeps a change to the switch.
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  await dialog().locator('[name="name"]').fill('Offers');
  await dialog().locator('.buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-edit:promotions"]')).toContainText('Offers');
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  await notify().click();
  await dialog().locator('.buttons button', { hasText: "Use Rukoo's text" }).click();
  await expect(settings().locator('[data-a="cat-edit:promotions"] .title')).toHaveText('Promotions');
  await expect(settings().locator('[data-a="cat-edit:promotions"] .cat-muted')).toHaveCount(1);
});

test('with notifications off for all mail, the category switch stays as it is and says why', async () => {
  await turnOn();
  const dialog = () => win.locator('.scrim .dialog');
  const notify = () => dialog().locator('.cat-notify');
  await win.evaluate(() => window.mail.call('updateSettings', { notifications: false }));
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  await expect(notify()).toBeDisabled();
  await expect(notify()).toHaveAttribute('aria-checked', 'true');
  await expect(dialog().locator('#cat-notify-help')).toHaveText('Notifications are off for all new mail, so this category stays quiet too. Turn on Notifications in Settings first.');
  await notify().click({ force: true });
  await expect(notify()).toHaveAttribute('aria-checked', 'true');
  await dialog().locator('.buttons button', { hasText: 'Cancel' }).click();

  // On again, the switch works as before.
  await win.evaluate(() => window.mail.call('updateSettings', { notifications: true }));
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  await expect(notify()).toBeEnabled();
  await expect(dialog().locator('#cat-notify-help')).toContainText('This choice stays on this computer.');
});

test('new mail in a muted category sends no notification', async () => {
  await turnOn();
  await settings().locator('[data-a="cat-edit:updates"]').click();
  await win.locator('.scrim .dialog .cat-notify').click();
  await win.locator('.scrim .dialog .buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-edit:updates"] .cat-muted')).toHaveCount(1);
  await closeSettings();
  // The window hears only of the mail that notifies, so a muted category gets no toast, pill or sound there either.
  await win.evaluate(() => {
    window.__newMail = [];
    window.mail.on(({ type, payload }) => type === 'new-mail' && window.__newMail.push(...payload.mails.map((m) => m.subject)));
  });
  const shown = await app.evaluate(async ({ Notification, BrowserWindow }) => {
    // Notifications are recorded instead of shown, as for a window in the background.
    BrowserWindow.getAllWindows()[0].isFocused = () => false;
    global.__shown = [];
    Notification.prototype.show = function () {
      global.__shown.push(this);
    };
    const engine = global.__semEngine;
    const raw = (from, subject, n) =>
      [`From: ${from}`, 'To: demo@example.com', `Subject: ${subject}`, `Date: ${new Date().toUTCString()}`, `Message-ID: <${Date.now()}.${n}@example.com>`, '', 'Hello'].join('\r\n');
    // A sign-in alert, which the rules put in Updates without a doubt, and an email from a person.
    await engine.session(engine.accounts[0]).append('INBOX', raw('App <noreply@app.example>', 'New sign-in', 1));
    await engine.session(engine.accounts[0]).append('INBOX', raw('Sanne de Vries <sanne@example.com>', 'Lunch on Friday?', 2));
    await engine.syncAll();
    return global.__shown.map((n) => n.title);
  });
  expect(shown).toEqual(['Sanne de Vries']);
  await expect.poll(() => win.evaluate(() => window.__newMail)).toEqual(['Lunch on Friday?']);
  await expect(win.locator('.item', { hasText: 'New sign-in' })).toHaveCount(1);
});

test('new mail in Inbox and in a category Inbox leaves out: the list shows the one, a toast the other, and Show opens the category', async () => {
  await turnOn();
  for (const id of ['updates', 'promotions']) {
    await settings().locator(`[data-a="cat-edit:${id}"]`).click();
    await win.locator('.scrim .dialog [role="switch"]', { hasText: 'Show in Inbox' }).click();
    await win.locator('.scrim .dialog .buttons button', { hasText: 'Save' }).click();
    await expect(settings().locator(`[data-a="cat-edit:${id}"] .cat-off`)).toHaveCount(1);
  }
  await closeSettings();
  await expect(win.locator('.list-title h1')).toHaveText('Inbox');
  // In front, where a toast stands in for the desktop notification. Those are recorded, should one come.
  await app.evaluate(({ Notification, BrowserWindow }) => {
    BrowserWindow.getAllWindows()[0].isFocused = () => true;
    global.__shown = [];
    Notification.prototype.show = function () {
      global.__shown.push(this.title);
    };
  });
  // Emails land in the demo inbox, and one sync finds them all.
  const arrive = (mails) =>
    app.evaluate(async (_e, mails) => {
      const engine = global.__semEngine;
      for (const [n, [from, subject]] of mails.entries()) {
        const raw = [`From: ${from}`, 'To: demo@example.com', `Subject: ${subject}`, `Date: ${new Date().toUTCString()}`, `Message-ID: <${Date.now()}.${n}@example.com>`, '', 'Hello'];
        await engine.session(engine.accounts[0]).append('INBOX', raw.join('\r\n'));
      }
      await engine.syncAll();
    }, mails);
  const items = () => win.locator('.list-scroll .item');
  const toastEl = () => win.locator('#toast');

  // Sanne's email is at the top of Inbox; the sign-in alert, which the rules put in Updates, gets the toast.
  await arrive([
    ['Sanne de Vries <sanne@example.com>', 'Lunch on Friday?'],
    ['App <noreply@app.example>', 'New sign-in']
  ]);
  await expect(toastEl()).toContainText('New email from App: New sign-in');
  await expect(items().filter({ hasText: 'Lunch on Friday?' })).toBeInViewport();
  await expect(items().filter({ hasText: 'New sign-in' })).toHaveCount(0);
  await expect(win.locator('.new-pill')).toBeHidden();
  await toastEl().locator('button', { hasText: 'Open' }).click();
  await expect(win.locator('.list-title h1')).toHaveText('Updates');
  await expect(win.locator('.reader-subject')).toHaveText('New sign-in');

  // Two alerts for Updates while Inbox is on screen: the toast says where they are, and Show goes there.
  await inboxRow().click();
  await expect(win.locator('.list-title h1')).toHaveText('Inbox');
  await arrive([
    ['App <noreply@app.example>', 'Your code is 123456'],
    ['Bank <alerts@bank.example>', 'Card used abroad']
  ]);
  await expect(toastEl()).toContainText('2 new emails in Updates');
  await toastEl().locator('button', { hasText: 'Show' }).click();
  await expect(win.locator('.list-title h1')).toHaveText('Updates');
  await expect(win.locator('.nav-item.nav-sub.active')).toContainText('Updates');
  await expect(items().filter({ hasText: 'Your code is 123456' })).toHaveCount(1);
  await expect(items().filter({ hasText: 'Card used abroad' })).toHaveCount(1);

  // One for Updates and one for Promotions, both kept out of Inbox: the toast counts them without naming either, and
  // Show goes to Updates, the higher of the two in the sidebar.
  await inboxRow().click();
  await expect(win.locator('.list-title h1')).toHaveText('Inbox');
  await arrive([
    ['Shop <newsletter@shop.example>', 'Twenty percent off this weekend'],
    ['App <noreply@app.example>', 'Your password was changed']
  ]);
  await expect(toastEl().locator('span')).toHaveText('2 new emails');
  await toastEl().locator('button', { hasText: 'Show' }).click();
  await expect(win.locator('.nav-item.nav-sub.active')).toContainText('Updates');
  await expect(items().filter({ hasText: 'Your password was changed' })).toHaveCount(1);
  expect(await app.evaluate(() => global.__shown)).toEqual([]);
});

test('Jev needs a key and says what it shares', async () => {
  await turnOn();
  await settings().locator('[data-a="cat-jev"]').click();
  await expect(win.locator('#toast')).toContainText('Add your TypeSafe API key first.');
  const dialog = win.locator('.scrim .dialog');
  await expect(dialog).toContainText('TypeSafe, Inc. in the United States');
  await dialog.locator('input').fill('not-a-real-key');
  await dialog.locator('.buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-key"] .value')).toHaveText('Set on this computer');
  await expect(settings().locator('[data-a="cat-jev"] .switch')).toHaveClass(/on/);

  // The account's privacy page names TypeSafe while Jev is on.
  await settings().locator('[data-a="back"]').click();
  await settings().locator('[data-a="rukoo"]').click();
  await settings().locator('[data-a="rukoo-go:rukoo-privacy"]').click();
  await expect(settings().locator('.row', { hasText: 'TypeSafe, Inc.' })).toContainText('Sorts your inbox with Jev');
});

// The count on a row of the sidebar, 0 when it shows none.
async function countOn(row) {
  const count = row.locator('.count');
  return (await count.count()) ? Number(await count.textContent()) : 0;
}

test('a category kept out of Inbox leaves Inbox and its counts, keeps its own mail, and stays so after a restart', async () => {
  await turnOn();
  await closeSettings();
  const promotionsRow = () => win.locator('.nav-item.nav-sub', { hasText: 'Promotions' });
  const items = () => win.locator('.list-scroll .item');
  // One of its emails is open when the category leaves Inbox.
  await items().filter({ hasText: 'ANWB Newsletter' }).first().click();
  await expect(win.locator('.reader-subject')).toBeVisible();
  await expect.poll(() => app.evaluate(() => global.__semEngine.listMessages({ view: 'inbox' }).find((m) => m.from.name === 'ANWB Newsletter').unread)).toBe(false);
  const inboxUnread = () => app.evaluate(() => global.__semEngine.counts('all').views.inbox);
  await expect.poll(async () => (await countOn(inboxRow())) === (await inboxUnread())).toBe(true);
  await expect.poll(() => countOn(promotionsRow())).toBeGreaterThan(0);
  const all = await countOn(inboxRow());
  const promotions = await countOn(promotionsRow());
  expect(all).toBeGreaterThan(promotions);

  // The switch is in the category's dialog, on until it is turned off.
  await win.click('[data-action="settings"]');
  await settings().locator('[data-a="categories"]').click();
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  const dialog = () => win.locator('.scrim .dialog');
  const sw = () => dialog().locator('[role="switch"]', { hasText: 'Show in Inbox' });
  await expect(sw()).toHaveAttribute('aria-checked', 'true');
  await sw().click();
  await expect(sw()).toHaveAttribute('aria-checked', 'false');
  await dialog().locator('.buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-edit:promotions"] .cat-off')).toHaveAttribute('aria-label', 'Not shown in Inbox');
  await expect(settings().locator('[data-a="cat-edit:money"] .cat-off')).toHaveCount(0);
  await closeSettings();

  // Inbox leaves its mail out, also from the reading pane, and the counts follow; its own row still has all of it,
  // marked as not in Inbox.
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(0);
  await expect(win.locator('.reader-subject')).toHaveCount(0);
  await expect(items().filter({ hasText: 'Forward Future' })).toHaveCount(0);
  await expect(items().filter({ hasText: 'Sanne de Vries' })).toHaveCount(1);
  await expect.poll(() => countOn(inboxRow())).toBe(all - promotions);
  await expect(win.locator('.list-title .sub')).toContainText(`${all - promotions} unread`);
  await expect(promotionsRow().locator('.cat-off')).toBeVisible();
  await expect(promotionsRow()).toHaveAttribute('title', "Promotions isn't shown in Inbox");
  expect(await countOn(promotionsRow())).toBe(promotions);
  await promotionsRow().click();
  await expect(win.locator('.list-title h1')).toHaveText('Promotions');
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(1);

  // The badge and the tray: unread mail, and the default count of new mail since the window had focus.
  expect(await app.evaluate(() => global.__semEngine.unreadCount())).toBe(all - promotions);
  if (await app.evaluate(() => Boolean(global.__semTray))) await expect.poll(() => app.evaluate(() => global.__semTray.unread)).toBe(all - promotions);
  await win.evaluate(() => {
    window.__badges = [];
    window.mail.on(({ type, payload }) => type === 'badge' && window.__badges.push(payload));
  });
  const arrived = await app.evaluate(() => {
    const engine = global.__semEngine;
    // No notifications on the screen during the test.
    engine.settings.notifications = false;
    const unread = engine.listMessages({ view: 'inbox' }).filter((m) => m.unread);
    const [a, b] = unread.filter((m) => m.category === 'needs-you');
    for (const m of [a, unread.find((x) => x.category === 'promotions'), b]) engine.emit('new-mail', [m]);
    return a.id;
  });
  await expect.poll(() => win.evaluate(() => window.__badges.length)).toBe(3);
  const [first, hidden, second] = await win.evaluate(() => window.__badges);
  expect(hidden).toBe(first);
  expect(second).toBe(first + 1);
  // New mail put in a hidden category after it came in stops counting, and counts again when it comes back.
  const lastBadge = () => win.evaluate(() => window.__badges[window.__badges.length - 1]);
  await app.evaluate((_e, id) => global.__semCategories.move([id], 'promotions'), arrived);
  await expect.poll(lastBadge).toBe(second - 1);
  await app.evaluate((_e, id) => global.__semCategories.move([id], 'needs-you'), arrived);
  await expect.poll(lastBadge).toBe(second);

  // A click on a notification for it opens it in its own category, where its row is.
  await inboxRow().click();
  const anwb = await app.evaluate(() => global.__semEngine.listMessages({ view: 'inbox' }).find((m) => m.from.name === 'ANWB Newsletter'));
  await app.evaluate(({ BrowserWindow }, id) => BrowserWindow.getAllWindows()[0].webContents.send('mail:event', { type: 'open-message', payload: id }), anwb.id);
  await expect(win.locator('.nav-item.nav-sub.active')).toContainText('Promotions');
  await expect(items().filter({ hasText: 'ANWB Newsletter' }).first()).toHaveClass(/selected/);
  // It stays open when the list is fetched again.
  await app.evaluate(() => global.__semEngine.emit('updated'));
  await win.waitForTimeout(500);
  await expect(win.locator('.reader-subject')).toBeVisible();
  // Opening it read it.
  const promotionsLeft = promotions - (anwb.unread ? 1 : 0);
  await expect.poll(() => countOn(promotionsRow())).toBe(promotionsLeft);

  // A search still finds it, in Inbox too.
  await inboxRow().click();
  await win.fill('#search', 'ANWB');
  await expect(items().filter({ hasText: 'ANWB Newsletter' }).first()).toBeVisible();
  await win.click('[data-action="search-clear"]');
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(0);

  // All accounts leaves it out too, and so does the account menu's count.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const server = { host: '127.0.0.1', port: 1, secure: false, user: 'werk@example.com' };
    engine.accounts.push({ id: 'w1', type: 'imap', provider: 'other', email: 'werk@example.com', name: 'Werk', label: 'werk@example.com', color: '#ff8a3d', signature: null, secret: 'plain:', imap: server, smtp: server, createdAt: Date.now() });
    engine.caches.set('w1', { folders: [], boxes: {}, lastSync: Date.now() });
    engine.emit('updated');
  });
  await expect(async () => {
    await win.click('[data-action="accounts"]');
    await expect(win.locator('.menu button', { hasText: 'All accounts' })).toBeVisible({ timeout: 500 });
  }).toPass();
  await expect(win.locator('.menu button', { hasText: 'demo@example.com' }).locator('.mh')).toHaveText(String(all - promotions));
  await expect(win.locator('.menu button', { hasText: 'All accounts' }).locator('.mh')).toHaveText(String(all - promotions));
  await win.locator('.menu button', { hasText: 'All accounts' }).click();
  await expect(win.locator('.account-switch')).toContainText('All accounts');
  await expect(items().filter({ hasText: 'Sanne de Vries' })).toHaveCount(1);
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(0);
  await expect.poll(() => countOn(inboxRow())).toBe(all - promotions);

  // Mark all as read in Inbox marks what Inbox lists, not the mail it leaves out.
  await win.click('[data-action="list-more"]');
  await win.locator('.menu button', { hasText: 'Mark all as read' }).click();
  await expect(win.locator('#toast')).toContainText(`${all - promotions}`);
  await expect.poll(() => countOn(inboxRow())).toBe(0);
  expect(promotionsLeft).toBeGreaterThan(0);
  expect(await countOn(promotionsRow())).toBe(promotionsLeft);

  // After a restart, the category is still out of Inbox.
  await closeApp(app);
  await launch();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await expect(promotionsRow().locator('.cat-off')).toBeVisible();
  await expect(items().filter({ hasText: 'Sanne de Vries' })).toHaveCount(1);
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(0);
  await win.click('[data-action="settings"]');
  await settings().locator('[data-a="categories"]').click();
  await settings().locator('[data-a="cat-edit:promotions"]').click();
  await expect(sw()).toHaveAttribute('aria-checked', 'false');
  // Turned on again, the mail is back in Inbox.
  await sw().click();
  await dialog().locator('.buttons button', { hasText: 'Save' }).click();
  await expect(settings().locator('[data-a="cat-edit:promotions"] .cat-off')).toHaveCount(0);
  await closeSettings();
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(1);
  await expect(promotionsRow().locator('.cat-off')).toHaveCount(0);
});

test('an email open in the reading pane leaves it when Inbox stops listing it, but an editor stays', async () => {
  await turnOn();
  await closeSettings();
  const items = () => win.locator('.list-scroll .item');
  const row = (sender) => items().filter({ hasText: sender }).first();
  const reader = () => win.locator('.reader-subject');
  const showInInbox = (id, inbox) => win.evaluate(([id, inbox]) => window.mail.call('categoriesSave', { id, inbox }), [id, inbox]);

  // Open and checked: its row, the selection and the reading pane all let it go.
  await row('ANWB Newsletter').click();
  await expect(reader()).toBeVisible();
  await row('ANWB Newsletter').click({ modifiers: ['Control'] });
  await expect(win.locator('.list-tools.selecting')).toBeVisible();
  await showInInbox('promotions', false);
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(0);
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await expect(reader()).toHaveCount(0);

  // Moved into a category Inbox already leaves out.
  await row('Bencompare').click();
  await expect(reader()).toBeVisible();
  await row('Bencompare').click({ button: 'right' });
  await win.locator('.menu button', { hasText: 'Move to category' }).click();
  await win.locator('.scrim .radio-row', { hasText: 'Promotions' }).click();
  await expect(items().filter({ hasText: 'Bencompare' })).toHaveCount(0);
  await expect(reader()).toHaveCount(0);

  // Categories turned on again, with Promotions still kept out of Inbox.
  await win.evaluate(() => window.mail.call('categoriesSetEnabled', false));
  await expect(win.locator('.nav-item.nav-sub')).toHaveCount(0);
  await row('ANWB Newsletter').click();
  await expect(reader()).toBeVisible();
  await win.evaluate(() => window.mail.call('categoriesSetEnabled', true));
  await expect(win.locator('.nav-item.nav-sub')).toHaveCount(5);
  await expect(items().filter({ hasText: 'ANWB Newsletter' })).toHaveCount(0);
  await expect(reader()).toHaveCount(0);

  // A reply in the reading pane stays when its email's category leaves Inbox.
  await row('Sanne de Vries').click();
  await win.click('[data-reader="reply"]');
  await expect(win.locator('.composer .editor')).toBeVisible({ timeout: 15000 });
  await showInInbox('needs-you', false);
  await expect(items().filter({ hasText: 'Sanne de Vries' })).toHaveCount(0);
  await expect(win.locator('.composer .editor')).toBeVisible();
  // Closing that reply doesn't bring back the email Inbox no longer lists, not even until the next refresh.
  await win.evaluate(() => {
    window.__readerBack = false;
    new MutationObserver(() => {
      if (document.querySelector('.reader-subject')) window.__readerBack = true;
    }).observe(document.body, { childList: true, subtree: true });
  });
  await win.click('.composer [data-c="close"]');
  await expect(win.locator('.composer')).toHaveCount(0);
  expect(await win.evaluate(() => window.__readerBack)).toBe(false);
  await expect(reader()).toHaveCount(0);
});
