// Images in the composer's text: a click selects one, which frames it with a handle on each corner and puts a bar
// under it with Small, Best fit, Original size and Remove. In the editor in the reading pane and in a compose window,
// in English and in Dutch: the real app with a throwaway profile and the demo account.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

const LABELS = {
  en: { small: 'Small', fit: 'Best fit', original: 'Original size', remove: 'Remove', bar: 'Image' },
  nl: { small: 'Klein', fit: 'Passend', original: 'Oorspronkelijke grootte', remove: 'Verwijderen', bar: 'Afbeelding' }
};
// The picture the image button inserts: wider than Best fit, so every size differs from the others.
const NATURAL = { width: 800, height: 400 };

async function launch(language) {
  dataDir = tempDir('rukoo-compose-images-e2e-');
  fs.mkdirSync(path.join(dataDir, 'data'));
  fs.writeFileSync(path.join(dataDir, 'data', 'settings.json'), JSON.stringify({ language }));
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.'], cwd: path.join(__dirname, '..', '..'), env });
  win = await app.firstWindow();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 950));
  await win.waitForSelector('.provider-grid');
  await win.click('[data-s="demo"]');
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
  // The file picker answers with a picture made here, without opening.
  const file = path.join(dataDir, 'picture.png');
  const png = await app.evaluate(({ nativeImage }, { width, height }) => {
    const pixels = Buffer.alloc(width * height * 4);
    for (let i = 0; i < pixels.length; i += 4) pixels.writeUInt32BE(i % (width * 4) < width * 2 ? 0x3366ccff : 0xcc8833ff, i);
    return nativeImage.createFromBitmap(pixels, { width, height }).toPNG().toString('base64');
  }, NATURAL);
  fs.writeFileSync(file, Buffer.from(png, 'base64'));
  await app.evaluate(({ dialog }, file) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [file] });
  }, file);
}

test.afterEach(async () => {
  await closeApp(app);
  removeDir(dataDir);
});

// The two places the composer runs: the reading pane, in English, and a window of its own, in Dutch.
const SURFACES = [
  {
    name: 'in the reading pane',
    language: 'en',
    async open() {
      await launch('en');
      await win.click('[data-action="compose"]');
      await win.waitForSelector('.composer .editor');
      return win;
    }
  },
  {
    name: 'in a compose window, in Dutch',
    language: 'nl',
    async open() {
      await launch('nl');
      await win.click('[data-action="compose"]');
      await win.waitForSelector('.composer .editor');
      const [compose] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
      await compose.waitForSelector('.compose .editor');
      return compose;
    }
  }
];

const editor = (page) => page.locator('.editor');
const image = (page) => page.locator('.editor img');
const frame = (page) => page.locator('.img-frame');
const bar = (page) => page.locator('.img-bubble');
const action = (page, name) => page.locator(`.img-bubble [data-a="${name}"]`);
const undo = (page) => page.keyboard.press('ControlOrMeta+z');
// The image's size in the text: its width attribute and the width it shows at.
const size = (page) => image(page).evaluate((img) => ({ attr: img.getAttribute('width'), shown: Math.round(img.getBoundingClientRect().width), ratio: Math.round((img.getBoundingClientRect().width / img.getBoundingClientRect().height) * 10) / 10 }));
const box = (locator) => locator.evaluate((el) => { const r = el.getBoundingClientRect(); return { left: Math.round(r.left), top: Math.round(r.top), width: Math.round(r.width), height: Math.round(r.height) }; });

async function insertImage(page) {
  await page.click('.editor');
  await page.keyboard.type('Above the picture.');
  await page.keyboard.press('Enter');
  await page.click('[data-c="image"]');
  await expect(image(page)).toHaveCount(1);
  await expect.poll(() => image(page).evaluate((img) => img.naturalWidth)).toBe(NATURAL.width);
}
// Drags a corner of the frame sideways by dx pixels.
async function dragCorner(page, corner, dx) {
  const at = await page.locator(`.img-handle[data-corner="${corner}"]`).boundingBox();
  const x = at.x + at.width / 2;
  const y = at.y + at.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y + 3, { steps: 3 });
  await page.mouse.move(x + dx, y + 6, { steps: 3 });
  return () => page.mouse.up();
}

for (const surface of SURFACES) {
  const L = LABELS[surface.language];

  test(`a selected image gets a frame and a bar with sizes ${surface.name}`, async () => {
    const page = await surface.open();
    await insertImage(page);
    await expect(frame(page)).toBeHidden();
    await expect(bar(page)).toBeHidden();

    await image(page).click();
    await expect(frame(page)).toBeVisible();
    await expect(bar(page)).toBeVisible();
    await expect(bar(page)).toHaveAttribute('aria-label', L.bar);
    expect(await bar(page).locator('button').allTextContents()).toEqual([L.small, L.fit, L.original, L.remove]);
    // The frame is the image's box, and the bar sits under it. The formatting bar stays away.
    await expect.poll(() => box(frame(page))).toEqual(await box(image(page)));
    const img = await box(image(page));
    expect((await box(bar(page))).top).toBeGreaterThanOrEqual(img.top + img.height);
    await expect(page.locator('.float-bar')).toBeHidden();
    await expect(action(page, 'original')).toHaveAttribute('aria-pressed', 'true');

    await action(page, 'small').click();
    await expect.poll(() => size(page)).toEqual({ attr: '200', shown: 200, ratio: 2 });
    await expect(action(page, 'small')).toHaveAttribute('aria-pressed', 'true');
    // The image stays selected, with the frame around its new size.
    await expect.poll(() => box(frame(page))).toEqual(await box(image(page)));
    await action(page, 'fit').click();
    await expect.poll(() => size(page)).toEqual({ attr: '560', shown: 560, ratio: 2 });
    await action(page, 'original').click();
    await expect.poll(async () => (await size(page)).attr).toBe(null);
    expect((await size(page)).shown).toBeGreaterThan(560);
    // Ctrl+Z takes one size back at a time.
    await undo(page);
    await expect.poll(() => size(page)).toEqual({ attr: '560', shown: 560, ratio: 2 });
    await undo(page);
    await expect.poll(() => size(page)).toEqual({ attr: '200', shown: 200, ratio: 2 });

    // Neither the frame nor the bar is part of the text, and the width is the image's own.
    const html = await editor(page).evaluate((e) => e.innerHTML);
    expect(html).not.toMatch(/img-frame|img-handle|img-bubble/);
    expect(html).toMatch(/<img [^>]*width="200"[^>]*style="[^"]*width: 200px/);
  });

  test(`a corner drags an image to a size, and Escape lets go of it ${surface.name}`, async () => {
    const page = await surface.open();
    await insertImage(page);
    await image(page).click();
    await action(page, 'fit').click();
    await expect.poll(() => size(page)).toEqual({ attr: '560', shown: 560, ratio: 2 });

    // While it drags, the image shows the size and the bar is away.
    let drop = await dragCorner(page, 'se', -160);
    await expect.poll(async () => (await size(page)).shown).toBe(400);
    await expect(bar(page)).toBeHidden();
    await drop();
    await expect.poll(() => size(page)).toEqual({ attr: '400', shown: 400, ratio: 2 });
    await expect(bar(page)).toBeVisible();
    await expect(action(page, 'fit')).toHaveAttribute('aria-pressed', 'false');
    // A corner on the left grows the image as it moves left.
    drop = await dragCorner(page, 'nw', -60);
    await drop();
    await expect.poll(() => size(page)).toEqual({ attr: '460', shown: 460, ratio: 2 });
    // Escape during a drag leaves the size alone.
    drop = await dragCorner(page, 'se', -200);
    await page.keyboard.press('Escape');
    await drop();
    await expect.poll(() => size(page)).toEqual({ attr: '460', shown: 460, ratio: 2 });
    // It never gets smaller than a handle can still grab.
    drop = await dragCorner(page, 'se', -700);
    await drop();
    await expect.poll(() => size(page)).toEqual({ attr: '24', shown: 24, ratio: 2 });
    await undo(page);
    await expect.poll(() => size(page)).toEqual({ attr: '460', shown: 460, ratio: 2 });

    // Escape lets go of the image and keeps the composer open; typing goes on after it.
    await image(page).click();
    await expect(frame(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(frame(page)).toBeHidden();
    await expect(bar(page)).toBeHidden();
    await expect(editor(page)).toBeVisible();
    await expect(image(page)).toHaveCount(1);
    // A click in the text lets go as well.
    await image(page).click();
    await expect(frame(page)).toBeVisible();
    await editor(page).getByText('Above the picture.').click();
    await expect(frame(page)).toBeHidden();
  });

  test(`Remove and the Delete key take an image out, and Ctrl+Z brings it back ${surface.name}`, async () => {
    const page = await surface.open();
    await insertImage(page);
    await image(page).click();
    await action(page, 'remove').click();
    await expect(image(page)).toHaveCount(0);
    await expect(frame(page)).toBeHidden();
    await expect(bar(page)).toBeHidden();
    await expect(editor(page)).toContainText('Above the picture.');
    await undo(page);
    await expect(image(page)).toHaveCount(1);

    await image(page).click();
    await expect(frame(page)).toBeVisible();
    await page.keyboard.press('Delete');
    await expect(image(page)).toHaveCount(0);
    await expect(frame(page)).toBeHidden();

    // The bar works from the keyboard: Tab from the text, arrows between its buttons, Enter to choose.
    await undo(page);
    await image(page).click();
    await page.keyboard.press('Tab');
    await expect(action(page, 'small')).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(action(page, 'fit')).toBeFocused();
    await page.keyboard.press('Enter');
    await expect.poll(() => size(page)).toEqual({ attr: '560', shown: 560, ratio: 2 });
  });
}

test('a draft keeps the size of its image when it is saved, closed and opened again, and the reader shows it at that size', async () => {
  await launch('en');
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  await win.locator('.composer [data-field="subject"]').fill('A sized picture');
  await insertImage(win);
  await image(win).click();
  await action(win, 'small').click();
  await expect.poll(() => size(win)).toEqual({ attr: '200', shown: 200, ratio: 2 });
  await win.keyboard.press('ControlOrMeta+s');
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await win.locator('.composer [data-c="close"]').click();
  await expect(win.locator('.composer')).toHaveCount(0);

  await win.click('[data-view="drafts"]');
  await win.locator('.item', { hasText: 'A sized picture' }).first().click();
  // The reader shows it at that size too.
  const shown = win.frameLocator('.mail-frame').locator('img');
  await expect.poll(() => shown.evaluate((img) => Math.round(img.getBoundingClientRect().width))).toBe(200);
  await win.locator('[data-reader="edit"]').click();
  await win.waitForSelector('.composer .editor');
  await expect.poll(() => size(win)).toEqual({ attr: '200', shown: 200, ratio: 2 });
  // The reopened image sizes like a new one.
  await image(win).click();
  await expect(action(win, 'small')).toHaveAttribute('aria-pressed', 'true');
  await action(win, 'fit').click();
  await expect.poll(() => size(win)).toEqual({ attr: '560', shown: 560, ratio: 2 });
  const stored = await editor(win).evaluate((e) => e.innerHTML);
  expect(stored).not.toMatch(/img-frame|img-handle|img-bubble/);
});
