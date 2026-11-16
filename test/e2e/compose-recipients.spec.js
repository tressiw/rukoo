// Dragging a recipient between To, Cc and Bcc, in the editor in the reading pane and in a compose window: onto a
// field that is open, onto the Cc or Bcc link that opens one, and onto a field that already has that address.
// The real app with a throwaway profile and the demo account.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-compose-recipients-e2e-');
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

const chips = (page, field) => page.locator(`[data-rfield="${field}"] .recipient`);
const type = async (page, field, text) => {
  await page.locator(`[data-rinput="${field}"]`).fill(text);
  await page.locator(`[data-rinput="${field}"]`).press('Enter');
};

test('recipients move between To, Cc and Bcc by dragging, inline and in the compose window', async () => {
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  for (const a of ['anna@test.invalid', 'joris@test.invalid', 'sanne@test.invalid']) await type(win, 'to', a);
  await expect(chips(win, 'to')).toHaveText(['anna@test.invalid', 'joris@test.invalid', 'sanne@test.invalid']);
  await expect(win.locator('[data-rfield="cc"]')).toBeHidden();

  // Onto the Cc link while Cc is closed: it opens with the address in it.
  await chips(win, 'to').nth(1).dragTo(win.locator('[data-c="cc"]'));
  await expect(win.locator('[data-rfield="cc"]')).toBeVisible();
  await expect(win.locator('[data-c="cc"]')).toBeHidden();
  await expect(chips(win, 'cc')).toHaveText(['joris@test.invalid']);
  await expect(chips(win, 'to')).toHaveText(['anna@test.invalid', 'sanne@test.invalid']);

  // Onto an open field.
  await chips(win, 'to').first().dragTo(win.locator('[data-rfield="cc"]'));
  await expect(chips(win, 'cc')).toHaveText(['joris@test.invalid', 'anna@test.invalid']);
  await expect(chips(win, 'to')).toHaveText(['sanne@test.invalid']);

  // Onto its own field nothing changes; onto the Bcc link it moves there.
  await chips(win, 'to').first().dragTo(win.locator('[data-rinput="to"]'));
  await expect(chips(win, 'to')).toHaveText(['sanne@test.invalid']);
  await chips(win, 'to').first().dragTo(win.locator('[data-c="bcc"]'));
  await expect(chips(win, 'bcc')).toHaveText(['sanne@test.invalid']);
  await expect(chips(win, 'to')).toHaveCount(0);

  // An address the field already has isn't added twice.
  await type(win, 'to', 'joris@test.invalid');
  await chips(win, 'to').first().dragTo(win.locator('[data-rfield="cc"]'));
  await expect(chips(win, 'cc')).toHaveText(['joris@test.invalid', 'anna@test.invalid']);
  await expect(chips(win, 'to')).toHaveCount(0);

  // The compose window gets the fields as they are, and dragging works there too.
  const [compose] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  compose.on('dialog', () => {});
  await compose.waitForSelector('.compose .editor');
  await expect(chips(compose, 'cc')).toHaveText(['joris@test.invalid', 'anna@test.invalid']);
  await expect(chips(compose, 'bcc')).toHaveText(['sanne@test.invalid']);
  await chips(compose, 'bcc').first().dragTo(compose.locator('[data-rfield="to"]'));
  await expect(chips(compose, 'to')).toHaveText(['sanne@test.invalid']);
  await expect(chips(compose, 'bcc')).toHaveCount(0);
  await chips(compose, 'cc').last().dragTo(compose.locator('[data-rfield="bcc"]'));
  await expect(chips(compose, 'bcc')).toHaveText(['anna@test.invalid']);
  await expect(chips(compose, 'cc')).toHaveText(['joris@test.invalid']);
});

test('a recipient let go over Send, the text or the subject stays where it was', async () => {
  const errors = [];
  win.on('pageerror', (err) => errors.push(err.message));
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  for (const a of ['anna@test.invalid', 'joris@test.invalid']) await type(win, 'to', a);

  await chips(win, 'to').first().dragTo(win.locator('[data-c="send"]'));
  await chips(win, 'to').first().dragTo(win.locator('.composer .editor'));
  await chips(win, 'to').last().dragTo(win.locator('[data-field="subject"]'));
  await expect(chips(win, 'to')).toHaveText(['anna@test.invalid', 'joris@test.invalid']);
  // The chips are drawn from the composer's own list, not just left on screen: a new address draws them again.
  await type(win, 'to', 'sanne@test.invalid');
  await expect(chips(win, 'to')).toHaveText(['anna@test.invalid', 'joris@test.invalid', 'sanne@test.invalid']);
  await expect(win.locator('.composer .editor')).not.toContainText('test.invalid');
  await expect(win.locator('[data-field="subject"]')).toHaveValue('');
  await expect(win.locator('.drop-recipient, .recipient.dragging')).toHaveCount(0);
  expect(errors).toEqual([]);
});

// Drag events made in the page, for what a mouse in a test can't time: the dragenter Chromium sends instead of a
// dragover when the element under the pointer changed on the last update before the drop, a list drawn again during
// the drag, and a drag whose end never reached the composer.
test('a recipient is taken on dragenter and moves by its address; files dragged over a field are no recipient', async () => {
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  for (const a of ['anna@test.invalid', 'joris@test.invalid', 'sanne@test.invalid']) await type(win, 'to', a);
  await win.click('[data-c="cc"]');

  const chip = await win.evaluate(() => {
    const ev = (type, dataTransfer) => new DragEvent(type, { dataTransfer, bubbles: true, cancelable: true });
    const dt = new DataTransfer();
    document.querySelectorAll('[data-rfield="to"] .recipient')[1].dispatchEvent(ev('dragstart', dt));
    const label = document.querySelector('[data-rfield="cc"] label');
    const result = {
      taken: !label.dispatchEvent(ev('dragenter', dt)),
      marked: document.querySelector('[data-rfield="cc"]').classList.contains('drop-recipient'),
      send: !document.querySelector('[data-c="send"]').dispatchEvent(ev('dragenter', dt)),
      editor: !document.querySelector('.composer .editor').dispatchEvent(ev('dragover', dt))
    };
    // Dropped on Send anyway: nothing happens to the recipients.
    document.querySelector('[data-c="send"]').dispatchEvent(ev('drop', dt));
    // To is drawn again while the chip is on its way, as when an agent writes the recipients: anna goes, so
    // joris is first now.
    document.querySelector('[data-rfield="to"] [data-remove="0"]').click();
    label.dispatchEvent(ev('dragover', dt));
    label.dispatchEvent(ev('drop', dt));
    return result;
  });
  expect(chip).toEqual({ taken: true, marked: true, send: false, editor: false });
  await expect(chips(win, 'cc')).toHaveText(['joris@test.invalid']);
  await expect(chips(win, 'to')).toHaveText(['sanne@test.invalid']);

  // A chip drag whose dragend never came, as when its chip was drawn again and it was let go outside the window, then a
  // file from Explorer over Cc: it moves no recipient, and the composer takes it as an attachment. The file comes from
  // a file input, so it has a path on disk the way a file from Explorer does.
  const file = path.join(dataDir, 'note.txt');
  fs.writeFileSync(file, 'hello');
  await win.evaluate(() => document.body.append(Object.assign(document.createElement('input'), { type: 'file', id: 'test-file', hidden: true })));
  await win.setInputFiles('#test-file', file);
  const files = await win.evaluate(() => {
    const ev = (type, dataTransfer) => new DragEvent(type, { dataTransfer, bubbles: true, cancelable: true });
    document.querySelector('[data-rfield="to"] .recipient').dispatchEvent(ev('dragstart', new DataTransfer()));
    const input = document.getElementById('test-file');
    const dt = new DataTransfer();
    dt.items.add(input.files[0]);
    input.remove();
    const cc = document.querySelector('[data-rfield="cc"]');
    const taken = !cc.dispatchEvent(ev('dragenter', dt));
    cc.dispatchEvent(ev('dragover', dt));
    const result = { taken, overlay: document.querySelector('.composer').classList.contains('drop-files'), marked: cc.classList.contains('drop-recipient') };
    result.dropped = !cc.dispatchEvent(ev('drop', dt));
    return result;
  });
  expect(files).toEqual({ taken: true, overlay: true, marked: false, dropped: true });
  await expect(win.locator('.composer .att-chip .n')).toHaveText(['note.txt']);
  await expect(win.locator('.composer.drop-files')).toHaveCount(0);
  await expect(chips(win, 'to')).toHaveText(['sanne@test.invalid']);
  await expect(chips(win, 'cc')).toHaveText(['joris@test.invalid']);
  // That drag ends with the next dragend anywhere in the window.
  await win.evaluate(() => document.dispatchEvent(new DragEvent('dragend')));
  await expect(win.locator('.recipient.dragging')).toHaveCount(0);
});

// A chip drag in this window whose dragend never came leaves the composer thinking one of its chips is still on its way.
// A chip from another compose window has the same type, and its drop here must not move that chip: the drop has to
// bring back the drag's own token.
test('a chip from another compose window moves no recipient here, even after a drag here that never ended', async () => {
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  for (const a of ['anna@test.invalid', 'joris@test.invalid']) await type(win, 'to', a);
  await win.click('[data-c="cc"]');
  await type(win, 'cc', 'sanne@test.invalid');
  const [compose] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  compose.on('dialog', () => {});
  const errors = [];
  compose.on('pageerror', (err) => errors.push(err.message));
  await compose.waitForSelector('.compose .editor');
  await expect(chips(compose, 'to')).toHaveText(['anna@test.invalid', 'joris@test.invalid']);
  await expect(chips(compose, 'cc')).toHaveText(['sanne@test.invalid']);

  const dropped = await compose.evaluate(() => {
    const ev = (type, dataTransfer) => new DragEvent(type, { dataTransfer, bubbles: true, cancelable: true });
    // anna's drag starts here and its end never arrives.
    document.querySelector('[data-rfield="to"] .recipient').dispatchEvent(ev('dragstart', new DataTransfer()));
    // A chip from another compose window, with that window's token.
    const dt = new DataTransfer();
    dt.setData('application/x-rukoo-recipient', 'another-compose-window');
    const cc = document.querySelector('[data-rfield="cc"]');
    cc.dispatchEvent(ev('dragenter', dt));
    cc.dispatchEvent(ev('dragover', dt));
    return !cc.dispatchEvent(ev('drop', dt));
  });
  // Taken, so nothing else in the window acts on it, but neither list changes.
  expect(dropped).toBe(true);
  await expect(chips(compose, 'to')).toHaveText(['anna@test.invalid', 'joris@test.invalid']);
  await expect(chips(compose, 'cc')).toHaveText(['sanne@test.invalid']);
  await expect(compose.locator('.drop-recipient, .recipient.dragging')).toHaveCount(0);

  // The chips are still drawn from the composer's own lists, and a drag of its own still moves one.
  await chips(compose, 'to').first().dragTo(compose.locator('[data-rfield="cc"]'));
  await expect(chips(compose, 'cc')).toHaveText(['sanne@test.invalid', 'anna@test.invalid']);
  await expect(chips(compose, 'to')).toHaveText(['joris@test.invalid']);
  expect(errors).toEqual([]);
});
