// RUK-101: @-mentions in the composer, in the reading pane and in the compose window, against the demo account.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

async function launch(language = null) {
  dataDir = tempDir('sem-e2e-');
  if (language) {
    fs.mkdirSync(path.join(dataDir, 'data'));
    fs.writeFileSync(path.join(dataDir, 'data', 'settings.json'), JSON.stringify({ language }));
  }
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.'], cwd: path.join(__dirname, '..', '..'), env });
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
}

test.afterEach(async () => {
  await closeApp(app);
  removeDir(dataDir);
});

const item = (text) => win.locator('.item', { hasText: text }).first();
const list = (page) => page.locator('.mention-suggest [role="listbox"]');
const names = (page) => page.locator('.mention-suggest [role="option"] .mention-name');
const chips = (page, field) => page.locator(`[data-rfield="${field}"] .recipient`);
const mentions = (page) => page.locator('.composer .editor a.mention');

async function replyAllToSanne() {
  await item('Sanne de Vries').click();
  await win.click('[data-reader="replyAll"]');
  await win.waitForSelector('.composer .editor');
  await win.click('.composer .editor');
}

async function newMessage() {
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
}

// The field's own suggestions turn a known address into the person's name. Enter picks the suggestion only while the
// field has the focus; typed text that is left behind becomes a recipient without a name. So the helper checks both
// ends: a pick that didn't happen fails here, where it happened, and not at the next check of the recipients (#194).
async function addRecipient(page, field, address, name) {
  if (field !== 'to') await page.click(`[data-c="${field}"]`);
  const input = page.locator(`[data-rinput="${field}"]`);
  await input.fill(address);
  await expect(page.locator(`[data-rfield="${field}"] .suggest [role="option"]`).first()).toContainText(address);
  await expect(input).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(chips(page, field).last()).toHaveText(name);
  await expect(input).toHaveValue('');
}

// The stored copy of a message, by subject, as the reader gets it.
async function stored(view, subject) {
  return win.evaluate(
    async ({ view, subject }) => {
      const found = (await window.mail.call('list', { view })).find((m) => m.subject === subject);
      return found ? window.mail.call('get', found.id) : null;
    },
    { view, subject }
  );
}

test('a bare @ in a reply offers the thread; a pick of a recipient only inserts the mention', async () => {
  await launch();
  await replyAllToSanne();
  await win.keyboard.type('Hi @');
  await expect(list(win)).toBeVisible();
  await expect(names(win)).toHaveText(['Sanne de Vries', 'Joris Bakker']);
  await expect(win.locator('.mention-suggest .mention-tag')).toHaveText(['To', 'Cc']);
  await expect(win.locator('.mention-suggest .mention-hint')).toBeHidden();
  await expect(list(win)).toHaveAttribute('aria-label', 'People to mention');

  // The editor points at the list and its active option, which the arrows move round.
  const editor = win.locator('.composer .editor');
  const options = win.locator('.mention-suggest [role="option"]');
  await expect(editor).toHaveAttribute('aria-controls', await list(win).getAttribute('id'));
  await expect(editor).toHaveAttribute('aria-activedescendant', await options.nth(0).getAttribute('id'));
  await expect(options.nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(options.nth(0)).toHaveAttribute('aria-label', 'Sanne de Vries, sanne@example.com, Already in To');
  await win.keyboard.press('ArrowDown');
  await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');
  await expect(editor).toHaveAttribute('aria-activedescendant', await options.nth(1).getAttribute('id'));
  await win.keyboard.press('ArrowDown');
  await expect(options.nth(0)).toHaveAttribute('aria-selected', 'true');
  await win.keyboard.press('ArrowUp');
  await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');

  await win.keyboard.type('jo');
  await expect(names(win)).toHaveText(['Joris Bakker']);
  await win.keyboard.press('Enter');
  await expect(list(win)).toHaveCount(0);
  await expect(editor).not.toHaveAttribute('aria-controls', /.+/);
  await expect(mentions(win)).toHaveText(['@Joris Bakker']);
  await expect(mentions(win)).toHaveAttribute('href', 'mailto:joris@example.com');
  // A colon right after the mention takes the place of the space after it; Ctrl+Z gives the space back.
  await win.keyboard.type(':');
  await expect(editor).toHaveText('Hi @Joris Bakker:');
  await win.keyboard.press('Control+z');
  await expect(editor).toHaveText('Hi @Joris Bakker ');
  await win.keyboard.press('Control+y');
  await win.keyboard.type(' can you join?');
  await expect(editor).toHaveText('Hi @Joris Bakker: can you join?');
  // A word goes after the space.
  await win.keyboard.type(' And @sa');
  await win.keyboard.press('Enter');
  await win.keyboard.type('too');
  await expect(editor).toHaveText('Hi @Joris Bakker: can you join? And @Sanne de Vries too');
  // Joris was in Cc already: the recipients stay as they were.
  await expect(chips(win, 'to')).toHaveText(['Sanne de Vries']);
  await expect(chips(win, 'cc')).toHaveText(['Joris Bakker']);
  await expect(win.locator('#toast')).not.toContainText('added to Cc');
});

test('someone not on the email yet goes to Cc; Backspace takes the mention off and leaves them there', async () => {
  await launch();
  await newMessage();
  await addRecipient(win, 'to', 'joris@example.com', 'Joris Bakker');
  await win.click('.composer .editor');
  await win.keyboard.type('Hello @vande');
  await expect(names(win)).toHaveText(['Vandebron']);
  await expect(win.locator('.mention-suggest .mention-tag')).toHaveCount(0);
  await expect(win.locator('.mention-suggest .mention-hint')).toHaveText("People who aren't on the email yet go to Cc");
  await expect(win.locator('.mention-suggest [role="option"]').first()).toHaveAttribute('aria-label', 'Vandebron, support@vandebron.nl, Adds to Cc');
  // Tab picks, like Enter.
  await win.keyboard.press('Tab');
  await expect(mentions(win)).toHaveText(['@Vandebron']);
  await expect(win.locator('[data-rfield="cc"]')).toBeVisible();
  await expect(chips(win, 'cc')).toHaveText(['Vandebron']);
  await expect(win.locator('#toast')).toHaveText('Vandebron added to Cc');
  await expect(win.locator('.composer .editor')).toBeFocused();

  // Joris is in To: the mention goes in, the recipients stay.
  await win.keyboard.type('and @jor');
  await expect(win.locator('.mention-suggest .mention-tag')).toHaveText(['To']);
  await win.keyboard.press('Enter');
  await expect(mentions(win)).toHaveText(['@Vandebron', '@Joris Bakker']);
  await expect(chips(win, 'to')).toHaveText(['Joris Bakker']);
  await expect(chips(win, 'cc')).toHaveText(['Vandebron']);

  // The space after the mention, then the mention in one go. Ctrl+Z brings it back.
  await win.keyboard.press('Backspace');
  await win.keyboard.press('Backspace');
  await expect(mentions(win)).toHaveText(['@Vandebron']);
  await expect(chips(win, 'to')).toHaveText(['Joris Bakker']);
  await win.keyboard.press('Control+z');
  await expect(mentions(win)).toHaveText(['@Vandebron', '@Joris Bakker']);
  await expect(mentions(win).last()).toHaveAttribute('contenteditable', 'false');
});

test('an address in the text, quoted text and the signature open nothing; Escape closes only the list', async () => {
  await launch();
  await newMessage();
  await win.click('.composer .editor');
  await win.keyboard.type('Write to demo@example.com or sanne@exa');
  await win.waitForTimeout(300);
  await expect(list(win)).toHaveCount(0);

  // Quoted text and the signature, with the caret at the end of each.
  for (const html of ['<blockquote>Earlier mail</blockquote>', '<div class="signature">Demo</div>']) {
    await win.evaluate((html) => {
      const editor = document.querySelector('.composer .editor');
      editor.insertAdjacentHTML('beforeend', html);
      const text = editor.lastElementChild.firstChild;
      const range = document.createRange();
      range.setStart(text, text.length);
      window.getSelection().removeAllRanges();
      window.getSelection().addRange(range);
    }, html);
    await win.keyboard.type(' @sa');
    await win.waitForTimeout(300);
    await expect(list(win)).toHaveCount(0);
  }

  await win.evaluate(() => {
    const editor = document.querySelector('.composer .editor');
    const range = document.createRange();
    range.setStart(editor.firstChild, editor.firstChild.childNodes.length);
    window.getSelection().removeAllRanges();
    window.getSelection().addRange(range);
  });
  await win.keyboard.type(' @sa');
  await expect(names(win)).toHaveText(['Sanne de Vries']);
  await win.keyboard.press('Escape');
  await expect(list(win)).toHaveCount(0);
  await expect(win.locator('.composer')).toHaveCount(1);
  // Typing on after Escape leaves the list closed; the next @ opens it again.
  await win.keyboard.type('n');
  await win.waitForTimeout(300);
  await expect(list(win)).toHaveCount(0);
  await win.keyboard.type(' @jo');
  await expect(names(win)).toHaveText(['Joris Bakker']);
  // Clicking elsewhere closes it too.
  await win.click('[data-field="subject"]');
  await expect(list(win)).toHaveCount(0);
});

test('a saved draft keeps its mentions after reopening; the stored mail has no editor locks', async () => {
  await launch();
  await newMessage();
  await addRecipient(win, 'to', 'joris@example.com', 'Joris Bakker');
  await win.fill('[data-field="subject"]', 'Mention draft');
  await win.click('.composer .editor');
  await win.keyboard.type('@sa');
  await expect(names(win)).toHaveText(['Sanne de Vries']);
  await win.keyboard.press('Enter');
  await win.keyboard.type(': the proposal');
  await win.keyboard.press('Control+s');
  await expect(win.locator('.composer .save-state')).toHaveText('Draft saved');
  await win.keyboard.press('Escape');
  await expect(win.locator('.composer')).toHaveCount(0, { timeout: 10000 });

  const draft = await stored('drafts', 'Mention draft');
  expect(draft.html).toContain('<a href="mailto:sanne@example.com" class="mention">@Sanne de Vries</a>');
  expect(draft.html).not.toContain('contenteditable');
  expect(draft.cc.map((a) => a.address)).toEqual(['sanne@example.com']);

  await win.click('[data-view="drafts"]');
  await item('Mention draft').click();
  await win.click('[data-reader="edit"]');
  await win.waitForSelector('.composer .editor');
  await expect(mentions(win)).toHaveText(['@Sanne de Vries']);
  await expect(mentions(win)).toHaveAttribute('href', 'mailto:sanne@example.com');
  await expect(mentions(win)).toHaveAttribute('contenteditable', 'false');
  await expect(chips(win, 'cc')).toHaveText(['Sanne de Vries']);
});

test('the sent mail links mentions of people in To and Cc, and names someone in Bcc without the address', async () => {
  await launch();
  await newMessage();
  await addRecipient(win, 'to', 'joris@example.com', 'Joris Bakker');
  await addRecipient(win, 'bcc', 'sanne@example.com', 'Sanne de Vries');
  // Someone in Bcc without a name: the mention reads the part before the @.
  await win.fill('[data-rinput="bcc"]', 'hidden@example.org');
  await win.keyboard.press('Enter');
  await expect(chips(win, 'bcc')).toHaveCount(2);
  await expect(chips(win, 'bcc').last()).toHaveText('hidden@example.org');
  await win.fill('[data-field="subject"]', 'Mentions out');
  await win.click('.composer .editor');
  await win.keyboard.type('@jo');
  await expect(names(win)).toHaveText(['Joris Bakker']);
  await win.keyboard.press('Enter');
  await win.keyboard.type('and @sa');
  await expect(win.locator('.mention-suggest .mention-tag')).toHaveText(['Bcc']);
  await win.keyboard.press('Enter');
  await win.keyboard.type('and @hid');
  await expect(names(win)).toHaveText(['hidden@example.org']);
  await win.keyboard.press('Enter');
  await expect(mentions(win)).toHaveText(['@Joris Bakker', '@Sanne de Vries', '@hidden']);
  await expect(chips(win, 'cc')).toHaveCount(0);
  await win.click('[data-c="send"]');
  await expect(win.locator('.composer')).toHaveCount(0, { timeout: 10000 });

  await expect.poll(async () => Boolean(await stored('sent', 'Mentions out')), { timeout: 10000 }).toBe(true);
  const sent = await stored('sent', 'Mentions out');
  expect(sent.html).toContain('<a href="mailto:joris@example.com" class="mention">@Joris Bakker</a>');
  expect(sent.html).toContain('<span class="mention">@Sanne de Vries</span>');
  expect(sent.html).toContain('<span class="mention">@hidden</span>');
  expect(sent.html).not.toContain('sanne@example.com');
  expect(sent.html).not.toContain('hidden@example.org');
  expect(sent.html).not.toContain('contenteditable');
  // The plain-text part reads as written.
  expect(sent.text).toContain('@Joris Bakker and @Sanne de Vries and @hidden');
  expect(sent.text).not.toContain('mailto');
});

test('the compose window suggests the thread too, and fits a narrow window', async () => {
  await launch();
  const call = await win.evaluate(async () => (await window.mail.call('list', { view: 'inbox' })).find((m) => m.subject === 'Call on Thursday').id);
  const [popup] = await Promise.all([app.waitForEvent('window'), win.evaluate((id) => window.mail.call('openCompose', { mode: 'reply', id }), call)]);
  await popup.waitForSelector('.composer .editor');
  // A plain reply: only Sanne is in To, but Joris was on her message.
  await expect(chips(popup, 'to')).toHaveText(['Sanne de Vries']);
  await popup.click('.composer .editor');
  await popup.keyboard.type('@');
  await expect(names(popup)).toHaveText(['Sanne de Vries', 'Joris Bakker']);
  await expect(popup.locator('.mention-suggest .mention-tag')).toHaveText(['To']);
  await popup.keyboard.press('ArrowDown');
  await popup.keyboard.press('Enter');
  await expect(mentions(popup)).toHaveText(['@Joris Bakker']);
  await expect(chips(popup, 'cc')).toHaveText(['Joris Bakker']);
  await expect(popup.locator('#toast')).toHaveText('Joris Bakker added to Cc');

  // In a narrow window the list stays inside it, also from the end of a long line.
  await app.evaluate(({ BrowserWindow }) => {
    const compose = BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().includes('compose.html'));
    compose.setMinimumSize(200, 200);
    compose.setSize(380, 640);
  });
  await expect.poll(() => popup.evaluate(() => window.innerWidth)).toBeLessThan(400);
  await popup.keyboard.type(' and then a few more words @vande');
  await expect(names(popup)).toHaveText(['Vandebron']);
  const box = await popup.locator('.mention-suggest').boundingBox();
  const width = await popup.evaluate(() => window.innerWidth);
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(width);
});

test('in a low compose window at 200 percent the list fits and scrolls to the active option', async () => {
  await launch();
  const people = ['Anna', 'Bram', 'Cees', 'Dirk', 'Eva', 'Fleur'].map((name) => ({ name: `${name} Visser`, address: `${name.toLowerCase()}@example.com` }));
  const [popup] = await Promise.all([app.waitForEvent('window'), win.evaluate((to) => window.mail.call('openCompose', { mode: 'new', to }), people)]);
  await popup.waitForSelector('.composer .editor');
  await app.evaluate(({ BrowserWindow }) => {
    const compose = BrowserWindow.getAllWindows().find((w) => w.webContents.getURL().includes('compose.html'));
    compose.setSize(560, 460);
    compose.webContents.setZoomFactor(2);
  });
  await expect.poll(() => popup.evaluate(() => window.innerHeight)).toBeLessThan(240);
  await popup.click('.composer .editor');
  await popup.keyboard.type('@');
  await expect(names(popup)).toHaveText(people.map((p) => p.name));
  const fits = () =>
    popup.evaluate(() => {
      const r = document.querySelector('.mention-suggest').getBoundingClientRect();
      return r.top >= 0 && r.bottom <= window.innerHeight && r.left >= 0 && r.right <= window.innerWidth;
    });
  expect(await fits()).toBe(true);
  const listEl = list(popup);
  expect(await listEl.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
  // Up from the first option wraps to the last, which scrolls into view.
  await popup.keyboard.press('ArrowUp');
  const last = popup.locator('.mention-suggest [role="option"]').last();
  await expect(last).toHaveAttribute('aria-selected', 'true');
  const shown = await last.evaluate((el) => {
    const box = el.closest('[role="listbox"]').getBoundingClientRect();
    const r = el.getBoundingClientRect();
    return r.top >= box.top - 1 && r.bottom <= box.bottom + 1;
  });
  expect(shown).toBe(true);
  await popup.keyboard.press('Enter');
  await expect(mentions(popup)).toHaveText(['@Fleur Visser']);
});

test('the suggestions and the Cc notice speak Dutch', async () => {
  await launch('nl');
  await newMessage();
  await addRecipient(win, 'to', 'joris@example.com', 'Joris Bakker');
  await win.click('.composer .editor');
  await win.keyboard.type('@vande');
  await expect(list(win)).toHaveAttribute('aria-label', 'Personen om te vermelden');
  await expect(win.locator('.mention-suggest .mention-hint')).toHaveText('Wie nog niet op de e-mail staat, komt in Cc');
  await win.keyboard.press('Enter');
  await expect(win.locator('#toast')).toHaveText('Vandebron toegevoegd aan Cc');
  await win.keyboard.type('en @jo');
  const tag = win.locator('.mention-suggest .mention-tag');
  await expect(tag).toHaveText(['Aan']);
  await expect(tag).toHaveAttribute('title', 'Staat al in Aan');
});

// A recipient field that loses the focus closes its suggestions a moment later, so a click on one still picks it. That
// moment once closed the next field's list too, opened in the meantime, and Enter then added the typed address
// without its name. The clock stands still until the list is open, then moves past that moment.
for (const surface of ['inline', 'window']) {
  test(`suggestions opened right after another recipient field lost the focus stay open (${surface})`, async () => {
    await launch();
    let page = win;
    if (surface === 'inline') await newMessage();
    else {
      [page] = await Promise.all([app.waitForEvent('window'), win.evaluate(() => window.mail.call('openCompose', { mode: 'new' }))]);
      await page.waitForSelector('.composer .editor');
    }
    await expect(page.locator('[data-rinput="to"]')).toBeFocused();
    await page.clock.install();
    await page.clock.pauseAt(Date.now() + 1000);
    // Bcc takes the focus from To, which sets To's close going.
    await page.click('[data-c="bcc"]');
    await page.fill('[data-rinput="bcc"]', 'sanne@example.com');
    const options = page.locator('[data-rfield="bcc"] .suggest [role="option"]');
    await expect(options.first()).toContainText('sanne@example.com');
    await page.clock.runFor(1000);
    await expect(options).toHaveCount(1);
    await page.keyboard.press('Enter');
    await expect(chips(page, 'bcc')).toHaveText(['Sanne de Vries']);
  });
}

// WCAG's contrast ratio between two computed colors, such as 'rgb(37, 38, 45)'.
function contrast(a, b) {
  const luminance = (color) => {
    const [r, g, bl] = color
      .match(/\d+(\.\d+)?/g)
      .slice(0, 3)
      .map((v) => Number(v) / 255)
      .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + 0.05) / (dark + 0.05);
}

// The row Enter picks has the selected-row grey. The hover grey it had is darker than the list in the dark theme,
// 1.02 to 1, so nothing showed which recipient Enter would add.
test('the recipient suggestion Enter picks stands out from the list in the dark theme', async () => {
  await launch();
  await win.evaluate(() => window.mail.call('updateSettings', { theme: 'dark' }));
  await expect(win.locator('html')).not.toHaveClass(/light/);
  await newMessage();
  await win.fill('[data-rinput="to"]', 'joris');
  const active = win.locator('[data-rfield="to"] .suggest button.active');
  await expect(active).toContainText('joris@example.com');
  const [row, list] = await active.evaluate((el) => [getComputedStyle(el).backgroundColor, getComputedStyle(el.parentElement).backgroundColor]);
  expect(contrast(row, list)).toBeGreaterThanOrEqual(1.1);
});
