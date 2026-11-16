// mailto: links: a link Rukoo is started with or handed while it runs (as Windows does for the app the user picked),
// a link in a message, the compose window before any account exists, keeping what a link filled in when its editor
// closes, and Settings > Email links.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

const LINK =
  'mailto:anna@example.com,%22Doe%2C%20Jane%22%20%3Cjane@example.com%3E?cc=joris@example.com&bcc=sanne@example.com&subject=1+1%3D2&body=First%20line%0D%0ASecond%20line';

let app;
let dataDir;

async function launch(args = [], settings = {}) {
  fs.mkdirSync(path.join(dataDir, 'data'), { recursive: true });
  const file = path.join(dataDir, 'data', 'settings.json');
  const current = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8')) : {};
  fs.writeFileSync(file, JSON.stringify({ ...current, ...settings }));
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.', ...args], cwd: path.join(__dirname, '..', '..'), env });
  const win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  return win;
}

async function quit() {
  await closeApp(app);
  app = null;
}

// A profile with the demo account, ready for the next launch.
async function withDemo() {
  const win = await launch();
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  return win;
}

// The separate compose window, once it is there. Its question about unsaved changes comes from beforeunload, which
// Electron answers by itself (it keeps the window); without a listener of its own, Playwright answers it too.
async function composeWindow() {
  let page = null;
  await expect.poll(() => (page = app.windows().find((p) => p.url().endsWith('compose.html')) || null)).not.toBeNull();
  page.on('dialog', () => {});
  return page;
}

async function expectFilled(compose) {
  await compose.waitForSelector('.compose .editor');
  await expect(compose.locator('[data-rfield="to"] .recipient')).toHaveText(['anna@example.com', 'Doe, Jane']);
  await expect(compose.locator('[data-rfield="cc"] .recipient')).toHaveText(['joris@example.com']);
  await expect(compose.locator('[data-rfield="bcc"] .recipient')).toHaveText(['sanne@example.com']);
  await expect(compose.locator('[data-field="subject"]')).toHaveValue('1+1=2');
  expect(await compose.locator('.editor').evaluate((e) => e.innerText.trim())).toBe('First line\nSecond line');
}

const mainVisible = () =>
  app.evaluate(({ BrowserWindow }) => {
    const w = BrowserWindow.getAllWindows().find((x) => x.webContents.getURL().endsWith('index.html'));
    return Boolean(w && w.isVisible());
  });

// The compose window's own close button (or Cmd+W on a Mac): its beforeunload decides whether to ask.
const closeCompose = () =>
  app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().endsWith('compose.html')).close());
const composeOpen = () => app.windows().some((p) => p.url().endsWith('compose.html'));

// The draft a link left behind, opened again from Drafts in the main window's reading pane.
async function reopenDraft(win, subject) {
  await win.click('[data-view="drafts"]');
  await win.locator('.item', { hasText: subject }).first().click();
  await win.click('[data-reader="edit"]');
  await win.waitForSelector('.composer .editor');
}

// A newsletter that unsubscribes by email only, as mail in a real account can. The demo account unsubscribes in one
// click, so its messages are handed out with only this address, from an account that is not the demo.
const UNSUBSCRIBE = 'mailto:leave@anwb.example?subject=Unsubscribe%20me&body=Please%20remove%20me%20from%20this%20list.';
const unsubscribeByMail = () =>
  app.evaluate((_, link) => {
    const engine = global.__semEngine;
    const getMessage = engine.getMessage.bind(engine);
    const messageAccount = engine.messageAccount.bind(engine);
    engine.getMessage = async (...args) => {
      const m = await getMessage(...args);
      return m && m.unsubscribe ? { ...m, unsubscribe: { url: null, mail: link, oneClick: false } } : m;
    };
    engine.messageAccount = (id) => ({ ...messageAccount(id), type: 'imap' });
  }, UNSUBSCRIBE);

// What Windows does when Rukoo already runs: start it again with the link, which the first instance receives.
const startAgain = (args) => app.evaluate(({ app }, args) => app.emit('second-instance', {}, args, process.cwd()), args);

test.beforeEach(() => {
  dataDir = tempDir('sem-e2e-mailto-');
});

test.afterEach(async () => {
  await quit();
  removeDir(dataDir);
});

test('a link Rukoo is started with opens a filled-in compose window over the main window', async () => {
  await withDemo();
  await quit();
  await launch([LINK]);
  await expectFilled(await composeWindow());
  expect(await mainVisible()).toBe(true);
});

test('a link for a running Rukoo opens only its compose window, and a main window in the tray stays there', async () => {
  await withDemo();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].hide());
  await startAgain(['Rukoo Mail.exe', '--', LINK]);
  await expectFilled(await composeWindow());
  expect(await mainVisible()).toBe(false);
  // Started again without a link, Rukoo shows its main window as before.
  await startAgain(['Rukoo Mail.exe']);
  await expect.poll(mainVisible).toBe(true);
});

test('a link before any account is added says so, and becomes the message once there is one', async () => {
  const win = await launch();
  await win.waitForSelector('.provider-grid');
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].hide());
  await startAgain(['Rukoo Mail.exe', '--', LINK]);
  const compose = await composeWindow();
  await expect(compose.locator('[data-compose-empty]')).toHaveText('Add an email account in Rukoo Mail before you write an email.');
  await expect.poll(mainVisible).toBe(true);
  const titles = async (text) => {
    await expect(compose).toHaveTitle(text);
    await expect(compose.locator('#compose-title')).toHaveText(text);
  };
  await titles('New message');
  // It follows the language like the editor does, its title included.
  await win.evaluate(() => window.mail.call('updateSettings', { language: 'nl' }));
  await expect(compose.locator('[data-compose-empty]')).toHaveText('Voeg in Rukoo Mail eerst een e-mailaccount toe voordat je een e-mail schrijft.');
  await titles('Nieuw bericht');
  await win.evaluate(() => window.mail.call('updateSettings', { language: 'en' }));
  await expect(compose.locator('[data-compose-empty]')).toHaveText('Add an email account in Rukoo Mail before you write an email.');
  await titles('New message');
  // Add an account in the main window, come back: the link's message is ready to write.
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await compose.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expectFilled(compose);
});

test('the compose window for a link before any account can be closed', async () => {
  const win = await launch();
  await win.waitForSelector('.provider-grid');
  await startAgain(['Rukoo Mail.exe', '--', LINK]);
  const compose = await composeWindow();
  await compose.click('.compose-error button');
  await expect.poll(() => app.windows().some((p) => p.url().endsWith('compose.html'))).toBe(false);
});

test('a link still opens after the main window closed while a compose window stayed open', async () => {
  // The close button only hides the main window on a Mac; macos.spec.js covers closing it and links there.
  test.skip(process.platform === 'darwin', 'the main window never closes on macOS');
  await withDemo();
  await quit();
  const win = await launch([], { closeToTray: false, minimizeToTray: false });
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  await startAgain(['Rukoo Mail.exe', '--', LINK]);
  await expectFilled(await composeWindow());
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().endsWith('index.html')).close());
  await expect.poll(() => app.windows().some((p) => p.url().endsWith('index.html'))).toBe(false);
  await startAgain(['Rukoo Mail.exe', '--', 'mailto:second@example.com?subject=Second']);
  await expect.poll(() => app.windows().filter((p) => p.url().endsWith('compose.html')).length).toBe(2);
});

test('a mailto: link in a message opens a compose window with its Cc and Bcc', async () => {
  const win = await withDemo();
  const state = await win.evaluate(() => window.mail.call('state'));
  await win.evaluate(
    ([accountId, link]) => window.mail.call('saveDraft', { accountId, to: ['a@example.com'], subject: 'Has a link', html: `<p><a href="${link}">Write to us</a></p>` }),
    [state.accounts[0].id, LINK]
  );
  await win.click('[data-view="drafts"]');
  await win.locator('.item', { hasText: 'Has a link' }).first().click();
  await win.frameLocator('.mail-frame').locator('a', { hasText: 'Write to us' }).click();
  await expectFilled(await composeWindow());
});

test('closing a compose window a link filled in asks first; Save keeps the message in Drafts, where it opens again', async () => {
  const win = await withDemo();
  await startAgain(['Rukoo Mail.exe', '--', LINK]);
  const compose = await composeWindow();
  await expectFilled(compose);
  // Nobody typed, but the link's words are there to lose.
  await expect(compose.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  await closeCompose();
  await expect(compose.locator('.scrim')).toContainText('Save draft?');
  await expect(compose.locator('.scrim .buttons button')).toHaveText(["Don't save", 'Cancel', 'Save']);
  await compose.click('.scrim .buttons button:text-is("Cancel")');
  await expect(compose.locator('.scrim')).toHaveCount(0);
  expect(composeOpen()).toBe(true);
  await closeCompose();
  await expect(compose.locator('.scrim')).toContainText('Save draft?');
  await Promise.all([compose.waitForEvent('close'), compose.click('.scrim .buttons button:text-is("Save")')]);
  await expect(win.locator('#toast')).toContainText('Saved to Drafts');
  await reopenDraft(win, '1+1=2');
  await expectFilled(win);
});

test('a compose window a link filled in saves itself to Drafts without any typing, and then closes without asking', async () => {
  const win = await withDemo();
  await startAgain(['Rukoo Mail.exe', '--', LINK]);
  const compose = await composeWindow();
  await expectFilled(compose);
  // Autosave runs 8 seconds after the last change; the link's fields count as one.
  await expect(compose.locator('.save-state')).toHaveText('Draft saved', { timeout: 15000 });
  await expect(compose.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await closeCompose();
  await expect.poll(composeOpen).toBe(false);
  await expect(win.locator('#toast')).toContainText('Draft kept in Drafts');
  await reopenDraft(win, '1+1=2');
  await expectFilled(win);
});

test('a link with only an address is no message yet: its window closes without asking and leaves no draft', async () => {
  const win = await withDemo();
  const drafts = await win.locator('[data-view="drafts"] .count').textContent();
  await startAgain(['Rukoo Mail.exe', '--', 'mailto:anna@example.com']);
  const compose = await composeWindow();
  await compose.waitForSelector('.compose .editor');
  await expect(compose.locator('[data-rfield="to"] .recipient')).toHaveText(['anna@example.com']);
  await expect(compose.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await closeCompose();
  await expect.poll(composeOpen).toBe(false);
  await expect(win.locator('[data-view="drafts"] .count')).toHaveText(drafts);
});

test('an unsubscribe email in the reading pane counts as written, while a reply nobody touched still closes without asking', async () => {
  const win = await withDemo();
  await unsubscribeByMail();
  // Opened again, so the reader has the newsletter with only its address.
  await win.locator('.item', { hasText: 'Vandebron' }).first().click();
  await win.locator('.item', { hasText: 'ANWB Newsletter' }).first().click();
  await expect(win.locator('.reader-subject')).toHaveText('Traffic fines in 2027: what you will pay');
  await win.click('[data-reader="unsubscribe"]');
  await expect(win.locator('.scrim')).toContainText('Rukoo Mail will prepare an unsubscribe email for you.');
  await win.click('.scrim .buttons button:text-is("Unsubscribe")');
  await win.waitForSelector('.composer .editor');
  await expect(win.locator('[data-rfield="to"] .recipient')).toHaveText(['leave@anwb.example']);
  await expect(win.locator('[data-field="subject"]')).toHaveValue('Unsubscribe me');
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  // Closing asks; Cancel keeps it.
  await win.click('.composer [data-c="close"]');
  await expect(win.locator('.scrim')).toContainText('Save draft?');
  await win.click('.scrim .buttons button:text-is("Cancel")');
  await expect(win.locator('.scrim')).toHaveCount(0);
  // Opening another message keeps it as a draft, as it does with typed words.
  await win.locator('.item', { hasText: 'Vandebron' }).first().click();
  await expect(win.locator('.composer')).toHaveCount(0);
  await expect(win.locator('#toast')).toContainText('Draft saved to Drafts');
  await reopenDraft(win, 'Unsubscribe me');
  await expect(win.locator('[data-rfield="to"] .recipient')).toHaveText(['leave@anwb.example']);
  expect(await win.locator('.composer .editor').evaluate((e) => e.innerText.trim())).toBe('Please remove me from this list.');

  // A reply holds nothing the email it answers doesn't, so untouched it closes as before.
  await win.click('[data-view="inbox"]');
  await win.locator('.item', { hasText: 'Sanne de Vries' }).first().click();
  await win.click('[data-reader="reply"]');
  await expect(win.locator('.composer [data-field="subject"]')).toHaveValue('Re: Call on Thursday');
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await win.click('.composer [data-c="close"]');
  await expect(win.locator('.composer')).toHaveCount(0);
  await expect(win.locator('.scrim')).toHaveCount(0);
});

// What a run from source says instead of offering itself, per platform. A Mac asks for a copy in Applications.
const unavailable = {
  en: {
    linux: 'Install the .deb package to use Rukoo Mail for email links.',
    darwin: 'Move Rukoo Mail to Applications and open it there to use it for email links.',
    win32: 'Only in the installed or portable app'
  },
  nl: {
    linux: 'Installeer het .deb-pakket om Rukoo Mail voor e-maillinks te gebruiken.',
    darwin: 'Verplaats Rukoo Mail naar Programma’s en open de app daar om e-maillinks ermee te openen.',
    win32: 'Alleen in de geïnstalleerde of draagbare app'
  }
};
for (const [language, title, value] of [
  ['en', 'Email links', unavailable.en[process.platform] || unavailable.en.win32],
  ['nl', 'E-maillinks', unavailable.nl[process.platform] || unavailable.nl.win32]
]) {
  test(`Settings shows Email links, and a run from source can't offer itself (${language})`, async () => {
    await withDemo();
    await quit();
    const win = await launch([], { language });
    await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
    await win.click('[data-action="settings"]');
    const row = win.locator('[data-mailto]');
    await expect(row.locator('.title')).toHaveText(title);
    await expect(row.locator('[data-mailto-value]')).toHaveText(value);
    await expect(row.locator('[data-a="mailto-use"]')).toBeDisabled();
    // Main answers the same for a direct call, and writes nothing.
    await expect(win.evaluate(() => window.mail.call('useForMailto'))).rejects.toThrow(value);
  });
}
