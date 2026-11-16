// HTML signatures: written and pasted in Settings, and carried into the inline editor, the compose window,
// drafts and sent mail. The real app with a throwaway profile and the demo account, in English.
const { test, expect, _electron: electron } = require('@playwright/test');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');

let app;
let win;
let dataDir;

// A 1x1 PNG.
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
const SIGNATURE =
  `<table cellpadding="0"><tr><td style="padding-right:10px"><img src="${PNG}" alt="Logo" width="32" height="32"></td>` +
  '<td><b style="color:#1c2340">Demo Sender</b><br><span style="color:#6b7280">Customer success</span><br>' +
  '<a href="https://example.com/team">example.com/team</a></td></tr></table>';

test.beforeEach(async () => {
  dataDir = tempDir('rukoo-sig-e2e-');
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
const settings = () => win.evaluate(() => window.mail.call('state').then((s) => s.settings));
const account = () => win.evaluate(() => window.mail.call('state').then((s) => s.accounts[0]));
// Stored through main, then the window is reloaded so the editor starts from the stored settings.
async function setSettings(patch) {
  await win.evaluate((p) => window.mail.call('updateSettings', p), patch);
  await win.reload();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
}
const setSignature = (html) => setSettings({ signatureHtml: html });

// Pastes HTML into the focused editor the way the clipboard does.
const paste = (selector, html) =>
  win.locator(selector).evaluate((el, value) => {
    const data = new DataTransfer();
    data.setData('text/html', value);
    data.setData('text/plain', 'plain');
    el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
  }, html);

async function expectSignature(editor) {
  const sig = editor.locator('.signature');
  await expect(sig.locator('b')).toHaveText('Demo Sender');
  await expect(sig.locator('b')).toHaveCSS('color', 'rgb(28, 35, 64)');
  await expect(sig.locator('a[href="https://example.com/team"]')).toHaveText('example.com/team');
  await expect(sig.locator('img[alt="Logo"]')).toHaveAttribute('src', /^data:image\/png;base64,/);
  expect(await sig.locator('img').evaluate((img) => img.complete && img.naturalWidth)).toBe(1);
}

test('a pasted signature is cleaned, previewed and saved; its HTML can be edited', async () => {
  await win.click('[data-action="settings"]');
  await win.click('[data-a="signatures"]');
  await expect(win.locator('[data-a="signature"] .value')).toHaveText('None');
  await win.click('[data-a="signature"]');
  const editor = win.locator('.sig-editor');
  await expect(editor).toBeFocused();
  await paste('.sig-editor', `<meta charset="utf-8"><style>p{display:none}</style><p class="MsoNormal" onclick="alert(1)"><b>Pasted</b> <a href="javascript:alert(1)">bad</a> <a href="https://example.com/a">good</a></p><script>window.__pwned = 1</script><img src="https://example.com/x.png" onerror="window.__pwned = 1"><iframe src="https://example.com"></iframe>`);
  await expect(editor.locator('b')).toHaveText('Pasted');
  await expect(editor.locator('a')).toHaveCount(1);
  await expect(editor.locator('a')).toHaveAttribute('href', 'https://example.com/a');
  await expect(editor.locator('script, iframe, style, [onclick], [onerror], .MsoNormal')).toHaveCount(0);
  await expect(editor).toContainText('bad');

  // The HTML source: what is typed there is cleaned when switching back.
  await win.click('.sig-toolbar [data-c="source"]');
  await expect(win.locator('.sig-source')).toBeVisible();
  await expect(win.locator('.sig-source')).toHaveValue(/<b>Pasted<\/b>/);
  await expect(win.locator('.sig-toolbar [data-cmd="bold"]')).toBeDisabled();
  await win.locator('.sig-source').fill(`${SIGNATURE}<script>window.__pwned = 1</script><a href="javascript:x()">js</a>`);
  await win.click('.sig-toolbar [data-c="source"]');
  await expect(win.locator('.sig-source')).toBeHidden();
  await expect(editor.locator('script')).toHaveCount(0);
  await expect(editor.locator('a')).toHaveCount(1);
  await expect(editor.locator('table img[alt="Logo"]')).toBeVisible();

  await win.click('.scrim .buttons button:text-is("Save")');
  await expect(win.locator('.scrim')).toHaveCount(0);
  const preview = win.locator('[data-a="signature"] .sig-preview');
  await expect(preview.locator('b')).toHaveText('Demo Sender');
  await expect(preview.locator('img')).toHaveCount(1);
  await expect(preview).toHaveAttribute('inert', '');
  const s = await settings();
  expect(s.signatureHtml).toContain('<b style="color:#1c2340">Demo Sender</b>');
  expect(s.signatureHtml).not.toMatch(/script|javascript/);
  // The text for the plain-text part and earlier versions; the unsafe link kept its text.
  expect(s.signature).toMatch(/^Demo Sender\nCustomer success\nexample\.com\/team\s+js$/);
  expect(await win.evaluate(() => window.__pwned)).toBeUndefined();

  // Formatting from the toolbar.
  await win.click('[data-a="signature"]');
  await win.locator('.sig-editor b').selectText();
  await win.click('.sig-toolbar [data-cmd="italic"]');
  await win.click('.scrim .buttons button:text-is("Save")');
  await expect(win.locator('[data-a="signature"] .sig-preview i, [data-a="signature"] .sig-preview [style*="italic"]')).toHaveCount(1);
});

test('an account signature can be formatted, and set back to the general one', async () => {
  await setSignature('<i>General one</i>');
  await win.click('[data-action="settings"]');
  await win.click('.row:has-text("demo@example.com")');
  const row = win.locator('[data-a^="acc-signature:"]');
  await expect(row.locator('.value')).toHaveText('Uses the general signature');
  await expect(row.locator('.sig-preview i')).toHaveText('General one');
  // With sending: right after Send as and Sender addresses, with the booking link under it.
  const actions = await win.locator('.page.settings .card').first().locator('.row').evaluateAll((rows) => rows.map((r) => r.dataset.a || ''));
  const at = (prefix) => actions.findIndex((a) => a.startsWith(prefix));
  expect(at('acc-aliases')).toBe(at('acc-from') + 1);
  expect(at('acc-signature:')).toBe(at('acc-aliases') + 1);
  expect(at('booking-acc-footer:')).toBe(at('acc-signature:') + 1);
  await row.click();
  await expect(win.locator('.sig-editor i')).toHaveText('General one');
  await win.click('.sig-toolbar [data-c="source"]');
  await win.locator('.sig-source').fill('<p><u>Only for demo</u> <a href="mailto:team@example.com">mail us</a></p>');
  await win.click('.scrim .buttons button:text-is("Save")');
  await expect(row.locator('.sig-preview u')).toHaveText('Only for demo');
  await expect(row.locator('.value')).toHaveCount(0);
  expect((await account()).signatureHtml).toBe('<p><u>Only for demo</u> <a href="mailto:team@example.com">mail us</a></p>');

  await row.click();
  await win.click('.scrim .buttons button:text-is("Use general signature")');
  await expect(row.locator('.value')).toHaveText('Uses the general signature');
  const acc = await account();
  expect(acc.signature).toBeNull();
  expect(acc.signatureHtml).toBeNull();
});

test('a plain-text signature still shows as before', async () => {
  await setSettings({ signature: 'Regards,\n<Demo>' });
  await win.click('[data-action="compose"]');
  const sig = win.locator('.composer .editor .signature');
  await expect(sig).toHaveText('Regards,<Demo>');
  expect(await sig.innerHTML()).toBe('Regards,<br>&lt;Demo&gt;');
});

test('the signature is formatted in new mail, replies, forwards and the compose window', async () => {
  await setSignature(SIGNATURE);
  const editor = win.locator('.composer .editor');
  await win.click('[data-action="compose"]');
  await expectSignature(editor);
  await win.keyboard.press('Escape');
  await expect(win.locator('.composer')).toHaveCount(0);

  await item('Sanne de Vries').click();
  await win.click('[data-reader="reply"]');
  await expectSignature(editor);
  await win.keyboard.press('Escape');
  await win.click('[data-reader="forward"]');
  await expectSignature(editor);

  const [popped] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  await popped.waitForSelector('.compose .editor');
  await expectSignature(popped.locator('.compose .editor'));
});

// Two more accounts beside the demo one, on a server these tests never reach: Werk with a signature of its own,
// Privé with its own empty one. The window is reloaded so the editor starts from main's list with them in it.
async function addAccounts() {
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const add = (id, email, name, signature, signatureHtml) => {
      const server = { host: '127.0.0.1', port: 1, secure: false, user: email };
      engine.accounts.push({ id, type: 'imap', provider: 'other', email, name, label: email, color: '#ff8a3d', signature, signatureHtml, secret: 'plain:', imap: server, smtp: server, createdAt: Date.now() });
      engine.caches.set(id, { folders: [], boxes: {}, lastSync: Date.now() });
    };
    add('w1', 'werk@example.com', 'Werk', 'Werk signature', '<b>Werk signature</b>');
    add('p1', 'prive@example.com', 'Privé', '', null);
  });
  await win.reload();
  await expect(win.locator('.item').first()).toBeVisible({ timeout: 15000 });
}

test('another account in From brings its own signature, in the inline editor and the compose window', async () => {
  await setSignature('<i>General one</i>');
  await addAccounts();
  const demoId = await app.evaluate(() => global.__semEngine.accounts.find((a) => a.type === 'demo').id);

  async function check(page, editor) {
    const from = page.locator('[data-field="account"]');
    const sig = editor.locator(':scope > .signature');
    await expect(sig).toHaveText('General one');
    await editor.click({ position: { x: 4, y: 4 } });
    await page.keyboard.type('Hello there.');

    await from.selectOption('w1|werk@example.com');
    await expect(sig).toHaveCount(1);
    await expect(sig.locator('b')).toHaveText('Werk signature');
    await expect(sig.locator('i')).toHaveCount(0);
    // An account with an empty signature of its own: no signature, and no empty line left where it was.
    await from.selectOption('p1|prive@example.com');
    await expect(sig).toHaveCount(0);
    expect(await editor.innerHTML()).toBe('<div>Hello there.</div>');
    // Back to an account with one: it comes back at the end, after an empty line, as in a new message.
    await from.selectOption(`${demoId}|demo@example.com`);
    await expect(sig).toHaveText('General one');
    expect(await editor.innerHTML()).toBe('<div>Hello there.</div><div><br></div><div class="signature"><i>General one</i></div>');
    await expect(page.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  }

  await win.click('[data-action="compose"]');
  await check(win, win.locator('.composer .editor'));
  await win.click('[data-c="discard"]');
  await win.click('.scrim .buttons button:text-is("Delete")');
  await expect(win.locator('.composer')).toHaveCount(0);

  const [popup] = await Promise.all([app.waitForEvent('window'), win.evaluate(() => window.mail.call('openCompose', { mode: 'new' }))]);
  await popup.waitForSelector('.composer .editor');
  await check(popup, popup.locator('.composer .editor'));
});

test('a reopened draft has its signature on its own line again, and swaps it with From too', async () => {
  await setSignature('<i>General one</i>');
  await addAccounts();
  await win.click('[data-action="compose"]');
  const editor = win.locator('.composer .editor');
  await win.fill('[data-field="subject"]', 'Reopened draft');
  await editor.click({ position: { x: 4, y: 4 } });
  await win.keyboard.type('Hello there.');
  await win.keyboard.press('Control+s');
  await expect(win.locator('.composer .save-state')).toHaveText('Draft saved');
  await win.keyboard.press('Escape');
  await expect(win.locator('.composer')).toHaveCount(0);

  await win.click('[data-view="drafts"]');
  await item('Reopened draft').click();
  await win.click('[data-reader="edit"]');
  // The text and the signature are the editor's own lines again, not inside the font wrapper of the saved mail.
  expect(await editor.innerHTML()).toBe('<div>Hello there.</div><div><br></div><div class="signature"><i>General one</i></div>');
  const sig = editor.locator('.signature');
  await win.locator('.composer [data-field="account"]').selectOption('w1|werk@example.com');
  await expect(sig).toHaveCount(1);
  await expect(sig.locator('b')).toHaveText('Werk signature');
  await win.locator('.composer [data-field="account"]').selectOption('p1|prive@example.com');
  await expect(sig).toHaveCount(0);
  expect(await editor.innerHTML()).toBe('<div>Hello there.</div>');
});

test('a reopened draft keeps a formatted block of its own that uses the same font as the saved mail', async () => {
  await win.click('[data-action="compose"]');
  const editor = win.locator('.composer .editor');
  await win.fill('[data-field="subject"]', 'Styled draft');
  const styled = `<div style="font-family:'Segoe UI',Arial,sans-serif;font-size:12pt;color:rgb(200, 0, 0);text-align:center"><b>Centered and red</b></div>`;
  await editor.evaluate((el, html) => (el.innerHTML = html), styled);
  await win.keyboard.press('Control+s');
  await expect(win.locator('.composer .save-state')).toHaveText('Draft saved');
  await win.keyboard.press('Escape');
  await expect(win.locator('.composer')).toHaveCount(0);

  await win.click('[data-view="drafts"]');
  await item('Styled draft').click();
  await win.click('[data-reader="edit"]');
  const block = editor.locator(':scope > div');
  await expect(block).toHaveCount(1);
  await expect(block).toHaveCSS('color', 'rgb(200, 0, 0)');
  await expect(block).toHaveCSS('text-align', 'center');
  await expect(block.locator('b')).toHaveText('Centered and red');
});

test('a draft keeps the signature, and sent mail carries it with its image and text', async () => {
  await setSignature(SIGNATURE);
  await win.click('[data-action="compose"]');
  const editor = win.locator('.composer .editor');
  await win.fill('[data-rinput="to"]', 'demo@example.com');
  await win.keyboard.press('Enter');
  await win.fill('[data-field="subject"]', 'Signed mail');
  await editor.click({ position: { x: 4, y: 4 } });
  await win.keyboard.type('Hello there.');
  await win.keyboard.press('Control+s');
  await expect(win.locator('.composer .save-state')).toHaveText('Draft saved');
  await win.keyboard.press('Escape');
  await expect(win.locator('.composer')).toHaveCount(0);

  await win.click('[data-view="drafts"]');
  await item('Signed mail').click();
  await win.click('[data-reader="edit"]');
  await expectSignature(editor);
  await expect(editor).toContainText('Hello there.');
  // The image went into the draft as an inline part, not as an attachment.
  await expect(win.locator('.composer .compose-attachments .att-chip')).toHaveCount(0);

  // Keeps what the account sends, as it goes on the wire. (The demo mailbox stores mail parsed, with images as data.)
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const session = engine.session(engine.accounts[0]);
    const send = session.send.bind(session);
    session.send = async (mail) => {
      const raw = await send(mail);
      global.__sentRaw = raw.toString('utf8');
      return raw;
    };
  });
  await win.click('[data-c="send"]');
  await expect(win.locator('.composer')).toHaveCount(0, { timeout: 10000 });
  await win.click('[data-view="inbox"]');
  await expect(item('Signed mail')).toBeVisible({ timeout: 10000 });
  await item('Signed mail').click();
  const frame = win.frameLocator('.mail-frame');
  await expect(frame.locator('body')).toContainText('Hello there.');
  await expect(frame.locator('b')).toHaveText('Demo Sender');
  await expect(frame.locator('a[href="https://example.com/team"]')).toBeVisible();
  expect(await frame.locator('img[alt="Logo"]').evaluate((img) => img.naturalWidth)).toBe(1);

  // On the wire: the image is a Content-ID part, the plain-text part has the signature as text.
  const raw = await app.evaluate(() => global.__sentRaw);
  expect(raw).toMatch(/Content-Type: multipart\/related/);
  expect(raw).toMatch(/Content-ID: <[0-9a-f]+@rukoo>/);
  expect(raw).toMatch(/Content-Disposition: inline/);
  expect(raw).toMatch(/src=3D"cid:[0-9a-f]+@rukoo"/);
  expect(raw).not.toContain('data:image');
  expect(raw).toMatch(/Demo Sender\r?\nCustomer success\r?\nexample\.com\/team/);
});

test('Settings > Signatures shows what every account sends and edits it there', async () => {
  await setSignature('<i>General one</i>');
  await addAccounts();
  const demoId = await app.evaluate(() => global.__semEngine.accounts.find((a) => a.type === 'demo').id);
  await app.evaluate(() => {
    global.__semEngine.accounts.find((a) => a.id === 'w1').aliases = [
      { address: 'sales@example.com', name: 'Werk' },
      { address: 'info@example.com', name: 'Werk' }
    ];
  });
  const accountOf = (id) => win.evaluate((x) => window.mail.call('state').then((s) => s.accounts.find((a) => a.id === x)), id);
  const page = win.locator('.page.settings');
  const save = () => win.click('.scrim .buttons button:text-is("Save")');

  // One row under the accounts, reached by keyboard too; General no longer has the signature.
  await win.click('[data-action="settings"]');
  const open = page.locator('[data-a="signatures"]');
  await expect(open.locator('.desc')).toHaveText('General and per account');
  await expect(page.locator('[data-a="signature"], [data-a="booking-footer"]')).toHaveCount(0);
  await open.focus();
  await win.keyboard.press('Enter');
  await expect(page.locator('h1')).toHaveText('Signatures');

  const general = page.locator('[data-a="signature"]');
  await expect(general.locator('.title')).toHaveText('General signature');
  await expect(general.locator('.desc')).toHaveText('For accounts without a signature of their own');
  await expect(general.locator('.sig-preview i')).toHaveText('General one');
  await expect(page.locator('[data-a="booking-footer"]')).toBeVisible();

  // Each account: its own signature, the general one, or none, with the aliases that send it too.
  const acc = (id) => page.locator(`[data-a="acc-signature:${encodeURIComponent(id)}"]`);
  await expect(page.locator('.sig-account .acc-email')).toHaveText(['demo@example.com', 'werk@example.com', 'prive@example.com']);
  await expect(acc(demoId).locator('.default-badge')).toHaveText('Default');
  await expect(acc(demoId).locator('.value')).toHaveText('Uses the general signature');
  await expect(acc(demoId).locator('.sig-preview i')).toHaveText('General one');
  await expect(acc(demoId).locator('.desc')).toHaveCount(0);
  await expect(acc('w1').locator('.value')).toHaveText('Its own signature');
  await expect(acc('w1').locator('.sig-preview b')).toHaveText('Werk signature');
  await expect(acc('w1').locator('.desc')).toHaveText('Also under mail from sales@example.com and info@example.com');
  await expect(acc('p1').locator('.value')).toHaveText('None');
  await expect(acc('p1').locator('.sig-preview')).toHaveCount(0);

  // Edited from the list with the same editor, which says whose signature it is; the page stays.
  await acc('w1').click();
  await expect(win.locator('.scrim h2')).toHaveText('Signature for werk@example.com');
  await expect(win.locator('.sig-editor b')).toHaveText('Werk signature');
  await win.click('.sig-toolbar [data-c="source"]');
  await win.locator('.sig-source').fill('<b>Werk</b> <a href="https://example.com/werk">example.com/werk</a>');
  await save();
  await expect(acc('w1').locator('.sig-preview a')).toHaveText('example.com/werk');
  await expect(page.locator('h1')).toHaveText('Signatures');
  expect((await accountOf('w1')).signatureHtml).toBe('<b>Werk</b> <a href="https://example.com/werk">example.com/werk</a>');

  // Its size limit too: a signature that is too large is refused and the account keeps what it had.
  await acc('w1').click();
  await win.click('.sig-toolbar [data-c="source"]');
  await win.locator('.sig-source').fill(`<img src="data:image/png;base64,${'A'.repeat(1100 * 1024)}">`);
  await save();
  await expect(win.locator('.toast')).toHaveText('This signature is too large. Use smaller images, or images from the web.');
  await expect(acc('w1').locator('.sig-preview a')).toHaveText('example.com/werk');
  expect((await accountOf('w1')).signatureHtml).toContain('example.com/werk');

  // Back to the general signature from the list.
  await acc('w1').click();
  await win.click('.scrim .buttons button:text-is("Use general signature")');
  await expect(acc('w1').locator('.value')).toHaveText('Uses the general signature');
  await expect(acc('w1').locator('.sig-preview i')).toHaveText('General one');
  expect(await accountOf('w1')).toMatchObject({ signature: null, signatureHtml: null });

  // A new general signature shows under every account that uses it.
  await general.click();
  await expect(win.locator('.scrim h2')).toHaveText('General signature');
  await win.click('.sig-toolbar [data-c="source"]');
  await win.locator('.sig-source').fill('<i>New general</i>');
  await save();
  await expect(general.locator('.sig-preview i')).toHaveText('New general');
  await expect(acc(demoId).locator('.sig-preview i')).toHaveText('New general');
  await expect(acc('w1').locator('.sig-preview i')).toHaveText('New general');
  await expect(acc('p1').locator('.value')).toHaveText('None');

  // Keyboard: Tab goes from row to row, Enter opens the editor, Escape closes it and then the page.
  await general.focus();
  await win.keyboard.press('Tab');
  await expect(page.locator('[data-a="booking-footer"]')).toBeFocused();
  await win.keyboard.press('Tab');
  await expect(acc(demoId)).toBeFocused();
  await win.keyboard.press('Enter');
  await expect(win.locator('.sig-editor')).toBeFocused();
  await win.keyboard.press('Escape');
  await expect(win.locator('.scrim')).toHaveCount(0);
  await win.keyboard.press('Escape');
  await expect(page.locator('h1')).toHaveText('Email settings');

  // The sender addresses page says which signature all of an account's addresses send, and edits it too.
  await page.locator('.row:has-text("werk@example.com")').first().click();
  await page.locator('[data-a="acc-aliases"]').click();
  await expect(page.locator('h1')).toHaveText('Sender addresses');
  const sent = acc('w1');
  await expect(page.locator('.settings-group-title').last()).toHaveText('Signature');
  await expect(sent.locator('.title')).toHaveText('Under mail from all 3 addresses');
  await expect(sent.locator('.value')).toHaveText('Uses the general signature');
  await expect(sent.locator('.sig-preview i')).toHaveText('New general');
  await sent.click();
  await win.click('.sig-toolbar [data-c="source"]');
  await win.locator('.sig-source').fill('<b>Werk again</b>');
  await save();
  await expect(sent.locator('.sig-preview b')).toHaveText('Werk again');
  await expect(sent.locator('.value')).toHaveCount(0);
  await expect(page.locator('h1')).toHaveText('Sender addresses');
  expect((await accountOf('w1')).signatureHtml).toBe('<b>Werk again</b>');
});

test('the signature editor is in Dutch with the rest of the app', async () => {
  await setSettings({ language: 'nl' });
  await win.click('[data-action="settings"]');
  await win.click('[data-a="signatures"]');
  const page = win.locator('.page.settings');
  await expect(page.locator('h1')).toHaveText('Handtekeningen');
  await expect(page.locator('[data-a="signature"] .title')).toHaveText('Algemene handtekening');
  await expect(page.locator('[data-a="signature"] .desc')).toHaveText('Voor accounts zonder eigen handtekening');
  await expect(page.locator('[data-a="signature"] .value')).toHaveText('Geen');
  await expect(page.locator('[data-a^="acc-signature:"] .value')).toHaveText('Algemeen: geen');
  await page.locator('[data-a="back"]').click();
  await expect(page.locator('[data-a="signatures"] .desc')).toHaveText('Algemeen en per account');
  await win.click('.row:has-text("demo@example.com")');
  await expect(win.locator('[data-a^="acc-signature:"] .value')).toHaveText('Algemeen: geen');
  await win.click('[data-a^="acc-signature:"]');
  await expect(win.locator('.scrim h2')).toHaveText('Handtekening voor demo@example.com');
  await expect(win.locator('.sig-help')).toHaveText('Plak een handtekening uit een andere mailapp of van een webpagina, of schrijf en maak hem hier op.');
  await expect(win.locator('.sig-editor')).toHaveAttribute('data-placeholder', 'Je naam, functie en contactgegevens');
  await expect(win.locator('.sig-toolbar [data-c="source"]')).toHaveAttribute('title', 'HTML bewerken');
  await expect(win.locator('.scrim .buttons button')).toHaveText(['Algemene gebruiken', 'Annuleren', 'Opslaan']);

  // An account without aliases: its one address sends the signature.
  await win.keyboard.press('Escape');
  await expect(win.locator('.scrim')).toHaveCount(0);
  await page.locator('[data-a="acc-aliases"]').click();
  await expect(page.locator('h1')).toHaveText('Afzenderadressen');
  await expect(page.locator('.settings-group-title').last()).toHaveText('Handtekening');
  await expect(page.locator('[data-a^="acc-signature:"] .title')).toHaveText('Onder mail van het accountadres');
  await expect(page.locator('[data-a^="acc-signature:"] .value')).toHaveText('Algemeen: geen');
});
