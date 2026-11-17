// The event dialog's date and time fields under a 12-hour clock (#311): the real app on the demo account, started with
// --lang=en-US, which gives Chromium's time field the "12:30 PM" format on every platform. The fields used to have a
// fixed 96px time field that cut that off to "12:30 P".
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

test.beforeEach(async () => {
  dataDir = tempDir('sem-e2e-caltime-');
  const env = { ...process.env, SEM_DATA_DIR: dataDir, SEM_HIDDEN: '1', LANG: 'en_US.UTF-8' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await electron.launch({ args: ['.', '--lang=en-US'], cwd: path.join(__dirname, '..', '..'), env });
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

const form = () => win.locator('.dialog .cal-form');
const remindersNotice = () => win.locator('.cal-notice', { hasText: 'Rukoo now reminds you of events' });

async function setTheme(theme) {
  await win.evaluate((t) => window.mail.call('updateSettings', { theme: t }), theme);
  await expect(win.locator('html')).toHaveClass(theme === 'light' ? /light/ : /^(?!.*light).*$/);
}

const resize = (width, height) => app.evaluate(({ BrowserWindow }, [w, h]) => BrowserWindow.getAllWindows()[0].setSize(w, h), [width, height]);
const zoomTo = (factor) => app.evaluate(({ BrowserWindow }, f) => BrowserWindow.getAllWindows()[0].webContents.setZoomFactor(f), factor);

async function expectWindowWidth(width, factor) {
  // With Windows title-bar chrome, the native window's content can be wider than its outer bounds.
  await expect.poll(async () => {
    const { outer, content, zoom } = await app.evaluate(({ BrowserWindow }) => {
      const w = BrowserWindow.getAllWindows()[0];
      return { outer: w.getSize()[0], content: w.getContentSize()[0], zoom: w.webContents.getZoomFactor() };
    });
    const viewport = await win.evaluate(() => innerWidth);
    return outer === width && zoom === factor && Math.abs(viewport - content / factor) <= 1;
  }).toBe(true);
}

// Each date and time field beside the width its own text needs: a copy of it in the same place, so the same rules
// style it, but out of the flow and as wide as its content.
const fields = () =>
  form().evaluate((f) =>
    [...f.querySelectorAll('.cal-dt input')].map((input) => {
      const copy = input.cloneNode();
      copy.style.cssText = 'position: absolute; visibility: hidden; width: max-content; flex: none';
      input.parentElement.append(copy);
      const needs = copy.offsetWidth;
      copy.remove();
      return { name: input.name, width: input.offsetWidth, needs, top: input.offsetTop };
    })
  );

// Every field shows its whole value; Starts and Ends share a row (rows 1) or Ends goes under Starts (rows 2); and the
// dialog doesn't scroll sideways.
async function expectWhole(rows, label) {
  await expect.poll(async () => (await fields()).filter((f) => f.width < f.needs).map((f) => `${f.name} ${f.width}px of ${f.needs}px`), { message: label }).toEqual([]);
  const top = Object.fromEntries((await fields()).map((f) => [f.name, f.top]));
  if (rows === 1) expect(top.endDate, `${label}: Ends beside Starts`).toBe(top.startDate);
  else expect(top.endDate, `${label}: Ends under Starts`).toBeGreaterThan(top.startDate);
  expect(await win.locator('.dialog .dialog-body').evaluate((b) => b.scrollWidth - b.clientWidth), `${label}: no sideways scroll`).toBeLessThanOrEqual(0);
}

async function shot(name, testInfo) {
  const zoomed = name.endsWith('zoom');
  for (const theme of ['dark', 'light']) {
    await setTheme(theme);
    await expectWhole(zoomed ? 2 : 1, `${name}, ${theme}`);
    const output = testInfo.outputPath(`${name}-${theme}.png`);
    if (zoomed) {
      // Playwright's page screenshot crops the Electron window to CSS pixels at non-default zoom.
      const image = await app.evaluate(async ({ BrowserWindow }) => (await BrowserWindow.getAllWindows()[0].capturePage()).toPNG().toString('base64'));
      fs.mkdirSync(path.dirname(output), { recursive: true });
      fs.writeFileSync(output, Buffer.from(image, 'base64'));
    } else {
      await win.locator('.dialog').screenshot({ path: output, animations: 'disabled' });
    }
  }
  await setTheme('dark');
}

test('the event dialog shows a 12-hour time whole, with its date, wide, narrow and zoomed, in both themes', async ({}, testInfo) => {
  await win.click('[data-mode="calendar"]');
  await expect(win.locator('.calendar [data-ev]', { hasText: 'Roadmap review' }).first()).toBeVisible({ timeout: 10000 });
  if (await remindersNotice().count()) {
    await remindersNotice().locator('button', { hasText: 'Got it' }).click();
    await expect(remindersNotice()).toHaveCount(0);
  }
  await win.locator('.cal-scroll').evaluate((el) => (el.scrollTop = 8 * 56));
  await win.locator('.calendar [data-ev]', { hasText: 'Lunch with Sanne' }).first().click();
  await win.locator('.cal-pop [data-pop="edit"]').click();
  await expect(form().locator('[name="startTime"]')).toHaveValue('12:30');

  // The run really has a 12-hour clock: the time field needs room for "PM", where a 24-hour "12:30" takes about 92px.
  const time = (await fields()).find((f) => f.name === 'startTime');
  expect(time.needs, 'the time field has the 12-hour format').toBeGreaterThan(100);

  await shot('wide', testInfo);
  // The narrowest main window: the dialog is still 600px wide, so the two still share a row.
  await resize(760, 800);
  await expectWindowWidth(760, 1);
  await shot('narrow', testInfo);
  // At 200% the page has half the content width: Ends goes under Starts, and every field stays whole.
  await zoomTo(2);
  await expectWindowWidth(760, 2);
  await shot('narrow-zoom', testInfo);
  await zoomTo(1);
});
