// #317: selecting emails that aren't next to each other with the keyboard alone. The select key adds or removes the
// email at the focus ring, Ctrl+arrows move the ring without opening anything, and after the select key the step keys
// do the same. The real app with a throwaway profile and the demo account, in English.
const { test, expect, _electron: electron } = require('@playwright/test');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

const mac = process.platform === 'darwin';

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-keyselect-e2e-');
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.'], cwd: path.join(__dirname, '..', '..'), env });
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
});

test.afterEach(async () => {
  await closeApp(app);
  removeDir(dataDir);
});

const rows = () => win.locator('.list-scroll .item');
const row = (id) => win.locator(`.list-scroll .item[data-id="${id}"]`);
const ids = () => rows().evaluateAll((els) => els.map((el) => el.dataset.id));
const subjects = () => rows().evaluateAll((els) => els.map((el) => el.querySelector('.subject').textContent.trim()));
const count = () => win.locator('.list-tools .sel-count');
const checkedIds = () => win.locator('.list-scroll .item.checked').evaluateAll((els) => els.map((el) => el.dataset.id));
const press = async (key, times = 1) => {
  for (let i = 0; i < times; i++) await win.keyboard.press(key);
};
// The select key of Rukoo's own scheme. Control+Space on every platform: on a Mac Command+Space is Spotlight.
const SELECT = 'Control+Space';

test('the keyboard alone selects the first, fourth and seventh email, archives exactly those, and Ctrl+Z brings them back', async ({}, testInfo) => {
  const before = await subjects();
  const all = await ids();
  expect(all.length).toBeGreaterThan(7);
  const [first, fourth, seventh] = [all[0], all[3], all[6]];
  const opened = before[0];
  await expect(win.locator('.reader-subject')).toHaveText(opened);

  await win.locator('.list-scroll').focus();
  await press(SELECT);
  await expect(row(first)).toHaveClass(/checked/);
  await expect(count()).toHaveText('1 selected');

  // Ctrl+Down moves the focus ring and nothing else: the open email and the selection stay.
  await press('ControlOrMeta+ArrowDown', 3);
  await expect(row(fourth)).toHaveClass(/cursor/);
  await expect(win.locator('.list-scroll .item.cursor')).toHaveCount(1);
  await expect(win.locator('.reader-subject')).toHaveText(opened);
  await expect(row(fourth)).not.toHaveClass(/checked/);
  await press(SELECT);
  await expect(count()).toHaveText('2 selected');
  await expect(win.locator('.reader .multi')).toBeVisible();

  // After the select key, the plain arrows move the ring too and keep the selection.
  await press('ArrowDown', 3);
  await expect(row(seventh)).toHaveClass(/cursor/);
  await expect(count()).toHaveText('2 selected');
  await press(SELECT);
  await expect(count()).toHaveText('3 selected');
  expect((await checkedIds()).sort()).toEqual([first, fourth, seventh].sort());

  // Pressing it again on a selected email takes it out, and once more puts it back.
  await press(SELECT);
  await expect(row(seventh)).not.toHaveClass(/checked/);
  await expect(count()).toHaveText('2 selected');
  await press(SELECT);
  await expect(count()).toHaveText('3 selected');

  // The focus ring is its own cue: the open email keeps its grey, the ring sits on the seventh, inside the list.
  await expect(row(first)).toHaveClass(/selected/);
  await expect(row(first)).not.toHaveClass(/cursor/);
  await expect(row(seventh)).toHaveClass(/cursor/);
  await expect(win.locator('.list-scroll')).toBeFocused();
  await expect(win.locator('.list-scroll')).toHaveClass(/kbd/);
  await expect(win.locator('.list-scroll')).toHaveAttribute('aria-activedescendant', await row(seventh).getAttribute('id'));
  await win.screenshot({ path: testInfo.outputPath('three-keyboard-selected.png'), animations: 'disabled' });

  await press('e');
  for (const id of [first, fourth, seventh]) await expect(row(id)).toHaveCount(0);
  for (const id of [all[1], all[2], all[4], all[5]]) await expect(row(id)).toHaveCount(1);
  await expect(win.locator('#toast')).toContainText('3 emails moved to Archive');
  const archived = () => win.evaluate(async () => (await window.mail.call('list', { scope: 'all', view: 'archive' })).map((m) => m.subject));
  await expect.poll(archived).toEqual(expect.arrayContaining([before[0], before[3], before[6]]));
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);

  await press('Control+z');
  await expect.poll(subjects).toEqual(before);
});

test("in Gmail's scheme X selects, J and K move the ring and keep the selection, Esc clears it and Enter opens the outlined email", async () => {
  await win.evaluate(() => window.mail.call('updateSettings', { keymap: 'gmail' }));
  const all = await ids();
  const names = await subjects();
  await expect(row(all[0]).locator('[data-check]')).toHaveAttribute('title', 'Select (X)');
  await win.locator('.list-scroll').focus();

  await press('x');
  await expect(count()).toHaveText('1 selected');
  await press('j', 2);
  await expect(row(all[2])).toHaveClass(/cursor/);
  await expect(win.locator('.reader-subject')).toHaveText(names[0]);
  await press('x');
  await expect(count()).toHaveText('2 selected');
  expect((await checkedIds()).sort()).toEqual([all[0], all[2]].sort());
  await press('k');
  await expect(row(all[1])).toHaveClass(/cursor/);
  await expect(count()).toHaveText('2 selected');

  // Esc lets go of the selection; J then opens the next email again, as it always did.
  await press('Escape');
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await press('j');
  await expect(win.locator('.reader-subject')).toHaveText(names[1]);

  // Enter opens the email in the ring and lets go of the selection, as a click on it does.
  await press('x');
  await expect(count()).toHaveText('1 selected');
  await press('j');
  await expect(row(all[2])).toHaveClass(/cursor/);
  await expect(win.locator('.reader-subject')).toHaveText(names[1]);
  await press('Enter');
  await expect(win.locator('.reader-subject')).toHaveText(names[2]);
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await expect(row(all[2])).toHaveClass(/selected/);

  // Enter also clears a selection when the focused email is already the one open.
  await press('x');
  await expect(count()).toHaveText('1 selected');
  await press('Enter');
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await expect(win.locator('.reader-subject')).toHaveText(names[2]);
});

// A narrow window shows the reading pane in the list's place, and with two or more emails checked it shows how many.
// While the select key builds the selection, the list stays, so the ring stays in view.
test('in a narrow window the list stays on screen while the keyboard selects', async ({}, testInfo) => {
  await win.evaluate(() => window.mail.call('updateSettings', { theme: 'dark' }));
  await expect(win.locator('html')).not.toHaveClass(/light/);
  const all = await ids();
  await expect(win.locator('.reader-subject')).toHaveText((await subjects())[0]);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(820, 700));
  const back = win.locator('[data-reader="back"]');
  await expect(back).toBeVisible();
  await expect(win.locator('.listpane')).toBeHidden();
  await back.focus();
  await press(SELECT);
  await expect(win.locator('.listpane')).toBeVisible();
  await expect(win.locator('.list-scroll')).toBeFocused();
  await expect(row(all[0])).toHaveClass(/cursor/);
  await expect(row(all[0])).toHaveClass(/checked/);
  await press('ControlOrMeta+ArrowDown', 2);
  await press(SELECT);
  await expect(count()).toHaveText('2 selected');
  await win.screenshot({ path: testInfo.outputPath('narrow-keyboard-selection.png'), animations: 'disabled' });
  await expect(win.locator('.listpane')).toBeVisible();
  await expect(row(all[2])).toHaveClass(/cursor/);
  await press('ArrowDown');
  await press(SELECT);
  await expect(count()).toHaveText('3 selected');
  await expect(win.locator('.listpane')).toBeVisible();
  await press('Escape');
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await expect(win.locator('.listpane')).toBeVisible();
});

test('moving the keyboard focus from the expanded reader reveals the list', async () => {
  const all = await ids();
  const [opened] = await subjects();
  await expect(win.locator('.reader-subject')).toHaveText(opened);
  const expand = win.locator('[data-reader="expand"]');
  await expand.click();
  await expect(win.locator('.listpane')).toBeHidden();
  await expand.focus();
  await press('ControlOrMeta+ArrowDown');
  await expect(win.locator('.listpane')).toBeVisible();
  await expect(win.locator('.list-scroll')).toBeFocused();
  await expect(row(all[1])).toHaveClass(/cursor/);
  await expect(win.locator('.reader-subject')).toHaveText(opened);
  await press(SELECT);
  expect(await checkedIds()).toEqual([all[1]]);
});

test('inside an open group of mail from one sender, the select key picks single emails and the group stays open', async () => {
  await win.evaluate(() => window.mail.call('updateSettings', { stackSenders: true }));
  const head = win.locator('.list-scroll .item', { hasText: 'Your sign-in details have been updated' });
  const member = win.locator('.list-scroll .item', { hasText: 'How to change your My Vandebron password' });
  const last = win.locator('.list-scroll .item', { hasText: 'Failed sign-in attempt' });
  await expect(head.locator('.stack-btn')).toHaveText('+2');
  // The subject, away from the +2 on the first line.
  await head.locator('.subject').click();
  await expect(win.locator('.reader-subject')).toHaveText('Your sign-in details have been updated');
  await win.locator('.list-scroll').focus();
  await press('ArrowRight');
  await expect(last).toBeVisible();

  await press('ControlOrMeta+ArrowDown');
  await expect(member).toHaveClass(/cursor/);
  await press(SELECT);
  await press('ControlOrMeta+ArrowDown');
  await expect(last).toHaveClass(/cursor/);
  await press(SELECT);
  await expect(count()).toHaveText('2 selected');
  await expect(win.locator('.reader .multi')).toBeVisible();

  // ← leaves a group with selected emails open, and so does taking the last one out.
  await press('ArrowLeft');
  await expect(last).toBeVisible();
  await press(SELECT);
  await expect(count()).toHaveText('1 selected');
  await expect(last).not.toHaveClass(/checked/);
  await expect(last).toHaveClass(/cursor/);
  await expect(member).toHaveClass(/checked/);
  await expect(head).not.toHaveClass(/checked/);

  await press('e');
  await expect(member).toHaveCount(0);
  await expect(head).toBeVisible();
  await expect(last).toBeVisible();
  await expect(win.locator('#toast')).toContainText('Moved to Archive');
  await press('Control+z');
  await expect(member).toBeVisible();
});

test('in the unified inbox of two accounts the keyboard selects and deletes single emails, with undo', async () => {
  // A second account, on a server these tests never reach, makes the inbox the unified one of all accounts.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const server = { host: '127.0.0.1', port: 1, secure: false, user: 'werk@example.com' };
    engine.accounts.push({ id: 'w1', type: 'imap', provider: 'other', email: 'werk@example.com', name: 'Werk', color: '#ff8a3d', signature: null, secret: 'plain:', imap: server, smtp: server, createdAt: Date.now() });
    engine.caches.set('w1', { folders: [], boxes: {}, lastSync: Date.now() });
  });
  await win.reload();
  await expect(rows().first()).toBeVisible({ timeout: 15000 });
  await expect(rows().first().locator('.lead .acc-dot')).toHaveCount(1);
  const before = await subjects();
  const all = await ids();

  await win.locator('.list-scroll').focus();
  await press('ControlOrMeta+ArrowDown');
  await press(SELECT);
  await press('ControlOrMeta+ArrowDown', 2);
  await press(SELECT);
  await expect(count()).toHaveText('2 selected');
  expect((await checkedIds()).sort()).toEqual([all[1], all[3]].sort());

  await press('Delete');
  await expect(row(all[1])).toHaveCount(0);
  await expect(row(all[3])).toHaveCount(0);
  await expect(row(all[2])).toHaveCount(1);
  await expect(win.locator('#toast')).toContainText('2 emails moved to Trash');
  await press('Control+z');
  await expect.poll(subjects).toEqual(before);
});

test('Settings lists the select keys of each scheme, in English and Dutch', async ({}, testInfo) => {
  const page = win.locator('.page.settings');
  const keys = (name) => page.locator('.row.static', { has: win.locator('.title', { hasText: new RegExp(`^${name}$`) }) }).locator('kbd');
  const open = async () => {
    await win.click('[data-action="settings"]');
    await page.locator('[data-a="keyboard"]').click();
  };
  const close = async () => {
    while (await page.count()) await page.locator('[data-a="back"]').click();
  };
  await open();
  await expect(keys('Select or deselect email')).toHaveText(mac ? ['X', '⌃Space'] : ['Ctrl+Space', 'X']);
  await expect(keys('Next email without opening it')).toHaveText([mac ? '⌘↓' : 'Ctrl+↓']);
  await expect(keys('Previous email without opening it')).toHaveText([mac ? '⌘↑' : 'Ctrl+↑']);
  await expect(page.locator('.settings-text').last()).toContainText('Enter opens the email in the outline.');
  await page.locator('[data-a="keymap:gmail"]').click();
  await expect(keys('Select or deselect email')).toHaveText(['X']);
  await page.locator('[data-a="keymap:outlook"]').click();
  await expect(keys('Select or deselect email')).toHaveText(mac ? ['⌃Space', 'X'] : ['Ctrl+Space']);
  await page.locator('[data-a="keymap:apple"]').click();
  await expect(keys('Select or deselect email')).toHaveCount(0);
  await page.locator('[data-a="keymap:rukoo"]').click();
  await close();
  await expect(rows().first().locator('[data-check]')).toHaveAttribute('title', mac ? 'Select (X)' : 'Select (Ctrl+Space)');

  await win.evaluate(() => window.mail.call('updateSettings', { language: 'nl' }));
  await expect(win.locator('.list-title h1')).toHaveText('Postvak IN');
  await open();
  await expect(keys('E-mail selecteren of deselecteren')).toHaveText(mac ? ['X', '⌃Space'] : ['Ctrl+Space', 'X']);
  await expect(keys('Volgende e-mail zonder te openen')).toHaveCount(1);
  await expect(page.locator('.settings-text').last()).toContainText('Enter opent de e-mail in het kader.');
  await keys('E-mail selecteren of deselecteren').first().scrollIntoViewIfNeeded();
  await win.screenshot({ path: testInfo.outputPath('keyboard-settings-nl.png'), animations: 'disabled' });
});
