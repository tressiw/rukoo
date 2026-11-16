// Drops on the sidebar's folders and on the composer (#184), in the real app with a throwaway profile and the demo
// account. Chromium loses a drop in two ways. When the element under the pointer changed on the last update before the
// drop, it sends that element a dragenter and no dragover, and a dragenter nobody takes refuses the drop. When that
// element was drawn again after the last update, the drop goes to the old one, which has left the page.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-drop-e2e-');
  fs.mkdirSync(path.join(dataDir, 'data'));
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

const item = (text) => win.locator('.item', { hasText: text }).first();
const inboxCount = () => win.locator('.nav-item[data-view="inbox"] .count');

// Drags a message from the list onto a sidebar row with the mouse and keeps the button down there. The row is found,
// scrolled to and measured in one step in the page, as dragToChat in agent.spec.js does, so no redraw of the list
// comes in between. Returns where the pointer is.
async function holdOver(subject, row) {
  const [from, to] = await win.evaluate(
    ([text, row]) => {
      const el = [...document.querySelectorAll('.list-scroll .item')].find((x) => x.textContent.includes(text));
      el.scrollIntoView({ block: 'center', behavior: 'instant' });
      const a = el.getBoundingClientRect();
      const b = document.querySelector(row).getBoundingClientRect();
      return [
        [a.x + a.width / 2, a.y + a.height / 2],
        [b.x + b.width / 2, b.y + b.height / 2]
      ];
    },
    [subject, row]
  );
  await win.mouse.move(...from);
  await win.mouse.down();
  await win.mouse.move(...to, { steps: 5 });
  return to;
}

test('an email let go over a folder just after the sidebar was due to be drawn again still moves there, and Undo brings it back', async () => {
  await expect(inboxCount()).toHaveText('7');
  const anwb = await item('ANWB Newsletter').getAttribute('data-id');
  const at = await holdOver('Sign in to Bencompare', '[data-folder="Invoices"]');
  // The row itself is under the pointer while an email is dragged, not its name or its icon.
  const over = await win.evaluate(([x, y]) => {
    window.__row = document.querySelector('[data-folder="Invoices"]');
    return { row: document.elementFromPoint(x, y) === window.__row, marked: window.__row.classList.contains('drop-target') };
  }, at);
  expect(over).toEqual({ row: true, marked: true });

  // Mail changes while the email is held there: another message is read on another device, so the counts change.
  await app.evaluate((_e, id) => global.__semEngine.setFlags(id, { unread: false }), anwb);
  await expect(item('ANWB Newsletter')).not.toHaveClass(/unread/);
  // The list shows it. The sidebar waits until the drag is over, so the row under the pointer stays on the page.
  expect(await win.evaluate(() => window.__row.isConnected)).toBe(true);
  await expect(inboxCount()).toHaveText('7');

  await win.mouse.up();
  await expect(win.locator('#toast')).toContainText('Moved to Invoices');
  await expect(win.locator('.item', { hasText: 'Sign in to Bencompare' })).toHaveCount(0);
  // Once the drag is over, the sidebar catches up, also when the list drew the dragged row again and its dragend
  // reached nothing: the next mouse move ends it.
  await win.mouse.move(at[0], at[1] + 60);
  await expect(inboxCount()).toHaveText('6');
  await expect(win.locator('body.dragging-mail')).toHaveCount(0);
  expect(await win.evaluate(() => window.__row.isConnected)).toBe(false);

  await win.click('#toast button');
  await expect(item('Sign in to Bencompare')).toBeVisible();
  await win.click('[data-folder="Invoices"]');
  await expect(win.locator('.list-title h1')).toHaveText('Invoices');
  await expect(win.locator('.item', { hasText: 'Sign in to Bencompare' })).toHaveCount(0);
});

test('folder, Archive and Trash rows take a dragged email on dragenter', async () => {
  const result = await win.evaluate(() => {
    const ev = (type, dataTransfer) => new DragEvent(type, { dataTransfer, bubbles: true, cancelable: true });
    const dt = new DataTransfer();
    [...document.querySelectorAll('.list-scroll .item')].find((x) => x.textContent.includes('Sign in to Bencompare')).dispatchEvent(ev('dragstart', dt));
    const out = { dragging: document.body.classList.contains('dragging-mail') };
    for (const row of ['[data-folder="Invoices"]', '[data-drop-role="archive"]', '[data-drop-role="trash"]']) {
      const label = document.querySelector(`${row} .label`);
      out[row] = { taken: !label.dispatchEvent(ev('dragenter', dt)), passesThrough: getComputedStyle(label).pointerEvents === 'none' };
    }
    // The account switcher is no folder.
    out.account = !document.querySelector('.account-switch').dispatchEvent(ev('dragenter', dt));
    document.querySelector('[data-drop-role="trash"] .label').dispatchEvent(ev('drop', dt));
    return out;
  });
  expect(result).toEqual({
    dragging: true,
    '[data-folder="Invoices"]': { taken: true, passesThrough: true },
    '[data-drop-role="archive"]': { taken: true, passesThrough: true },
    '[data-drop-role="trash"]': { taken: true, passesThrough: true },
    account: false
  });
  await expect(win.locator('#toast')).toContainText('Trash');
  await expect(win.locator('.item', { hasText: 'Sign in to Bencompare' })).toHaveCount(0);
  // When the drag ends, a row's name and icon take clicks again.
  await win.evaluate(() => document.dispatchEvent(new DragEvent('dragend')));
  await expect(win.locator('body.dragging-mail')).toHaveCount(0);
  expect(await win.evaluate(() => getComputedStyle(document.querySelector('[data-folder="Invoices"] .label')).pointerEvents)).toBe('auto');
  await win.click('#toast button');
  await expect(item('Sign in to Bencompare')).toBeVisible();
});

// Files from Explorer: drag events whose DataTransfer holds a file. A file made in the page has no path on disk, so it
// never becomes an attachment; this is about the drop target. Returns what the composer shows while the files are over
// its text.
const filesOver = (page) =>
  page.evaluate(() => {
    const dt = new DataTransfer();
    dt.items.add(new File(['hello'], 'note.txt', { type: 'text/plain' }));
    window.__files = dt;
    const composer = document.querySelector('.composer');
    const editor = composer.querySelector('.editor');
    const taken = !editor.dispatchEvent(new DragEvent('dragenter', { dataTransfer: dt, bubbles: true, cancelable: true }));
    const r = editor.getBoundingClientRect();
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + Math.min(20, r.height / 2));
    return { taken, overlay: composer.classList.contains('drop-files'), target: hit === composer, hint: composer.dataset.dropHint, shown: getComputedStyle(composer, '::after').content };
  });
// The files leave the window, or are let go over the composer.
const filesEnd = (page, type) =>
  page.evaluate((type) => {
    const composer = document.querySelector('.composer');
    composer.dispatchEvent(new DragEvent(type, { dataTransfer: window.__files, bubbles: true, cancelable: true, relatedTarget: null }));
    const r = composer.querySelector('.editor').getBoundingClientRect();
    return { overlay: composer.classList.contains('drop-files'), text: composer.querySelector('.editor').contains(document.elementFromPoint(r.x + r.width / 2, r.y + Math.min(20, r.height / 2))) };
  }, type);

test('files dragged over the composer are taken on dragenter by the composer itself, inline and in the compose window, in English and Dutch', async () => {
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');

  const inline = await filesOver(win);
  expect(inline).toMatchObject({ taken: true, overlay: true, target: true, hint: 'Drop to attach' });
  expect(inline.shown).not.toContain('Loslaten');
  expect(await filesEnd(win, 'dragleave')).toEqual({ overlay: false, text: true });

  // An image moved within the text carries Files too, but it is the editor's: no overlay, and the editor keeps the
  // drop. Files from Explorer right after it are taken again (below), also when that drag's dragend never reached the
  // composer.
  const inner = await win.evaluate(() => {
    const editor = document.querySelector('.composer .editor');
    const img = document.createElement('img');
    img.src = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=';
    editor.prepend(img);
    const dt = new DataTransfer();
    dt.items.add(new File(['x'], 'image.png', { type: 'image/png' }));
    img.dispatchEvent(new DragEvent('dragstart', { dataTransfer: dt, bubbles: true, cancelable: true }));
    const taken = !editor.dispatchEvent(new DragEvent('dragenter', { dataTransfer: dt, bubbles: true, cancelable: true }));
    const overlay = document.querySelector('.composer').classList.contains('drop-files');
    const editorKeepsDrop = editor.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
    img.remove();
    return { taken, overlay, editorKeepsDrop };
  });
  expect(inner).toEqual({ taken: false, overlay: false, editorKeepsDrop: true });

  await win.evaluate(() => window.mail.call('updateSettings', { language: 'nl' }));
  await expect(win.locator('.composer [data-c="send"]')).toContainText('Verzenden');
  expect(await filesOver(win)).toMatchObject({ taken: true, overlay: true, target: true, hint: 'Loslaten om bij te voegen' });
  expect(await filesEnd(win, 'drop')).toEqual({ overlay: false, text: true });

  const [compose] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  compose.on('dialog', () => {});
  await compose.waitForSelector('.compose .editor');
  expect(await filesOver(compose)).toMatchObject({ taken: true, overlay: true, target: true, hint: 'Loslaten om bij te voegen' });
  expect(await filesEnd(compose, 'dragleave')).toEqual({ overlay: false, text: true });
  expect(await filesOver(compose)).toMatchObject({ taken: true, overlay: true, target: true });
  expect(await filesEnd(compose, 'drop')).toEqual({ overlay: false, text: true });
});
