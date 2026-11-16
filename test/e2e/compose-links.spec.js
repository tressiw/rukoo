// Links in the composer's text: addresses become links as they are pasted or typed, an address pasted over a selection
// links it, Ctrl+K opens a popover with the text and the address, and a link at the caret gets a bubble to open,
// change or remove it. In the editor in the reading pane and in a compose window, in English and in Dutch: the real
// app with a throwaway profile and the demo account. The clipboard is the real one.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

const LABELS = {
  en: { text: 'Text', link: 'Link', apply: 'Apply', change: 'Change', remove: 'Remove', open: 'Go to link: ', insert: 'Insert link', edit: 'Edit link', cantOpen: 'Rukoo only opens web and email links.' },
  nl: { text: 'Tekst', link: 'Link', apply: 'Toepassen', change: 'Wijzigen', remove: 'Verwijderen', open: 'Link openen: ', insert: 'Link invoegen', edit: 'Link bewerken', cantOpen: 'Rukoo opent alleen web- en e-maillinks.' }
};

async function launch(language) {
  dataDir = tempDir('rukoo-compose-links-e2e-');
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
  // Nothing opens in the test computer's browser: main only records what it was asked to open.
  await app.evaluate(({ shell }) => {
    global.__opened = [];
    shell.openExternal = async (url) => {
      global.__opened.push(url);
    };
  });
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

const opened = () => app.evaluate(() => global.__opened);
// data: { text, html }.
const setClipboard = (data) =>
  app.evaluate(({ clipboard, ClipboardItem }, data) => {
    const item = { 'text/plain': data.text };
    if (data.html) item['text/html'] = data.html;
    return clipboard.write([new ClipboardItem(item)]);
  }, data);
async function paste(page, data) {
  await setClipboard(data);
  await page.keyboard.press('ControlOrMeta+v');
}
const undo = (page) => page.keyboard.press('ControlOrMeta+z');

const editor = (page) => page.locator('.editor');
const pop = (page) => page.locator('.link-pop');
const bubble = (page) => page.locator('.link-bubble');
const field = (page, name) => page.locator(`.link-pop [data-f="${name}"]`);
// The lines of the text as its blocks and line breaks make them, with the editor's no-break spaces as ordinary ones.
const lines = (page) =>
  editor(page).evaluate((root) => {
    const BLOCK = /^(DIV|P|LI|UL|OL|H[1-6]|BLOCKQUOTE)$/;
    const out = [];
    let line = null;
    const end = () => {
      if (line !== null) out.push(line);
      line = null;
    };
    (function walk(node) {
      for (const child of node.childNodes) {
        if (child.nodeType === Node.ELEMENT_NODE && BLOCK.test(child.tagName)) {
          end();
          const before = out.length;
          walk(child);
          end();
          if (out.length === before) out.push('');
        } else if (child.nodeName === 'BR') {
          if (line === null) line = '';
          // The last one in a block only keeps an empty line open.
          if (child.nextSibling) end();
        } else line = (line || '') + child.textContent;
      }
    })(root);
    end();
    return out.map((l) => l.replace(/ /g, ' ').trimEnd());
  });
// Every link in the text: [address, text].
const links = (page) => editor(page).evaluate((e) => [...e.querySelectorAll('a')].map((a) => [a.getAttribute('href'), a.textContent]));
const selected = (page) => page.evaluate(() => String(getSelection()));
const caretOnly = (page) => page.evaluate(() => getSelection().isCollapsed);
const editorFocused = (page) => page.evaluate(() => document.activeElement === document.querySelector('.editor'));

async function write(page, text) {
  await page.click('.editor');
  await page.keyboard.type(text);
}
// Selects part of the text, or with at puts the caret that many characters into it. Returns where it is on screen.
async function select(page, part, at = null) {
  return page.evaluate(
    ([part, at]) => {
      const root = document.querySelector('.editor');
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const from = node.data.indexOf(part);
        if (from < 0) continue;
        const range = document.createRange();
        range.setStart(node, at === null ? from : from + at);
        range.setEnd(node, at === null ? from + part.length : from + at);
        root.focus();
        getSelection().removeAllRanges();
        getSelection().addRange(range);
        const r = range.getClientRects()[0];
        // The page hears of the new selection a moment later, as it does after a key or a click.
        return new Promise((done) => setTimeout(() => done({ x: r.left + r.width / 2, y: r.top + r.height / 2, bottom: r.bottom }), 30));
      }
      throw new Error(`"${part}" is not in the editor`);
    },
    [part, at]
  );
}
const empty = async (page) => {
  await editor(page).evaluate((e) => {
    e.innerHTML = '<div><br></div>';
  });
  await page.click('.editor');
};
// Puts html in the editor as pasted or signature HTML would bring it, with the focus in the editor.
const fill = (page, html) =>
  editor(page).evaluate((e, html) => {
    e.innerHTML = html;
    e.focus();
  }, html);
// Selects a node in the editor: its contents, or with whole the node itself.
const selectNode = (page, selector, whole = false) =>
  editor(page).evaluate(
    (root, [selector, whole]) => {
      const range = document.createRange();
      if (whole) range.selectNode(root.querySelector(selector));
      else range.selectNodeContents(root.querySelector(selector));
      root.focus();
      getSelection().removeAllRanges();
      getSelection().addRange(range);
      return new Promise((done) => setTimeout(done, 30));
    },
    [selector, whole]
  );
// A 12 by 12 pixel image, as a signature's logo.
const LOGO = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAwAAAAMCAIAAADZF8uwAAAAE0lEQVR4nGPQyLtDEDGMKhqMigAiyNAhtCcBpAAAAABJRU5ErkJggg==';

for (const surface of SURFACES) {
  const L = LABELS[surface.language];

  test(`pasted addresses become links and one undo takes the paste back, ${surface.name}`, async () => {
    const page = await surface.open();
    await write(page, 'Start: ');
    await paste(page, { text: 'Read https://rukoo.io/docs, then (www.example.com).' });
    await expect.poll(() => links(page)).toEqual([
      ['https://rukoo.io/docs', 'https://rukoo.io/docs'],
      ['https://www.example.com', 'www.example.com']
    ]);
    // The comma, the brackets and the full stop are text, and the paste is one step: no bubble comes up for it.
    expect(await lines(page)).toEqual(['Start: Read https://rukoo.io/docs, then (www.example.com).']);
    await expect(bubble(page)).toBeHidden();
    await expect(page.locator('.composer')).toHaveAttribute('data-dirty', 'true');
    // Typing goes on behind the paste.
    await page.keyboard.type(' Done');
    expect(await lines(page)).toEqual(['Start: Read https://rukoo.io/docs, then (www.example.com). Done']);
    await undo(page);
    await undo(page);
    await expect.poll(() => lines(page)).toEqual(['Start:']);
    expect(await links(page)).toEqual([]);

    // Several lines stay several lines.
    await empty(page);
    await paste(page, { text: 'one https://a.example/x?y=1&z=2\n\nthree www.b.example!' });
    await expect.poll(() => lines(page)).toEqual(['one https://a.example/x?y=1&z=2', '', 'three www.b.example!']);
    expect(await links(page)).toEqual([
      ['https://a.example/x?y=1&z=2', 'https://a.example/x?y=1&z=2'],
      ['https://www.b.example', 'www.b.example']
    ]);
    await undo(page);
    await expect.poll(() => links(page)).toEqual([]);
    expect(await lines(page)).toEqual(['']);

    // Pasted HTML keeps the links it came with and gets no others.
    await empty(page);
    await paste(page, { text: 'Docs at https://rukoo.io', html: '<a href="https://rukoo.io/docs">Docs</a> at https://rukoo.io' });
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'Docs']]);

    // Text pasted into a link stays that link's text.
    await select(page, 'Docs', 2);
    await paste(page, { text: 'www.example.com' });
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'Dowww.example.comcs']]);
    expect(await editor(page).locator('a a').count()).toBe(0);
  });

  test(`an address pasted over selected text links that text, ${surface.name}`, async () => {
    const page = await surface.open();
    await write(page, 'Read the docs today');
    await select(page, 'the docs');
    await paste(page, { text: ' https://rukoo.io/docs\n' });
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'the docs']]);
    expect(await lines(page)).toEqual(['Read the docs today']);
    // The caret rests behind the link: no formatting bar and no bubble.
    expect(await caretOnly(page)).toBe(true);
    await expect(page.locator('.float-bar')).toBeHidden();
    await expect(bubble(page)).toBeHidden();

    // Over a selection in a link, the address becomes that link's address.
    await select(page, 'docs');
    await paste(page, { text: 'www.example.com' });
    await expect.poll(() => links(page)).toEqual([['https://www.example.com', 'the docs']]);
    await undo(page);
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'the docs']]);
    await undo(page);
    await expect.poll(() => links(page)).toEqual([]);
    expect(await lines(page)).toEqual(['Read the docs today']);

    // An address with words around it is text like any other: it replaces the selection.
    await select(page, 'today');
    await paste(page, { text: 'at https://rukoo.io now' });
    await expect.poll(() => lines(page)).toEqual(['Read the docs at https://rukoo.io now']);
    expect(await links(page)).toEqual([['https://rukoo.io', 'https://rukoo.io']]);
  });

  test(`a typed address becomes a link at the space, Enter or Tab, ${surface.name}`, async () => {
    const page = await surface.open();
    await write(page, 'Go to www.example.com');
    expect(await links(page)).toEqual([]);
    await page.keyboard.type(' ');
    await expect.poll(() => links(page)).toEqual([['https://www.example.com', 'www.example.com']]);
    await expect(bubble(page)).toBeHidden();
    // Undo takes back the link and keeps the text, with the caret behind the space.
    await undo(page);
    await expect.poll(() => links(page)).toEqual([]);
    expect(await caretOnly(page)).toBe(true);
    await page.keyboard.type('now');
    expect(await lines(page)).toEqual(['Go to www.example.com now']);
    expect(await links(page)).toEqual([]);

    // Enter ends an address too. The full stop stays outside, and the caret goes on on the new line.
    await page.keyboard.press('Enter');
    await page.keyboard.type('See (https://rukoo.io/docs).');
    await page.keyboard.press('Enter');
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'https://rukoo.io/docs']]);
    await page.keyboard.type('next www.example.org');
    expect(await lines(page)).toEqual(['Go to www.example.com now', 'See (https://rukoo.io/docs).', 'next www.example.org']);
    // Tab leaves the text and links what was typed.
    await page.keyboard.press('Tab');
    await expect.poll(() => links(page)).toEqual([
      ['https://rukoo.io/docs', 'https://rukoo.io/docs'],
      ['https://www.example.org', 'www.example.org']
    ]);
    expect(await editorFocused(page)).toBe(false);

    // A space typed in a link, or after words that are no address, links nothing.
    await select(page, 'rukoo.io/docs', 5);
    await page.keyboard.type(' ');
    await select(page, 'now', 3);
    await page.keyboard.type(' example.com ');
    expect((await links(page)).length).toBe(2);
    expect(await editor(page).locator('a a').count()).toBe(0);
  });

  test(`Ctrl+K asks for the text and the address, and changes a link in place, ${surface.name}`, async () => {
    const page = await surface.open();
    await write(page, 'Read the docs today');
    const at = await select(page, 'the docs');
    await page.keyboard.press('ControlOrMeta+k');
    await expect(pop(page)).toBeVisible();
    await expect(pop(page)).toHaveAttribute('aria-label', L.insert);
    await expect(pop(page).locator('label span')).toHaveText([L.text, L.link]);
    await expect(pop(page).locator('[data-a="apply"]')).toHaveText(L.apply);
    await expect(field(page, 'text')).toHaveValue('the docs');
    await expect(field(page, 'url')).toHaveValue('');
    await expect(field(page, 'url')).toBeFocused();
    await expect(pop(page).locator('[data-a="apply"]')).toBeDisabled();
    // Under the selection, with the formatting bar out of the way.
    const box = await pop(page).boundingBox();
    expect(box.y).toBeGreaterThan(at.bottom);
    expect(box.y - at.bottom).toBeLessThan(16);
    await expect(page.locator('.float-bar')).toBeHidden();

    // Escape cancels and gives the selection back; the composer stays open.
    await page.keyboard.press('Escape');
    await expect(pop(page)).toBeHidden();
    expect(await selected(page)).toBe('the docs');
    expect(await editorFocused(page)).toBe(true);
    await expect(editor(page)).toBeVisible();
    expect(await links(page)).toEqual([]);

    // By keyboard alone: the address, back to the text, another text, Enter. Tab stays inside the popover.
    await page.keyboard.press('ControlOrMeta+k');
    await page.keyboard.type('rukoo.io/docs');
    await page.keyboard.press('Tab');
    await expect(pop(page).locator('[data-a="apply"]')).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(field(page, 'text')).toBeFocused();
    await page.keyboard.type('our guide');
    await page.keyboard.press('Enter');
    await expect(pop(page)).toBeHidden();
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'our guide']]);
    expect(await lines(page)).toEqual(['Read our guide today']);
    expect(await editorFocused(page)).toBe(true);
    await expect(bubble(page)).toBeHidden();
    await expect(page.locator('.composer')).toHaveAttribute('data-dirty', 'true');
    // One step back.
    await undo(page);
    await expect.poll(() => lines(page)).toEqual(['Read the docs today']);
    expect(await links(page)).toEqual([]);
    await page.keyboard.press('ControlOrMeta+Shift+z');
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'our guide']]);

    // With the caret in a link, Ctrl+K changes that link: both fields come filled in.
    await select(page, 'our guide', 3);
    await page.keyboard.press('ControlOrMeta+k');
    await expect(pop(page)).toHaveAttribute('aria-label', L.edit);
    await expect(field(page, 'text')).toHaveValue('our guide');
    await expect(field(page, 'url')).toHaveValue('https://rukoo.io/docs');
    await field(page, 'text').fill('the guide');
    await field(page, 'url').fill('https://rukoo.io/guide');
    await pop(page).locator('[data-a="apply"]').click();
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/guide', 'the guide']]);
    expect(await lines(page)).toEqual(['Read the guide today']);
    expect(await editorFocused(page)).toBe(true);

    // Only another address: the text keeps its formatting.
    await select(page, 'guide');
    await page.keyboard.press('ControlOrMeta+b');
    await select(page, 'the ', 1);
    await page.keyboard.press('ControlOrMeta+k');
    await expect(field(page, 'text')).toHaveValue('the guide');
    await field(page, 'url').fill('www.example.com/guide');
    await page.keyboard.press('Enter');
    await expect.poll(() => links(page)).toEqual([['https://www.example.com/guide', 'the guide']]);
    expect(await editor(page).locator('a *').evaluateAll((els) => els.some((el) => Number(getComputedStyle(el).fontWeight) >= 600))).toBe(true);

    // At a caret with the Text field empty, the address is the text. An email address becomes a mailto: link.
    await page.keyboard.press('ControlOrMeta+End');
    await page.keyboard.type(' or ');
    await page.keyboard.press('ControlOrMeta+k');
    await expect(field(page, 'text')).toHaveValue('');
    await page.keyboard.type('demo@example.com');
    await page.keyboard.press('Enter');
    await expect.poll(() => links(page)).toEqual([
      ['https://www.example.com/guide', 'the guide'],
      ['mailto:demo@example.com', 'demo@example.com']
    ]);

    // A selected address fills the Link field, and the toolbar's button opens the same popover.
    await page.keyboard.type(' www.example.org');
    await select(page, 'www.example.org');
    await page.locator('.float-bar [data-c="link"]').click();
    await expect(field(page, 'url')).toHaveValue('www.example.org');
    // A click in the text closes it and moves the caret there.
    await editor(page).click({ position: { x: 4, y: 4 } });
    await expect(pop(page)).toBeHidden();
    expect((await links(page)).length).toBe(2);
  });

  test(`a link at the caret gets a bubble to open, change or remove it, ${surface.name}`, async () => {
    const page = await surface.open();
    const long = 'https://www.example.com/a/very/long/path/that/goes/on/and/on/and/on/report-2026.pdf';
    await write(page, 'Read ');
    await page.keyboard.press('ControlOrMeta+k');
    await field(page, 'text').fill('the docs');
    await field(page, 'url').fill('https://rukoo.io/docs');
    await page.keyboard.press('Enter');
    await page.keyboard.type(' and ');
    await page.keyboard.press('ControlOrMeta+k');
    await field(page, 'text').fill('the report');
    await field(page, 'url').fill(long);
    await page.keyboard.press('Enter');
    await page.keyboard.type(' today');
    await expect(bubble(page)).toBeHidden();
    const url = page.url();

    // A click in a link opens the bubble under it and follows nothing.
    const docs = editor(page).locator('a', { hasText: 'the docs' });
    await docs.click();
    await expect(bubble(page)).toBeVisible();
    await expect(bubble(page).locator('button')).toHaveText(['https://rukoo.io/docs', L.change, L.remove]);
    await expect(bubble(page).locator('[data-a="go"]')).toHaveAttribute('aria-label', `${L.open}https://rukoo.io/docs`);
    const link = await docs.boundingBox();
    const box = await bubble(page).boundingBox();
    expect(box.y).toBeGreaterThan(link.y + link.height);
    expect(box.y - (link.y + link.height)).toBeLessThan(16);
    await page.waitForTimeout(300);
    expect(page.url()).toBe(url);
    expect(await opened()).toEqual([]);

    // Go to link asks main to open it in the browser; the window stays where it is.
    await bubble(page).locator('[data-a="go"]').click();
    await expect.poll(opened).toEqual(['https://rukoo.io/docs']);
    expect(page.url()).toBe(url);
    expect(await editorFocused(page)).toBe(true);

    // Escape closes the bubble and not the composer. It stays closed until the caret has left the link.
    await docs.click();
    await expect(bubble(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(bubble(page)).toBeHidden();
    await expect(editor(page)).toBeVisible();
    await page.keyboard.press('ArrowLeft');
    await expect(bubble(page)).toBeHidden();
    await select(page, 'Read', 2);
    await select(page, 'the docs', 4);
    await expect(bubble(page)).toBeVisible();
    // Typing closes it, and it stays closed while the caret is in that link.
    await page.keyboard.type('x');
    await expect(bubble(page)).toBeHidden();
    await undo(page);
    await select(page, 'the docs', 2);
    await expect(bubble(page)).toBeHidden();

    // A selection in a link gets the formatting bar instead.
    await select(page, 'Read', 2);
    await select(page, 'the docs', 4);
    await expect(bubble(page)).toBeVisible();
    await select(page, 'docs');
    await expect(bubble(page)).toBeHidden();
    await expect(page.locator('.float-bar')).toBeVisible();

    // A long address loses its middle; the whole address is in the label.
    await editor(page).locator('a', { hasText: 'the report' }).click();
    await expect(bubble(page).locator('.link-go-url')).toHaveText(/^https:\/\/www\.example\.com\/a\/.*….*report-2026\.pdf$/);
    await expect(bubble(page).locator('[data-a="go"]')).toHaveAttribute('aria-label', `${L.open}${long}`);
    // By keyboard: Tab goes from the text into the bubble, the arrows move along it and Enter presses.
    await page.keyboard.press('Tab');
    await expect(bubble(page).locator('[data-a="go"]')).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(bubble(page).locator('[data-a="change"]')).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(pop(page)).toBeVisible();
    await expect(bubble(page)).toBeHidden();
    await expect(field(page, 'text')).toHaveValue('the report');
    await expect(field(page, 'url')).toHaveValue(long);
    // Escape goes back to the text, where the bubble returns.
    await page.keyboard.press('Escape');
    await expect(pop(page)).toBeHidden();
    expect(await editorFocused(page)).toBe(true);
    await expect(bubble(page)).toBeVisible();

    // Remove takes the link off and keeps its text; one undo brings it back.
    await bubble(page).locator('[data-a="remove"]').click();
    await expect(bubble(page)).toBeHidden();
    await expect.poll(() => links(page)).toEqual([['https://rukoo.io/docs', 'the docs']]);
    expect(await lines(page)).toEqual(['Read the docs and the report today']);
    expect(await editorFocused(page)).toBe(true);
    await undo(page);
    await expect.poll(async () => (await links(page)).length).toBe(2);

    // Scrolling the text closes the bubble.
    await page.keyboard.press('ControlOrMeta+End');
    for (let i = 0; i < 45; i++) await page.keyboard.press('Enter');
    await page.locator('.composer-scroll').evaluate((el) => (el.scrollTop = 0));
    await docs.click();
    await expect(bubble(page)).toBeVisible();
    await page.mouse.wheel(0, 120);
    await expect(bubble(page)).toBeHidden();
  });

  test(`another address keeps a linked image and the link's formatting, ${surface.name}`, async () => {
    const page = await surface.open();
    // A signature's logo in a link, selected: Ctrl+K changes that link, and only its address changes.
    await fill(page, `<div>Logo: <a href="https://old.example/"><img src="${LOGO}" width="12" height="12" alt="Logo"></a> end</div>`);
    await selectNode(page, 'a img', true);
    await page.keyboard.press('ControlOrMeta+k');
    await expect(pop(page)).toHaveAttribute('aria-label', L.edit);
    await expect(field(page, 'text')).toHaveValue('');
    await expect(field(page, 'url')).toHaveValue('https://old.example/');
    await field(page, 'url').fill('https://new.example/');
    await page.keyboard.press('Enter');
    await expect(pop(page)).toBeHidden();
    await expect.poll(() => links(page)).toEqual([['https://new.example/', '']]);
    await expect(editor(page).locator('a img')).toHaveCount(1);
    expect(await editor(page).textContent()).not.toContain('new.example');

    // An image that is no link yet, selected: it becomes the link.
    await fill(page, `<div>Photo: <img src="${LOGO}" width="12" height="12" alt="Photo"> end</div>`);
    await selectNode(page, 'img', true);
    await page.keyboard.press('ControlOrMeta+k');
    await expect(pop(page)).toHaveAttribute('aria-label', L.insert);
    await field(page, 'url').fill('www.example.com');
    await page.keyboard.press('Enter');
    await expect.poll(() => links(page)).toEqual([['https://www.example.com', '']]);
    await expect(editor(page).locator('a img')).toHaveCount(1);

    // A link from pasted HTML with line breaks in its source: the untouched Text field keeps its bold and color.
    await fill(page, '<div><a href="https://old.example/">\n  <b>Company</b> <span style="color: rgb(200, 0, 0)">Name</span>\n</a> now</div>');
    await select(page, 'Company', 3);
    await page.keyboard.press('ControlOrMeta+k');
    await expect(pop(page)).toHaveAttribute('aria-label', L.edit);
    await field(page, 'url').fill('https://new.example/');
    await page.keyboard.press('Enter');
    await expect.poll(async () => [...new Set((await links(page)).map(([href]) => href))]).toEqual(['https://new.example/']);
    expect((await links(page)).map(([, text]) => text).join('').replace(/\s+/g, ' ').trim()).toBe('Company Name');
    await expect(editor(page).locator('a b')).toHaveText('Company');
    expect(await editor(page).locator('a span').evaluate((el) => getComputedStyle(el).color)).toBe('rgb(200, 0, 0)');
  });

  test(`Go to link says so when Rukoo doesn't open the address, ${surface.name}`, async () => {
    const page = await surface.open();
    await fill(page, '<div>Run <a href="javascript:alert(1)">this</a> or open <a href=" https://rukoo.io/docs ">that</a> now</div>');
    await editor(page).locator('a', { hasText: 'this' }).click();
    await expect(bubble(page)).toBeVisible();
    await bubble(page).locator('[data-a="go"]').click();
    await expect(page.locator('#toast')).toHaveText(L.cantOpen);
    await expect(page.locator('#toast')).toHaveClass(/\bshow\b/);
    expect(await opened()).toEqual([]);
    expect(await editorFocused(page)).toBe(true);
    // Spaces around an address are no reason to refuse it, and the bubble shows the address without them.
    await editor(page).locator('a', { hasText: 'that' }).click();
    await expect(bubble(page)).toBeVisible();
    await expect(bubble(page).locator('.link-go-url')).toHaveText('https://rukoo.io/docs');
    await expect(bubble(page).locator('[data-a="go"]')).toHaveAttribute('aria-label', `${L.open}https://rukoo.io/docs`);
    await bubble(page).locator('[data-a="go"]').click();
    await expect.poll(opened).toEqual(['https://rukoo.io/docs']);
  });
}

async function composeInPane() {
  await launch('en');
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
}

test('pasted text keeps the spaces at its ends', async () => {
  await composeInPane();
  await write(win, 'word');
  await paste(win, { text: ' see https://a.example ' });
  await expect.poll(() => links(win)).toEqual([['https://a.example', 'https://a.example']]);
  await win.keyboard.type('next');
  expect(await lines(win)).toEqual(['word see https://a.example next']);
  // Several lines: each keeps the space at its end.
  await empty(win);
  await paste(win, { text: 'a https://x.example \nb www.y.example ' });
  await expect.poll(() => links(win)).toEqual([
    ['https://x.example', 'https://x.example'],
    ['https://www.y.example', 'www.y.example']
  ]);
  await win.keyboard.type('next');
  expect(await lines(win)).toEqual(['a https://x.example', 'b www.y.example next']);
});

test('an address pasted over a selection that holds code stays text', async () => {
  await composeInPane();
  // Code is never linked, also inside a selection that starts and ends outside it: the address replaces the selection
  // as text, as it does over a selection that starts in code.
  const overCode = async (html, from, to) => {
    await fill(win, html);
    await editor(win).evaluate(
      (root, [from, to]) => {
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
        const texts = [];
        for (let node = walker.nextNode(); node; node = walker.nextNode()) texts.push(node);
        const start = texts.find((n) => n.data.includes(from));
        const end = texts.find((n) => n.data.includes(to));
        const range = document.createRange();
        range.setStart(start, start.data.indexOf(from));
        range.setEnd(end, end.data.indexOf(to) + to.length);
        getSelection().removeAllRanges();
        getSelection().addRange(range);
      },
      [from, to]
    );
    await win.waitForTimeout(30);
    await paste(win, { text: 'https://rukoo.io' });
    await expect.poll(() => editor(win).textContent()).toContain('https://rukoo.io');
    expect(await links(win)).toEqual([]);
  };
  await overCode('<div>Run <code>npm start</code> first</div>', 'Run', 'first');
  await overCode('<div>Before</div><pre>npm test</pre><div>then ship</div>', 'Before', 'then');
});

test('a draft keeps its links and their text when it is saved, closed and opened again', async () => {
  await launch('en');
  await win.click('[data-action="compose"]');
  await win.waitForSelector('.composer .editor');
  await win.locator('.composer [data-field="subject"]').fill('Links in a draft');
  await write(win, 'Typed www.example.com and ');
  await paste(win, { text: 'pasted https://rukoo.io/docs.' });
  await win.keyboard.type(' Also ');
  await win.keyboard.press('ControlOrMeta+k');
  await field(win, 'text').fill('our guide');
  await field(win, 'url').fill('rukoo.io/guide');
  await win.keyboard.press('Enter');
  const before = [
    ['https://www.example.com', 'www.example.com'],
    ['https://rukoo.io/docs', 'https://rukoo.io/docs'],
    ['https://rukoo.io/guide', 'our guide']
  ];
  await expect.poll(() => links(win)).toEqual(before);
  await win.keyboard.press('ControlOrMeta+s');
  await expect(win.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await win.locator('.composer [data-c="close"]').click();
  await expect(win.locator('.composer')).toHaveCount(0);

  await win.click('[data-view="drafts"]');
  await win.locator('.item', { hasText: 'Links in a draft' }).first().click();
  await win.locator('[data-reader="edit"]').click();
  await win.waitForSelector('.composer .editor');
  await expect.poll(() => links(win)).toEqual(before);
  expect(await lines(win)).toEqual(['Typed www.example.com and pasted https://rukoo.io/docs. Also our guide']);
  // The reopened links work like new ones.
  await editor(win).locator('a', { hasText: 'our guide' }).click();
  await expect(bubble(win).locator('.link-go-url')).toHaveText('https://rukoo.io/guide');
  // Neither the popover nor the bubble went into the saved text.
  const stored = await win.evaluate(() => document.querySelector('.editor').innerHTML);
  expect(stored).not.toMatch(/link-pop|link-bubble/);
});
