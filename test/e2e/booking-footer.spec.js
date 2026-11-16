// The booking link in mail, in the real app against the in-memory account server: the line under the signature
// (on, off, per account, in the booking page's language, and in the plain-text part), its rows in Settings, and the
// composer's calendar button inserting the link and free times, inline and in the compose window.
const { test, expect, _electron: electron } = require('@playwright/test');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');
const { startAccountServer } = require('../fixtures/account-server');

let app;
let win;
let dataDir;
let server;

const PASSWORD = 'a long enough password';
const HANDLE = 'demo-host';
const URL_ = `https://meet.example/${HANDLE}`;
const FOOTER_EN = `Want to meet with me? Pick a time, or share this link with your agent: ${URL_}`;
const FOOTER_NL = `Wil je met me afspreken? Kies een moment, of deel deze link met je AI-agent: ${URL_}`;

test.beforeEach(async () => {
  server = await startAccountServer();
  // The pages' public addresses are https, as on a real server.
  server.bookingOrigin = 'https://meet.example';
  dataDir = tempDir('rukoo-footer-e2e-');
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1', RUKOO_ACCOUNT_SERVER: server.url };
  delete env.ELECTRON_RUN_AS_NODE;
  const entry = process.platform === 'linux' ? path.join(__dirname, '..', 'fixtures', 'linux-keyring-app.js') : '.';
  app = await electron.launch({ args: [entry], cwd: path.join(__dirname, '..', '..'), env });
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
});

test.afterEach(async () => {
  await closeApp(app);
  removeDir(dataDir);
  await server.close();
});

const call = (method, ...args) => win.evaluate(([m, a]) => window.mail.call(m, ...a), [method, args]);
const editor = () => win.locator('.composer .editor');
const accountId = async () => (await call('state')).accounts[0].id;

async function pageOn() {
  await call('accountSignUp', { email: 'host@example.com', password: PASSWORD, invite: '' });
  const state = await call('bookingEnable', { handle: HANDLE, name: 'Demo Host', timeZone: 'Europe/Amsterdam', destination: `${await accountId()}\npersonal` });
  expect(state.url).toBe(URL_);
}

// Stored through main, then the window is reloaded so a new message starts from what main keeps.
async function reload() {
  await win.reload();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
}

async function compose() {
  await win.click('[data-action="compose"]');
  await expect(editor()).toBeVisible();
}
async function closeComposer() {
  await win.keyboard.press('Escape');
  await expect(win.locator('.composer')).toHaveCount(0);
}

test('with the footer on, a new message ends with the link inside the signature, and the plain text keeps the address', async () => {
  await pageOn();
  await call('updateSettings', { signatureHtml: '<b>Demo Sender</b>', bookingFooter: true });
  await reload();
  await compose();
  const sig = editor().locator('.signature');
  await expect(sig.locator('b')).toHaveText('Demo Sender');
  await expect(sig.locator(`a[href="${URL_}"]`)).toHaveText(URL_);
  // The footer comes after the signature, in the same block.
  expect(await sig.evaluate((el) => el.innerText.trim())).toBe(`Demo Sender\n${FOOTER_EN}`);

  // What the engine builds from it, for the draft and for the mail: the plain-text part has the address.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const build = engine.buildMail.bind(engine);
    global.__built = [];
    engine.buildMail = async (acc, payload) => {
      const mail = await build(acc, payload);
      global.__built.push(mail.text);
      return mail;
    };
  });
  await win.fill('[data-rinput="to"]', 'demo@example.com');
  await win.keyboard.press('Enter');
  await win.fill('[data-field="subject"]', 'Footer check');
  await editor().click({ position: { x: 4, y: 4 } });
  await win.keyboard.type('Hello there.');
  await win.keyboard.press('Control+s');
  await expect(win.locator('.composer .save-state')).toHaveText('Draft saved');
  const drafted = await app.evaluate(() => global.__built.at(-1));
  expect(drafted).toContain('Hello there.');
  expect(drafted).toContain(FOOTER_EN);

  await win.click('[data-c="send"]');
  await expect(win.locator('.composer')).toHaveCount(0, { timeout: 10000 });
  expect(await app.evaluate(() => global.__built.at(-1))).toContain(FOOTER_EN);
});

test('the footer follows the general setting, the account override, its own text and the page language', async () => {
  await pageOn();
  // Off: nothing, and no empty signature block either.
  await reload();
  await compose();
  await expect(editor().locator('a')).toHaveCount(0);
  await expect(editor().locator('.signature')).toHaveCount(0);
  await closeComposer();

  // On without a signature: the block holds only the footer.
  await call('updateSettings', { bookingFooter: true });
  await reload();
  await compose();
  await expect(editor().locator('.signature')).toHaveText(FOOTER_EN);
  await closeComposer();

  // Off for this account while the general setting is on.
  const id = await accountId();
  await call('updateAccount', id, { bookingFooter: false });
  await reload();
  await compose();
  await expect(editor().locator('.signature')).toHaveCount(0);
  await closeComposer();

  // On for this account with its own text, while the general setting is off; markup in it stays text.
  await call('updateSettings', { bookingFooter: false });
  await call('updateAccount', id, { bookingFooter: true, bookingFooterText: 'Book <i>me</i>: {url}' });
  await reload();
  await compose();
  await expect(editor().locator('.signature')).toHaveText(`Book <i>me</i>: ${URL_}`);
  await expect(editor().locator('.signature i')).toHaveCount(0);
  await closeComposer();

  // Back to the general setting, on, in the booking page's language (Dutch) while the app is in English.
  await call('updateAccount', id, { bookingFooter: null });
  expect((await call('state')).accounts[0]).toMatchObject({ bookingFooter: null, bookingFooterText: null });
  await call('updateSettings', { bookingFooter: true });
  await call('bookingSave', { locale: 'nl' });
  await reload();
  await compose();
  await expect(editor().locator('.signature')).toHaveText(FOOTER_NL);
  await closeComposer();

  // A text without {url} isn't kept: the default comes back.
  await call('updateSettings', { bookingFooterText: 'No link here' });
  expect((await call('state')).settings.bookingFooterText).toBeNull();
});

test('Settings has the footer beside the signature, with a preview, its text and the per-account choice', async () => {
  await pageOn();
  await win.click('[data-action="settings"]');
  const page = win.locator('.page.settings');
  await page.locator('[data-a="signatures"]').click();
  const row = page.locator('[data-a="booking-footer"]');
  await expect(row.locator('.title')).toHaveText('Booking link under signature');
  await expect(row.locator('.switch')).not.toHaveClass(/\bon\b/);
  await row.click();
  await expect(page.locator('[data-a="booking-footer"] .switch')).toHaveClass(/\bon\b/);
  await expect(page.locator('[data-a="booking-footer"] .sig-preview')).toHaveText(FOOTER_EN);
  expect((await call('state')).settings.bookingFooter).toBe(true);

  // Its own text: {url} has to stay in it.
  await page.locator('[data-a="booking-footer-text"]').click();
  const text = win.locator('.scrim textarea');
  await expect(text).toHaveValue('Want to meet with me? Pick a time, or share this link with your agent: {url}');
  await text.fill('Pick a slot');
  await win.locator('.scrim .buttons button', { hasText: 'Save' }).click();
  await expect(win.locator('.scrim [data-error]')).toHaveText("Keep {url} in the text: that's where the link goes.");
  await text.fill('Pick a slot: {url}');
  await win.locator('.scrim .buttons button', { hasText: 'Save' }).click();
  await expect(page.locator('[data-a="booking-footer"] .sig-preview')).toHaveText(`Pick a slot: ${URL_}`);
  await expect(page.locator('[data-a="booking-footer-text"] .value')).toHaveText('Pick a slot: {url}');
  expect((await call('state')).settings.bookingFooterText).toBe('Pick a slot: {url}');

  // The account follows the general setting until it is set to off.
  await page.locator('[data-a="back"]').click();
  await page.locator('.row:has-text("demo@example.com")').first().click();
  const acc = page.locator('[data-a^="booking-acc-footer:"]');
  await expect(acc.locator('.value')).toHaveText('Follows the general setting (On)');
  await acc.click();
  await win.locator('.scrim .radio-row', { hasText: 'Off for this account' }).click();
  await expect(page.locator('[data-a^="booking-acc-footer:"] .value')).toHaveText('Off');
  expect((await call('state')).accounts[0].bookingFooter).toBe(false);
});

test('the footer rows follow the booking page while Signatures or an account page stays open', async () => {
  await call('updateSettings', { bookingFooter: true });
  await win.click('[data-action="settings"]');
  const page = win.locator('.page.settings');
  await page.locator('[data-a="signatures"]').click();
  const row = page.locator('[data-a="booking-footer"]');
  await expect(row.locator('.desc')).toHaveText('Shows once your booking page is on.');
  await expect(row.locator('.sig-preview')).toHaveCount(0);

  // Turned on elsewhere, as an agent or another window would: the link shows without leaving the page.
  await pageOn();
  await expect(row.locator('.desc')).toHaveText('Every new message ends with a line and your booking link.');
  await expect(row.locator('.sig-preview')).toHaveText(FOOTER_EN);
  // The booking page's language is the footer's.
  await call('bookingSave', { locale: 'nl' });
  await expect(row.locator('.sig-preview')).toHaveText(FOOTER_NL);

  // The account's page follows as well: turning the booking page off takes the link away.
  await page.locator('[data-a="back"]').click();
  await page.locator('.row:has-text("demo@example.com")').first().click();
  const acc = page.locator('[data-a^="booking-acc-footer:"]');
  await expect(acc.locator('.sig-preview')).toHaveText(FOOTER_NL);
  await call('bookingDisable');
  await expect(acc.locator('.sig-preview')).toHaveCount(0);
});

test('free times with the link use the default duration of a type guests can book at several lengths', async () => {
  await pageOn();
  const [type] = (await call('bookingState')).settings.types;
  await call('bookingSave', { types: [{ ...type, durations: [15, 30, 60], duration: 60 }] });
  await reload();
  await compose();
  await editor().click({ position: { x: 4, y: 4 } });
  await win.click('.composer [data-c="calendar"]');
  await win.locator('.menu button', { hasText: 'Insert my free times…' }).click();
  const dialog = win.locator('.scrim .dialog');
  await expect(dialog.locator('[name="type"] option')).toHaveText([`${type.title} · 1 hr`]);
  await dialog.locator('[name="range"]').selectOption('days14');
  await expect(dialog.locator('.free-preview li').first()).toBeVisible({ timeout: 10000 });
});

test('the calendar button inserts the booking link and free times, inline and in the compose window', async () => {
  // Without a booking page the menu offers free times only.
  await compose();
  await win.click('.composer [data-c="calendar"]');
  await expect(win.locator('.menu button')).toHaveText(['Insert my free times…']);
  await win.keyboard.press('Escape');
  await closeComposer();

  await pageOn();
  await call('updateSettings', { signatureHtml: '<b>Demo Sender</b>' });
  await reload();
  await compose();
  // With the caret in the text the link goes there.
  await editor().click({ position: { x: 4, y: 4 } });
  await win.keyboard.type('Hi, ');
  await win.click('.composer [data-c="calendar"]');
  await expect(win.locator('.menu button')).toHaveText(['Insert booking link', 'Insert my free times…']);
  await win.locator('.menu button', { hasText: 'Insert booking link' }).click();
  await expect(editor().locator(`a[href="${URL_}"]`)).toHaveText(URL_);
  expect(await editor().evaluate((el) => el.innerText.replace(/ /g, ' ').split('\n')[0])).toBe(`Hi, ${URL_}`);

  // Free times with the link: the meeting type's length and the booking hours, a short list and the link.
  await win.click('.composer [data-c="calendar"]');
  await win.locator('.menu button', { hasText: 'Insert my free times…' }).click();
  const dialog = win.locator('.scrim .dialog');
  await expect(dialog.locator('h2')).toHaveText('Insert free times');
  await expect(dialog.locator('[name="link"]')).toBeChecked();
  await expect(dialog.locator('[name="type"]')).toHaveValue('30min');
  await expect(dialog.locator('[name="hours"]')).toBeDisabled();
  await expect(dialog.locator('[name="hours"]')).toHaveValue('booking');
  await dialog.locator('[name="range"]').selectOption('days14');
  await expect(dialog.locator('.free-preview li').first()).toBeVisible({ timeout: 10000 });
  const preview = await dialog.locator('.free-preview li').allTextContents();
  expect(preview.length).toBeGreaterThan(0);
  expect(preview.length).toBeLessThanOrEqual(5);
  await expect(dialog.locator('.free-preview')).toContainText(`Or pick a time that suits you: ${URL_}`);
  await dialog.locator('.buttons button', { hasText: 'Insert' }).click();
  await expect(editor().locator('ul li')).toHaveText(preview);
  await expect(editor()).toContainText("I'm free at these times:");
  await expect(editor()).toContainText(`Or pick a time that suits you: ${URL_}`);
  // Nothing went into the signature.
  await expect(editor().locator('.signature')).toHaveText('Demo Sender');

  // Without the caret in the text, the link goes above the signature.
  await win.locator('[data-field="subject"]').click();
  await win.click('.composer [data-c="calendar"]');
  await win.locator('.menu button', { hasText: 'Insert booking link' }).click();
  const order = await editor().evaluate((el, url) => {
    const kids = [...el.children];
    const sig = kids.findIndex((k) => k.classList.contains('signature'));
    const link = kids.findLastIndex((k) => k.matches(`a[href="${url}"]`) || k.querySelector(`a[href="${url}"]`));
    return { sig, link };
  }, URL_);
  expect(order.link).toBeGreaterThanOrEqual(0);
  expect(order.link).toBeLessThan(order.sig);

  // The compose window has the same button.
  const [popped] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  await popped.waitForSelector('.compose .editor');
  const other = popped.locator('.compose .editor');
  await other.click({ position: { x: 4, y: 4 } });
  await popped.click('[data-c="calendar"]');
  await popped.locator('.menu button', { hasText: 'Insert my free times…' }).click();
  await popped.locator('.scrim [name="link"]').uncheck();
  await expect(popped.locator('.scrim [name="length"]')).toBeVisible();
  await expect(popped.locator('.scrim [name="hours"]')).toBeEnabled();
  await popped.locator('.scrim [name="range"]').selectOption('days14');
  await expect(popped.locator('.scrim .free-preview li').first()).toBeVisible({ timeout: 10000 });
  await expect(popped.locator('.scrim .free-preview')).not.toContainText(URL_);
  await popped.locator('.scrim .buttons button', { hasText: 'Insert' }).click();
  await expect(other.locator('ul li').first()).toBeVisible();
  await popped.click('[data-c="calendar"]');
  await popped.locator('.menu button', { hasText: 'Insert booking link' }).click();
  await expect(other.locator(`a[href="${URL_}"]`).first()).toHaveText(URL_);
});

test("free times with the link leave out what the meeting type's notice and horizon rule out", async () => {
  await pageOn();
  // Bookable only from a week ahead, and no further than a week: the page has nothing to offer.
  const [type] = (await call('bookingState')).settings.types;
  await call('bookingSave', { types: [{ ...type, minNotice: 10080, horizon: 7 }] });
  await compose();
  await editor().click({ position: { x: 4, y: 4 } });
  await win.click('.composer [data-c="calendar"]');
  await win.locator('.menu button', { hasText: 'Insert my free times…' }).click();
  const dialog = win.locator('.scrim .dialog');
  await expect(dialog.locator('[name="link"]')).toBeChecked();
  await dialog.locator('[name="range"]').selectOption('days14');
  await expect(dialog.locator('.free-preview')).toHaveText('No open time in this period. Try another period, length or hours.', { timeout: 10000 });
  await expect(dialog.locator('.buttons button', { hasText: 'Insert' })).toBeDisabled();
  // The same period without the link has times.
  await dialog.locator('[name="link"]').uncheck();
  await expect(dialog.locator('.free-preview li').first()).toBeVisible({ timeout: 10000 });
  await expect(dialog.locator('.buttons button', { hasText: 'Insert' })).toBeEnabled();
});

test('free times in a message whose footer has the link follow the page without a second link', async () => {
  await pageOn();
  await call('updateSettings', { signatureHtml: '<b>Demo Sender</b>', bookingFooter: true });
  await reload();
  await compose();
  await expect(editor().locator(`.signature a[href="${URL_}"]`)).toHaveCount(1);
  await editor().click({ position: { x: 4, y: 4 } });
  await win.click('.composer [data-c="calendar"]');
  await win.locator('.menu button', { hasText: 'Insert my free times…' }).click();
  const dialog = win.locator('.scrim .dialog');
  await expect(dialog.locator('[name="link"]')).toBeChecked();
  await expect(dialog.locator('.field-help')).toContainText("The link is under your signature already, so it isn't added twice.");
  await dialog.locator('[name="range"]').selectOption('days14');
  await expect(dialog.locator('.free-preview li').first()).toBeVisible({ timeout: 10000 });
  await expect(dialog.locator('.free-preview')).not.toContainText(URL_);
  await dialog.locator('.buttons button', { hasText: 'Insert' }).click();
  await expect(editor().locator('ul li').first()).toBeVisible();
  await expect(editor().locator(`a[href="${URL_}"]`)).toHaveCount(1);
});
