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

const bubble = (page) => page.locator('.link-bubble');
// Screenshots for the issue: in RUKOO_CAPTURE_DIR when it is set, and with the test's other output otherwise.
async function capture(page, name) {
  const dir = process.env.RUKOO_CAPTURE_DIR;
  if (dir) fs.mkdirSync(dir, { recursive: true });
  // Playwright's page capture crops to CSS pixels at 200% zoom. Capture the complete Electron window.
  const window = await app.browserWindow(page);
  const image = await window.evaluate(async (win) => (await win.capturePage()).toPNG().toString('base64'));
  await window.dispose();
  const file = dir ? path.join(dir, `${name}.png`) : test.info().outputPath(`${name}.png`);
  fs.writeFileSync(file, Buffer.from(image, 'base64'));
}
// A bar or bubble against the window, and against its anchor, the index-th match of anchor: how many rows its
// buttons take, whether its separator shows, and what is wrong with where it is. It belongs 8 pixels inside the
// window, with every button inside it, where placePopover() in editor-links.js puts a box of the size it shows: under
// the anchor, or above it where the window has no room below. A box measured at another size than it shows sits
// elsewhere. visible: the anchor is the part of it the panes and the window leave in view, as for an image.
const placed = (page, selector, anchor, index, visible) =>
  page.evaluate(
    ([selector, anchor, index, visible]) => {
      const el = document.querySelector(selector);
      const target = document.querySelectorAll(anchor)[index];
      let { left, top, bottom } = target.getClientRects()[0];
      if (visible) {
        for (let p = target.parentElement; p; p = p.parentElement) {
          if (!/auto|scroll|hidden|clip/.test(getComputedStyle(p).overflowY)) continue;
          const c = p.getBoundingClientRect();
          [left, top, bottom] = [Math.max(left, c.left), Math.max(top, c.top), Math.min(bottom, c.bottom)];
        }
        [top, bottom] = [Math.max(top, 0), Math.min(bottom, innerHeight)];
      }
      const b = el.getBoundingClientRect();
      const below = bottom + 6;
      const y = below + b.height <= innerHeight - 8 || top - 6 - b.height < 8 ? below : top - 6 - b.height;
      const want = { left: Math.max(8, Math.min(left, innerWidth - b.width - 8)), top: Math.max(8, Math.min(y, innerHeight - b.height - 8)) };
      const buttons = [...el.querySelectorAll('button')].map((x) => x.getBoundingClientRect());
      const problems = [
        b.left < 7.5 && `left ${b.left}`,
        b.right > innerWidth - 7.5 && `right ${b.right} in ${innerWidth}`,
        Math.abs(b.left - want.left) > 1 && `left ${b.left} for ${want.left}`,
        Math.abs(b.top - want.top) > 1 && `top ${b.top} for ${want.top}`,
        !buttons.every((x) => x.left >= b.left - 0.5 && x.right <= b.right + 0.5 && x.bottom <= b.bottom + 0.5) && 'a button outside'
      ].filter(Boolean);
      return { rows: new Set(buttons.map((x) => Math.round(x.top))).size, separator: getComputedStyle(el.querySelector('.sep')).display !== 'none', problems };
    },
    [selector, anchor, index, visible]
  );

// #319: a compose window 680 pixels wide at 200 percent is 340 CSS pixels, less than the Dutch image bar needs on one
// row. Its buttons wrap inside the window, and the link bubble, which shares its styles and its placing, fits too.
test('in a compose window 680 pixels wide at 200 percent the image bar and the link bubble stay inside it, in Dutch', async () => {
  await launch('nl');
  await win.evaluate(() => window.mail.call('updateSettings', { theme: 'dark' }));
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  const [page] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  await page.waitForSelector('.compose .editor');
  await insertImage(page);
  // The picture in a list starts further right. A bar that wraps is as wide as the room right of where it is, so
  // there it has to be measured at the window's edge to get the size it has once it moves.
  await image(page).evaluate((img) => {
    const list = document.createElement('ul');
    img.replaceWith(list);
    list.appendChild(document.createElement('li')).append(img);
  });
  // Two links after it, with a few words in front: a short address and one longer than the window.
  await editor(page).evaluate((el) => {
    el.insertAdjacentHTML(
      'beforeend',
      '<div>Een kort adres: <a href="https://a.io/">kort</a></div><div>Een lang adres: <a href="https://www.example.com/a/very/long/path/that/goes/on/to/the/report-2026.pdf">rapport</a></div>'
    );
  });
  const compose = await app.browserWindow(page);
  await compose.evaluate((w) => {
    // Size the content to 680 pixels, including on Windows where the content extends into the frame.
    w.setSize(680, 720);
    const frameWidth = w.getSize()[0] - w.getContentSize()[0];
    if (frameWidth) w.setSize(680 + frameWidth, 720);
  });
  const zoom = async (factor) => {
    await compose.evaluate((w, f) => w.webContents.setZoomFactor(f), factor);
    await expect.poll(() => page.evaluate(() => innerWidth)).toBe(680 / factor);
  };
  await zoom(2);

  // The image bar wraps inside the window, without the separator that would end its first row.
  const imageBar = () => placed(page, '.img-bubble', '.editor img', 0, true);
  await image(page).click();
  await expect(bar(page)).toBeVisible();
  expect(await bar(page).locator('button').allTextContents()).toEqual([LABELS.nl.small, LABELS.nl.fit, LABELS.nl.original, LABELS.nl.remove]);
  await expect.poll(imageBar).toEqual({ rows: 2, separator: false, problems: [] });
  await capture(page, 'image-bar-680-200-nl-dark');
  // With room for one row it takes one again, with its separator, under the picture. Back at 200 percent it wraps
  // again from there: measured where it was, it would come out narrower and run to the window's edge.
  await zoom(1);
  await expect.poll(imageBar).toEqual({ rows: 1, separator: true, problems: [] });
  await zoom(2);
  await expect.poll(imageBar).toEqual({ rows: 2, separator: false, problems: [] });

  // The link bubble has the bar's styles and placing, on one row: the address gives way.
  const links = page.locator('.editor a');
  const site = page.locator('.link-bubble .link-go-site bdi');
  await links.nth(0).click();
  await expect(bubble(page)).toBeVisible();
  await expect(site).toHaveText('https://a.io');
  await expect.poll(() => placed(page, '.link-bubble', '.editor a', 0, false)).toEqual({ rows: 1, separator: true, problems: [] });
  await links.nth(1).click();
  await expect(site).toHaveText('https://www.example.com');
  await expect.poll(() => placed(page, '.link-bubble', '.editor a', 1, false)).toEqual({ rows: 1, separator: true, problems: [] });
  await capture(page, 'link-bubble-680-200-nl-dark');

  await win.evaluate(() => window.mail.call('updateSettings', { theme: 'light' }));
  await expect(page.locator('html')).toHaveClass(/light/);
  await links.nth(1).click();
  await expect(bubble(page)).toBeVisible();
  await expect.poll(() => placed(page, '.link-bubble', '.editor a', 1, false)).toEqual({ rows: 1, separator: true, problems: [] });
  await capture(page, 'link-bubble-680-200-nl-light');
  await image(page).click();
  await expect(bar(page)).toBeVisible();
  await expect.poll(imageBar).toEqual({ rows: 2, separator: false, problems: [] });
  await capture(page, 'image-bar-680-200-nl-light');
  await compose.dispose();
});

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

test('a linked image keeps its link when it changes size, and Ctrl+Z keeps it too', async () => {
  await launch('en');
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  await insertImage(win);
  await image(win).evaluate((img) => {
    const a = document.createElement('a');
    a.href = 'https://example.org/';
    img.replaceWith(a);
    a.append(img);
  });
  const link = () => image(win).evaluate((img) => img.parentElement.closest('a')?.getAttribute('href') || null);
  await image(win).click();
  await action(win, 'small').click();
  await expect.poll(() => size(win)).toEqual({ attr: '200', shown: 200, ratio: 2 });
  expect(await link()).toBe('https://example.org/');
  await action(win, 'fit').click();
  await expect.poll(() => size(win)).toEqual({ attr: '560', shown: 560, ratio: 2 });
  expect(await link()).toBe('https://example.org/');
  await undo(win);
  await expect.poll(() => size(win)).toEqual({ attr: '200', shown: 200, ratio: 2 });
  expect(await link()).toBe('https://example.org/');
  expect(await editor(win).locator('a').count()).toBe(1);
});

test('a save during a drag waits for it, so a drag that Escape cancels never reaches the draft', async () => {
  await launch('en');
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  await win.locator('.composer [data-field="subject"]').fill('A dragged picture');
  await insertImage(win);
  await image(win).click();
  await action(win, 'fit').click();
  await expect.poll(() => size(win)).toEqual({ attr: '560', shown: 560, ratio: 2 });

  const drop = await dragCorner(win, 'se', -160);
  await expect.poll(async () => (await size(win)).shown).toBe(400);
  await win.keyboard.press('ControlOrMeta+s');
  // The save holds off while the corner is down.
  await win.waitForTimeout(800);
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  await win.keyboard.press('Escape');
  await drop();
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await expect.poll(() => size(win)).toEqual({ attr: '560', shown: 560, ratio: 2 });
  await win.locator('.composer [data-c="close"]').click();
  await expect(win.locator('.composer')).toHaveCount(0);

  await win.click('[data-view="drafts"]');
  await win.locator('.item', { hasText: 'A dragged picture' }).first().click();
  await win.locator('[data-reader="edit"]').click();
  await win.waitForSelector('.composer .editor');
  await expect.poll(() => size(win)).toEqual({ attr: '560', shown: 560, ratio: 2 });
});
