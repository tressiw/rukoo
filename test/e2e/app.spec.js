// End-to-end tests: launch the real Electron app with a throwaway profile and the demo account.
const { test, expect, _electron: electron } = require('@playwright/test');
const fs = require('fs');
const http = require('http');
const path = require('path');
const { tempDir, removeDir } = require('../tmp');
const { closeApp } = require('./close');
const { titlebar, edges } = require('./titlebar');

let app;
let win;
let dataDir;

test.beforeEach(async () => {
  dataDir = tempDir('sem-e2e-');
  // Keep the existing Dutch interaction suite as coverage for the supported Dutch locale.
  fs.mkdirSync(path.join(dataDir, 'data'));
  fs.writeFileSync(path.join(dataDir, 'data', 'settings.json'), JSON.stringify({ language: 'nl' }));
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
  // Each profile is a few MB; a full run would otherwise leave them all in the temp folder.
  removeDir(dataDir);
});

const item = (text) => win.locator('.item', { hasText: text }).first();

// The editor opens in the reading pane.
async function composer(trigger) {
  await trigger();
  await win.waitForSelector('.composer .editor');
  return win;
}
const composerGone = () => expect(win.locator('.composer')).toHaveCount(0, { timeout: 10000 });

// Makes engine calls slow or failing in the main process, to reach the race windows.
const slowDown = (method, ms) =>
  app.evaluate(
    (_, [method, ms]) => {
      const engine = global.__semEngine;
      const orig = engine[method].bind(engine);
      global.__calls = global.__calls || {};
      global.__calls[method] = 0;
      engine[method] = async (...args) => {
        global.__calls[method]++;
        await new Promise((r) => setTimeout(r, ms));
        return orig(...args);
      };
    },
    [method, ms]
  );
const calls = (method) => app.evaluate((_, m) => (global.__calls || {})[m] || 0, method);

// ---------- window ----------

// With the window buttons on the right, as on Windows and by default on Linux, the title bar keeps the layout it has
// always had: the icon 22px from the left edge and 150px kept free for the buttons. linux.spec.js covers both Linux
// layouts in both windows.
test('the title bar keeps its fixed room for window buttons on the right', async () => {
  test.skip(process.platform === 'darwin', 'macOS has traffic lights, without a window controls overlay');
  // The buttons take part of the bar once the overlay knows its geometry.
  await expect.poll(() => titlebar(win).then((g) => g.width - (g.area.right - g.area.left))).toBeGreaterThan(40);
  const g = await titlebar(win);
  test.skip(g.area.left > 0, 'this Linux desktop puts the window buttons on the left');
  expect(await edges(win)).toEqual({ controls: null, icon: 22, actions: 150 });
  // The buttons fit in those 150px.
  expect(g.actions.right).toBeLessThan(g.area.right);
});

// ---------- reading ----------

test('first run shows the demo inbox with the newest message open, still unread', async () => {
  await expect(win.locator('.list-title h1')).toHaveText('Postvak IN');
  await expect(win.locator('.list-title .sub')).toContainText('demo@example.com');
  await expect(win.locator('.reader-subject')).toHaveText('Traffic fines in 2027: what you will pay');
  await expect(item('ANWB Newsletter')).toHaveClass(/unread/);
  await expect(win.locator('.nav-item.active .count')).toHaveText('7');
  await expect(win.locator('.group-head').first()).toContainText('Vandaag');
  await expect(win.locator('.sync-status')).toContainText('bijgewerkt');
});

test('opening a message renders it and marks it read', async () => {
  await item('ANWB Newsletter').click();
  await expect(win.frameLocator('.mail-frame').locator('h1')).toContainText('low sun');
  await expect(item('ANWB Newsletter')).not.toHaveClass(/unread/);
  await expect(item('ANWB Newsletter')).toHaveClass(/selected/);
  await expect(win.locator('.nav-item.active .count')).toHaveText('6');
  await expect(win.locator('.msg-from')).toContainText('newsletter@anwb.nl');
  await expect(win.locator('.to-summary')).toHaveText('aan mij');
  await win.click('[data-reader="next"]');
  await expect(win.locator('.reader-subject')).toContainText('music industry');
});

// #99: the list holds the newest 300 inbox messages, and the sidebar's Inbox count adds the unread mail older than those.
test('the list header names the unread mail older than the list, which Mark all as read does not reach', async () => {
  await app.evaluate(async () => {
    const engine = global.__semEngine;
    const acc = engine.accounts[0];
    while (engine.syncs.size) await new Promise((r) => setTimeout(r, 50));
    // A server that counts 434 unread inbox messages below the window.
    const session = engine.session(acc);
    const sync = session.syncFolder.bind(session);
    session.syncFolder = async (p, opts) => {
      const res = await sync(p, opts);
      return opts.olderUnseen ? { ...res, olderUnseen: 434 } : res;
    };
    await engine.syncAccount(acc.id);
  });
  const sub = win.locator('.list-title .sub');
  const inboxCount = win.locator('.nav-item[data-view="inbox"] .count');
  const unreadChip = win.locator('.filter[data-filter="unread"] .n');
  await expect(inboxCount).toHaveText('441');
  await expect(sub).toHaveText('7 ongelezen, nog 434 in oudere mail · demo@example.com');
  await expect(sub).toHaveAttribute('title', /nieuwste 300 berichten\. 434 ongelezen berichten zijn ouder\./);
  await expect(unreadChip).toHaveText('7');

  // Mark all as read reaches the list only; the header then names just the older mail.
  await win.click('[data-action="list-more"]');
  await win.click('.menu [role="menuitem"]:has-text("Alles als gelezen markeren")');
  await expect(inboxCount).toHaveText('434');
  await expect(sub).toHaveText('434 ongelezen in oudere mail · demo@example.com');
  await expect(unreadChip).toHaveCount(0);

  // Other views count their own messages and say nothing about the older inbox mail.
  await win.click('.nav-item[data-view="starred"]');
  await expect(sub).toHaveText('demo@example.com');
  await expect(sub).not.toHaveAttribute('title', /.+/);
});

test('previews skip "view in browser" lines and greetings', async () => {
  await expect(item('ANWB Newsletter').locator('.preview')).not.toContainText('View email in browser');
  await expect(item('Vandebron').locator('.preview')).toContainText('Customer number');
  await expect(item('Vandebron').locator('.preview')).not.toContainText('Dear');
});

test('the header summarises recipients and shows the details on request', async () => {
  await item('Sanne de Vries').click();
  await expect(win.locator('.to-summary')).toHaveText('aan mij, Joris Bakker');
  await expect(win.locator('.details')).toHaveCount(0);
  await win.click('.to-summary');
  await expect(win.locator('.details')).toContainText('joris@example.com');
  await expect(win.locator('[data-reader="unsubscribe"]')).toHaveCount(0);
});

test('newsletters can be unsubscribed from in one click', async () => {
  await expect(win.locator('.reader-subject')).toHaveText('Traffic fines in 2027: what you will pay');
  await win.click('[data-reader="unsubscribe"]');
  await expect(win.locator('.scrim')).toContainText('Uitschrijven bij ANWB Newsletter?');
  await win.click('.scrim .buttons button:text-is("Uitschrijven")');
  await expect(win.locator('#toast')).toContainText('Uitgeschreven bij ANWB Newsletter');
});

test('every email has its own row until stacks are turned on, and then unread mail still does', async () => {
  // By default, the four emails in a row from Vandebron No Reply each have a row.
  for (const subject of ['Your password has changed', 'Your sign-in details have been updated', 'How to change your My Vandebron password', 'Failed sign-in attempt'])
    await expect(item(subject)).toBeVisible();
  await expect(win.locator('.stack-btn')).toHaveCount(0);

  await win.click('[data-action="settings"]');
  await expect(win.locator('[data-a="toggle:stackSenders"] .title')).toHaveText('Mail van één afzender groeperen');
  await win.click('[data-a="toggle:stackSenders"]');
  await expect(win.locator('[data-a="toggle:stackSenders"] .switch')).toHaveClass(/on/);
  await win.click('[data-a="back"]');

  // The unread one keeps its row; the three read ones after it fold into one.
  await expect(item('Your password has changed').locator('.stack-btn')).toHaveCount(0);
  const head = item('Your sign-in details have been updated');
  await expect(head.locator('.stack-btn')).toHaveText('+2');
  await expect(win.locator('.item', { hasText: 'Failed sign-in attempt' })).toHaveCount(0);
  await head.locator('.stack-btn').click();
  await expect(item('Failed sign-in attempt')).toBeVisible();
  await head.locator('.stack-btn').click();
  await expect(win.locator('.item', { hasText: 'Failed sign-in attempt' })).toHaveCount(0);

  // Mail marked unread leaves the stack, so the stack can't hide it.
  await win.evaluate(async () => {
    const list = await window.mail.call('list', { scope: 'all', view: 'inbox' });
    const m = list.find((x) => x.subject.startsWith('Failed sign-in attempt'));
    await window.mail.call('setFlags', m.id, { unread: true });
  });
  await expect(item('Failed sign-in attempt')).toBeVisible();
  await expect(win.locator('.stack-btn')).toHaveCount(0);
});

// #152: a stack says what it is, and the arrow keys open and close it.
test('the arrow keys open and close a stack, also from an email inside it', async () => {
  const head = item('Your sign-in details have been updated');
  const member = win.locator('.list-scroll .item', { hasText: 'How to change your My Vandebron password' });
  const last = win.locator('.list-scroll .item', { hasText: 'Failed sign-in attempt' });
  // Without stacks, the arrows do nothing in the list.
  await head.click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('ArrowLeft');
  await win.keyboard.press('ArrowRight');
  await expect(win.locator('.reader-subject')).toHaveText('Your sign-in details have been updated');
  await expect(last).toBeVisible();

  await win.evaluate(() => window.mail.call('updateSettings', { stackSenders: true }));
  const chip = head.locator('.stack-btn');
  await expect(chip).toHaveText('+2');
  await expect(chip.locator('svg')).toHaveCount(1);
  await expect(chip).toHaveAttribute('aria-label', 'Nog 2 e-mails van Vandebron No Reply');
  await expect(chip).toHaveAttribute('title', 'Nog 2 e-mails van Vandebron No Reply (→)');
  await expect(last).toHaveCount(0);
  // The row keeps room for the edge of the pile under it.
  await expect(win.locator('.item-wrap.folded')).toHaveCount(1);
  await expect(win.locator('.item-wrap.folded')).toHaveCSS('padding-bottom', '4px');

  await win.locator('.list-scroll').focus();
  await win.keyboard.press('ArrowRight');
  await expect(last).toBeVisible();
  await expect(win.locator('.item-wrap.folded')).toHaveCount(0);
  await expect(chip).toHaveAttribute('aria-expanded', 'true');
  await expect(chip).toHaveAttribute('title', 'Inklappen (←)');
  await win.keyboard.press('ArrowLeft');
  await expect(last).toHaveCount(0);
  await expect(chip).toHaveAttribute('aria-expanded', 'false');

  // From an email inside, ← closes the stack and opens its first row.
  await win.keyboard.press('ArrowRight');
  await win.keyboard.press('ArrowDown');
  await expect(win.locator('.reader-subject')).toHaveText('How to change your My Vandebron password');
  await expect(member).toHaveClass(/selected/);
  await win.keyboard.press('ArrowLeft');
  await expect(member).toHaveCount(0);
  await expect(win.locator('.reader-subject')).toHaveText('Your sign-in details have been updated');
  await expect(head).toHaveClass(/selected/);

  // A row outside a stack leaves the arrows alone.
  await item('Sign in to Bencompare').click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('ArrowRight');
  await expect(last).toHaveCount(0);
  await expect(win.locator('.reader-subject')).toHaveText('Sign in to Bencompare');
});

test('related emails under the open email open the rest of the conversation', async () => {
  await item('Call on Thursday').click();
  const related = win.locator('.reader .related');
  await expect(related.locator('.rel-toggle')).toHaveText('1 gerelateerde e-mail');
  const answer = related.locator('.rel');
  await expect(answer.locator('.rel-who')).toHaveText('Jij');
  await expect(answer.locator('.rel-text')).toContainText('Thursday at 10:00 works for me');
  await expect(answer.locator('.pill')).toHaveText('Verzonden');

  // The answer is in Sent, which the list doesn't show; it opens in the reading pane all the same.
  await answer.click();
  await expect(win.locator('.reader-subject')).toHaveText('Re: Call on Thursday');
  await expect(win.locator('.list-scroll .item.selected')).toHaveCount(0);
  const back = related.locator('.rel', { hasText: 'Sanne de Vries' });
  await expect(back.locator('.pill')).toHaveText('Postvak IN');
  await back.click();
  await expect(win.locator('.reader-subject')).toHaveText('Call on Thursday');
  await expect(item('Call on Thursday')).toHaveClass(/selected/);

  // Folded away, the list stays folded for the next email too.
  await related.locator('.rel-toggle').click();
  await expect(related).toHaveClass(/closed/);
  await expect(related.locator('.rel-toggle')).toHaveAttribute('aria-expanded', 'false');
  await item('Re: Dinner on Saturday').click();
  await expect(win.locator('.reader-subject')).toHaveText('Re: Dinner on Saturday');
  await expect(related).toHaveClass(/closed/);
  await related.locator('.rel-toggle').click();
  await expect(related.locator('.rel')).toContainText('Saturday works for me too');

  // Mail without a conversation has no section.
  await item('Bencompare').click();
  await expect(win.locator('.reader-subject')).toHaveText('Sign in to Bencompare');
  await expect(related).toHaveCount(0);

  // Opening a related email lets go of a checked one, so Delete removes the email on screen.
  await item('Call on Thursday').click();
  await item('Call on Thursday').locator('[data-check]').click();
  await expect(item('Call on Thursday')).toHaveClass(/checked/);
  await related.locator('.rel').click();
  await expect(win.locator('.reader-subject')).toHaveText('Re: Call on Thursday');
  await expect(win.locator('.list-scroll .item.checked')).toHaveCount(0);
  await win.keyboard.press('Delete');
  const inTrash = () => win.evaluate(async () => (await window.mail.call('list', { scope: 'all', view: 'trash' })).map((m) => m.subject));
  await expect.poll(inTrash).toContain('Re: Call on Thursday');
  expect(await inTrash()).not.toContain('Call on Thursday');
  await expect(item('Call on Thursday')).toBeVisible();

  // A long conversation shows four rows, and the rest on request with the keyboard on the first new row.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    engine.related = (id) => engine.listMessages({ view: 'inbox' }).filter((m) => m.id !== id).slice(0, 7);
  });
  await item('Call on Thursday').click();
  await expect(related.locator('.rel-toggle')).toHaveText('7 gerelateerde e-mails');
  await expect(related.locator('.rel')).toHaveCount(4);
  await related.locator('.rel-more').click();
  await expect(related.locator('.rel')).toHaveCount(7);
  await expect(related.locator('.rel').nth(4)).toBeFocused();
  await expect(related.locator('.rel-more')).toHaveCount(0);
});

test('attachments are listed for messages that have them', async () => {
  await item('Vandebron').click();
  await expect(win.locator('.att-name')).toHaveText('Contract-confirmation.pdf');
  await expect(win.locator('.att .ftype')).toHaveText('PDF');
  await expect(win.locator('.atts-head')).toHaveCount(0);
});

// #56: a large message with attachments goes into the demo inbox, and its downloads wait until the test lets them go:
// the background download always, the user's when holdUser is set. A banner over the inline-picture budget (a 40 x 30
// PNG with padding after its end) comes with the whole message only.
async function largeMessage({ holdUser = false } = {}) {
  await app.evaluate(
    async ({ app, nativeImage }, holdUser) => {
      // The main process here only resolves Node's own modules; the app's come through its own path.
      const { createRequire } = process.mainModule.require('module');
      const MailComposer = createRequire(`${app.getAppPath()}/package.json`)('nodemailer/lib/mail-composer');
      const engine = global.__semEngine;
      const acc = engine.accounts[0];
      const png = (bgra) => nativeImage.createFromBitmap(Buffer.alloc(40 * 30 * 4).map((_, i) => bgra[i % 4]), { width: 40, height: 30 }).toPNG();
      const paragraphs = Array.from({ length: 30 }, (_, i) => `<p>Section ${i + 1}: revenue, costs and the outlook for the next quarter.</p>`).join('');
      const raw = await new MailComposer({
        from: { name: 'Sanne de Vries', address: 'sanne@example.com' },
        to: 'demo@example.com',
        subject: 'Quarterly report',
        html: `<p id="intro">The quarterly report is attached.</p><img id="banner" src="cid:banner@example.com" width="40" height="30" alt="">${paragraphs}`,
        attachments: [
          { filename: 'banner.png', content: Buffer.concat([png([0xd0, 0x80, 0x30, 0xff]), Buffer.alloc(2200 * 1024)]), cid: 'banner@example.com' },
          { filename: 'Quarterly-report.pdf', content: Buffer.alloc(3 * 1024 * 1024, 0x25), contentType: 'application/pdf' },
          { filename: 'chart.png', content: png([0x40, 0xa0, 0x20, 0xff]) }
        ]
      })
        .compile()
        .build();
      await engine.session(acc).append('INBOX', raw);
      await engine.syncAll();
      while (engine.prefetching.size) await new Promise((r) => setTimeout(r, 20));
      const session = engine.session(acc);
      const real = session.fetchSource.bind(session);
      global.__fetches = [];
      global.__held = { background: [], user: [] };
      session.fetchSource = (p, u, opts = {}) => {
        const kind = opts.background ? 'background' : 'user';
        global.__fetches.push(kind);
        if (kind === 'user' && !holdUser) return real(p, u, opts);
        return new Promise((resolve, reject) =>
          global.__held[kind].push({ go: () => real(p, u, opts).then(resolve, reject), fail: () => reject(new Error('Connection lost')) })
        );
      };
    },
    holdUser
  );
  // The save dialog answers with a file in the profile.
  await app.evaluate(({ dialog }, dir) => {
    const path = process.mainModule.require('path');
    dialog.showSaveDialog = async (_win, opts) => ({ canceled: false, filePath: path.join(dir, path.basename(opts.defaultPath)) });
  }, dataDir);
}
const release = (kind) => app.evaluate((_, kind) => global.__held[kind].splice(0).forEach((h) => h.go()), kind);
const failDownload = (kind) => app.evaluate((_, kind) => global.__held[kind].splice(0).forEach((h) => h.fail()), kind);
const fetches = () => app.evaluate(() => global.__fetches);
const mailFrame = () => win.frameLocator('.mail-frame');

test('a large message shows its text and attachment cards before the attachments are in', async () => {
  await largeMessage();
  await item('Quarterly report').click();
  await expect(mailFrame().locator('#intro')).toHaveText('The quarterly report is attached.');
  // The cards, with names and sizes, before anything but the text came in; no thumbnail yet, and the banner keeps
  // its place.
  await expect(win.locator('.att-name')).toHaveText(['Quarterly-report.pdf', 'chart.png']);
  await expect(win.locator('.att').first().locator('.att-size')).toHaveText('3,0 MB');
  await expect(win.locator('.atts-toggle')).toContainText('2 bijlagen');
  await expect(win.locator('.att.image .thumb img')).toHaveCount(0);
  await expect(mailFrame().locator('#banner')).toHaveAttribute('src', /^data:image\/svg\+xml,/);
  // It keeps the box its width and height give it.
  expect(await mailFrame().locator('#banner').evaluate((img) => [img.clientWidth, img.clientHeight])).toEqual([40, 30]);
  expect(await fetches()).toEqual(['background']);

  // Scrolled down, the whole message lands: the thumbnail and the banner appear, and the reader stays where it was.
  const scroll = win.locator('.reader-scroll');
  await scroll.evaluate((el) => (el.scrollTop = 200));
  await release('background');
  await expect(win.locator('.att.image .thumb img')).toHaveCount(1);
  await expect.poll(() => mailFrame().locator('#banner').evaluate((img) => img.naturalWidth)).toBe(40);
  expect(Math.abs((await scroll.evaluate((el) => el.scrollTop)) - 200)).toBeLessThan(4);
  await expect(win.locator('.att').first().locator('.att-size')).toHaveText('3,0 MB');

  await win.locator('[data-save-att]').first().click();
  await expect(win.locator('#toast')).toHaveText('Bijlage opgeslagen');
  expect(fs.statSync(path.join(dataDir, 'Quarterly-report.pdf')).size).toBe(3 * 1024 * 1024);
  expect(await fetches()).toEqual(['background']);
});

test('saving an attachment of a partly shown message fetches the message in the user turn and shows a spinner meanwhile', async () => {
  await largeMessage({ holdUser: true });
  await item('Quarterly report').click();
  await expect(win.locator('.att-name')).toHaveText(['Quarterly-report.pdf', 'chart.png']);
  const save = win.locator('[data-save-att]').first();
  await save.click();
  await expect(save).toHaveAttribute('aria-busy', 'true');
  await expect(save.locator('.spinner')).toBeVisible();
  expect(await fetches()).toEqual(['background', 'user']);
  // The user's download brings the message: the attachment saves whole, and the reader completes without waiting for
  // the background download, which is still held.
  await release('user');
  await expect(win.locator('#toast')).toHaveText('Bijlage opgeslagen');
  expect(fs.statSync(path.join(dataDir, 'Quarterly-report.pdf')).size).toBe(3 * 1024 * 1024);
  await expect(save).toHaveAttribute('aria-busy', 'false');
  await expect(save.locator('svg')).toBeVisible();
  await expect(win.locator('.att.image .thumb img')).toHaveCount(1);
  await expect.poll(() => mailFrame().locator('#banner').evaluate((img) => img.naturalWidth)).toBe(40);
  await release('background');
});

test('a partly shown message whose background download failed completes once an attachment is opened', async () => {
  await largeMessage();
  await item('Quarterly report').click();
  await expect(win.locator('.att-name')).toHaveText(['Quarterly-report.pdf', 'chart.png']);
  await expect.poll(() => app.evaluate(() => global.__held.background.length)).toBe(1);
  // Offline for a moment: the text and the cards stay, without thumbnails.
  await failDownload('background');
  await expect(mailFrame().locator('#intro')).toBeVisible();
  await expect(win.locator('.att.image .thumb img')).toHaveCount(0);
  await win.locator('[data-save-att]').first().click();
  await expect(win.locator('#toast')).toHaveText('Bijlage opgeslagen');
  expect(fs.statSync(path.join(dataDir, 'Quarterly-report.pdf')).size).toBe(3 * 1024 * 1024);
  await expect(win.locator('.att.image .thumb img')).toHaveCount(1);
  await expect.poll(() => mailFrame().locator('#banner').evaluate((img) => img.naturalWidth)).toBe(40);
  // One download in the user's turn. The reader may have asked for the background download again meanwhile.
  expect((await fetches()).filter((kind) => kind === 'user')).toHaveLength(1);
});

// A full disk: the main process can't write a message to the profile.
const fullDisk = () =>
  app.evaluate(() => {
    const fs = process.mainModule.require('fs');
    const { sep } = process.mainModule.require('path');
    const write = fs.writeFileSync;
    fs.writeFileSync = (file, ...rest) => {
      if (String(file).includes(`${sep}bodies${sep}`)) throw Object.assign(new Error('ENOSPC: no space left on device'), { code: 'ENOSPC' });
      return write.call(fs, file, ...rest);
    };
  });

test('a partly shown message that comes in but cannot be kept still completes', async () => {
  await largeMessage();
  await fullDisk();
  await item('Quarterly report').click();
  await expect(win.locator('.att-name')).toHaveText(['Quarterly-report.pdf', 'chart.png']);
  await expect(win.locator('.att.image .thumb img')).toHaveCount(0);
  await release('background');
  // The whole message came in once but isn't on disk: the reader reads it whole again in the user's turn, and the
  // thumbnail does the same, as for any message that can't be kept.
  await expect(win.locator('.att.image .thumb img')).toHaveCount(1);
  await expect.poll(() => mailFrame().locator('#banner').evaluate((img) => img.naturalWidth)).toBe(40);
  await expect(mailFrame().locator('#intro')).toHaveText('The quarterly report is attached.');
  expect(await fetches()).toEqual(['background', 'user', 'user']);
});

test('star toggles and shows up under Sterren', async () => {
  await item('Bencompare').hover();
  await item('Bencompare').locator('[data-star]').click();
  await expect(item('Bencompare').locator('[data-star]')).toHaveClass(/on/);
  await win.click('[data-view="starred"]');
  await expect(win.locator('.list-title h1')).toHaveText('Sterren');
  await expect(item('Bencompare')).toBeVisible();
});

// ---------- acting on mail ----------

test('delete moves to Prullenbak and can be undone', async () => {
  await item('Bencompare').click();
  await win.click('[data-reader="delete"]');
  await expect(win.locator('#toast')).toContainText('Prullenbak');
  await expect(win.locator('.item', { hasText: 'Bencompare' })).toHaveCount(0);
  await win.click('#toast button:has-text("Ongedaan maken")');
  await expect(win.locator('#toast')).toContainText('Teruggezet');
  await expect(item('Bencompare')).toBeVisible();
  await expect(win.locator('.reader-subject')).toHaveText('Sign in to Bencompare');
  await win.click('[data-reader="delete"]');
  await win.click('[data-view="trash"]');
  await expect(item('Bencompare')).toBeVisible();
});

test('archive from the toolbar, undo with Ctrl+Z', async () => {
  await item('Bencompare').click();
  await win.click('[data-reader="archive"]');
  await expect(win.locator('#toast')).toContainText('Archief');
  await expect(win.locator('.item', { hasText: 'Bencompare' })).toHaveCount(0);
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Control+z');
  await expect(item('Bencompare')).toBeVisible();
});

// #211: Mark all as read covers what the list shows, has a shortcut and can be undone.
test('Shift+Esc marks what the list shows read, with a filter or a search, and Undo brings it back', async () => {
  const inboxCount = win.locator('.nav-item[data-view="inbox"] .count');
  await expect(inboxCount).toHaveText('7');
  // The More menu and Settings show the shortcut.
  await win.click('[data-action="list-more"]');
  await expect(win.locator('.menu button', { hasText: 'Alles als gelezen markeren' }).locator('kbd')).toHaveText('Shift+Esc');
  await win.keyboard.press('Escape');
  await win.click('[data-action="settings"]');
  const page = win.locator('.page.settings');
  await page.locator('[data-a="keyboard"]').click();
  await expect(page.locator('.row', { hasText: 'Alles als gelezen markeren' }).locator('kbd')).toHaveText('Shift+Esc');
  while (await page.count()) await page.locator('[data-a="back"]').click();

  // With the Starred filter on, only the starred unread email is marked.
  await win.evaluate(async () => {
    const list = await window.mail.call('list', { scope: 'all', view: 'inbox' });
    await window.mail.call('setFlags', list.find((x) => x.subject === 'Call on Thursday').id, { starred: true });
  });
  await win.click('[data-filter="starred"]');
  await expect(item('Call on Thursday')).toHaveClass(/unread/);
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+Escape');
  await expect(win.locator('#toast')).toContainText('1 e-mail als gelezen gemarkeerd');
  await expect(item('Call on Thursday')).not.toHaveClass(/unread/);
  await expect(inboxCount).toHaveText('6');
  await win.click('#toast button:has-text("Ongedaan maken")');
  await expect(item('Call on Thursday')).toHaveClass(/unread/);
  await expect(inboxCount).toHaveText('7');
  await win.click('[data-filter="all"]');

  // A search marks its results and leaves the rest of Inbox unread.
  await win.fill('#search', 'Vandebron');
  await expect(win.locator('.list-title h1')).toHaveText('Zoekresultaten');
  const found = await win.locator('.list-scroll .item.unread').count();
  expect(found).toBeGreaterThan(1);
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+Escape');
  await expect(win.locator('#toast')).toContainText(`${found} e-mails als gelezen gemarkeerd`);
  await expect(win.locator('.list-scroll .item.unread')).toHaveCount(0);
  await expect(inboxCount).toHaveText(String(7 - found));
  await win.keyboard.press('Shift+Escape');
  await expect(win.locator('#toast')).toHaveText('Alles is al gelezen');
  await win.keyboard.press('Escape');
  await expect(win.locator('.list-title h1')).toHaveText('Postvak IN');
  await expect(item('Call on Thursday')).toHaveClass(/unread/);
  await expect(item('You updated some of your Google Account details')).toHaveClass(/unread/);
});

test('with the Unread filter, Shift+Esc keeps the open email, Ctrl+Z undoes it, and an open editor stays', async () => {
  await item('Call on Thursday').click();
  await expect(item('Call on Thursday')).not.toHaveClass(/unread/);
  await win.click('[data-filter="unread"]');
  const rows = win.locator('.list-scroll .item');
  await expect(rows).toHaveCount(7);
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+Escape');
  await expect(win.locator('#toast')).toContainText('6 e-mails als gelezen gemarkeerd');
  // The list empties except for the email on screen, which the Unread filter keeps (#146).
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText('Call on Thursday');
  await expect(win.locator('.reader-subject')).toHaveText('Call on Thursday');
  await expect(win.locator('.nav-item[data-view="inbox"] .count')).toHaveCount(0);
  await win.keyboard.press('Control+z');
  await expect(rows).toHaveCount(7);
  await expect(win.locator('.nav-item[data-view="inbox"] .count')).toHaveText('6');

  // Esc closes the editor; Shift+Esc doesn't.
  await composer(() => win.keyboard.press('Control+r'));
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+Escape');
  await expect(win.locator('#toast')).toContainText('6 e-mails als gelezen gemarkeerd');
  await expect(rows).toHaveCount(1);
  await expect(win.locator('.composer')).toHaveCount(1);
});

test('when the server refuses one email, Mark all as read says so and its Undo still reaches the rest', async () => {
  const inboxCount = win.locator('.nav-item[data-view="inbox"] .count');
  await expect(inboxCount).toHaveText('7');
  // The demo server refuses to mark this one email read.
  await app.evaluate((_, subject) => {
    const engine = global.__semEngine;
    const m = engine.listMessages({ scope: engine.accounts[0].id, view: 'inbox' }).find((x) => x.subject === subject);
    const { acc, uid } = engine.locate(m.id);
    const session = engine.session(acc);
    const write = session.setFlags.bind(session);
    session.setFlags = async (p, u, patch) => {
      if (u === uid && patch.unread === false) throw new Error('The server refused the change.');
      return write(p, u, patch);
    };
  }, 'Call on Thursday');
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+Escape');
  await expect(win.locator('#toast')).toContainText('6 van 7 e-mails als gelezen gemarkeerd. The server refused the change.');
  await expect(win.locator('#toast button')).toHaveText('Ongedaan maken');
  await expect(item('Call on Thursday')).toHaveClass(/unread/);
  await expect(inboxCount).toHaveText('1');
  // Ctrl+Z still undoes the six it marked.
  await win.keyboard.press('Control+z');
  await expect(win.locator('.list-scroll .item.unread')).toHaveCount(7);
  await expect(inboxCount).toHaveText('7');
});

test('Ctrl+click selects several messages for one action', async () => {
  await item('Bencompare').click();
  // Command on a Mac, where Control+click opens the context menu.
  await item('You updated some of your Google Account details').click({ modifiers: ['ControlOrMeta'] });
  await expect(win.locator('.list-tools.selecting .sel-count')).toHaveText('2 geselecteerd');
  await expect(win.locator('.reader .multi .t')).toHaveText('2 e-mails geselecteerd');
  await win.click('.list-tools [data-bulk="delete"]');
  await expect(win.locator('#toast')).toContainText('2 e-mails verplaatst naar Prullenbak');
  await expect(win.locator('.item', { hasText: 'Bencompare' })).toHaveCount(0);
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await win.click('#toast button');
  await expect(item('Bencompare')).toBeVisible();
  await expect(item('You updated some of your Google Account details')).toBeVisible();
});

test('dragging a message onto a folder moves it', async () => {
  const dt = await win.evaluateHandle(() => new DataTransfer());
  await item('Bencompare').dispatchEvent('dragstart', { dataTransfer: dt });
  const target = win.locator('[data-folder="Invoices"]');
  await target.dispatchEvent('dragover', { dataTransfer: dt });
  await target.dispatchEvent('drop', { dataTransfer: dt });
  // A drag ends with a dragend, which lets the sidebar draw itself again.
  await win.evaluate(() => document.dispatchEvent(new DragEvent('dragend')));
  await expect(win.locator('#toast')).toContainText('Verplaatst naar Invoices');
  await expect(win.locator('.item', { hasText: 'Bencompare' })).toHaveCount(0);
  await win.click('[data-folder="Invoices"]');
  await expect(item('Bencompare')).toBeVisible();
});

test('a new folder can be made from the sidebar', async () => {
  await win.click('[data-action="new-folder"]');
  await win.fill('.scrim .form-input', 'Projects');
  await win.click('.scrim .buttons button:text-is("Opslaan")');
  await expect(win.locator('#toast')).toContainText('Map Projects aangemaakt');
  await expect(win.locator('.list-title h1')).toHaveText('Projects');
  await expect(win.locator('[data-folder="Projects"]')).toBeVisible();
});

// ---------- writing ----------

test('compose and send to self delivers to the inbox and Verzonden', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await expect(c.locator('.composer .from-static')).toContainText('demo@example.com');
  await c.fill('[data-rinput="to"]', 'demo@example.com');
  await c.keyboard.press('Enter');
  await expect(c.locator('[data-rfield="to"] .recipient')).toHaveCount(1);
  await c.fill('[data-field="subject"]', 'Hello from Windows');
  await c.click('.editor');
  await c.keyboard.type('This is a test.');
  await c.click('[data-c="send"]');
  await composerGone();
  await expect(win.locator('#toast')).toContainText('verzonden');
  await expect(item('Hello from Windows')).toBeVisible({ timeout: 10000 });
  await item('Hello from Windows').click();
  await expect(win.frameLocator('.mail-frame').locator('body')).toContainText('This is a test.');
  await expect(win.frameLocator('.mail-frame').locator('body')).not.toContainText('Verzonden vanaf mijn pc');
  await win.click('[data-view="sent"]');
  await expect(item('Hello from Windows')).toBeVisible();
});

test('reply all prefills recipients and subject; earlier mail waits behind the pill', async () => {
  await item('Sanne de Vries').click();
  await expect(win.locator('.reader-subject')).toHaveText('Call on Thursday');
  const c = await composer(() => win.click('[data-reader="replyAll"]'));
  await expect(c.locator('[data-field="subject"]')).toHaveValue('Re: Call on Thursday');
  await expect(c.locator('[data-rfield="to"] .recipient')).toHaveText(['Sanne de Vries']);
  await expect(c.locator('[data-rfield="cc"] .recipient')).toHaveText(['Joris Bakker']);
  await expect(c.locator('.quote')).toBeHidden();
  await c.click('.quote-pill');
  await expect(c.locator('.quote-head')).toContainText('Oorspronkelijk bericht');
  await c.click('[data-c="include"]');
  await expect(c.locator('.quote-pill')).toHaveText('··· niet meegestuurd');
});

test('selected text gets a floating formatting bar', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.click('.editor');
  await c.keyboard.type('Important');
  await expect(c.locator('.float-bar')).toBeHidden();
  await c.keyboard.press('Shift+Home');
  await expect(c.locator('.float-bar')).toBeVisible();
  await c.click('.float-bar [data-cmd="bold"]');
  await expect(c.locator('.float-bar [data-cmd="bold"]')).toHaveClass(/on/);
});

test('the shared editor formats text and inserts links inline and in a compose window, then reopens the saved draft', async () => {
  await composer(() => win.click('[data-action="compose"]'));
  await win.locator('.composer [data-field="subject"]').fill('Shared editor draft');
  const format = async (page, text, command, href) => {
    const editor = page.locator('.composer .editor');
    await editor.fill(text);
    await editor.press('Control+a');
    await page.locator(`.float-bar [data-cmd="${command}"]`).click();
    await editor.press('Control+k');
    await page.locator('.link-pop [data-f="url"]').fill(href);
    await page.locator('.link-pop [data-a="apply"]').click();
    await expect(editor.locator('a')).toHaveAttribute('href', href);
    expect(await editor.locator('*').evaluateAll((els, command) => els.some((el) => command === 'bold' ? Number(getComputedStyle(el).fontWeight) >= 600 : getComputedStyle(el).fontStyle === 'italic'), command)).toBe(true);
  };
  await format(win, 'Inline formatting', 'bold', 'https://example.com/inline');
  const [popup] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  await popup.waitForSelector('.composer .editor');
  await expect(popup.locator('.editor a')).toHaveAttribute('href', 'https://example.com/inline');
  await format(popup, 'Window formatting', 'italic', 'https://example.com/window');
  await popup.locator('.editor').press('Control+s');
  await expect(popup.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await popup.close();
  await win.click('[data-view="drafts"]');
  await item('Shared editor draft').click();
  await win.locator('[data-reader="edit"]').click();
  await expect(win.locator('.composer .editor a')).toHaveAttribute('href', 'https://example.com/window');
  expect(await win.locator('.composer .editor *').evaluateAll((els) => els.some((el) => getComputedStyle(el).fontStyle === 'italic'))).toBe(true);
});

test('forward keeps the original attachment', async () => {
  await item('Sanne de Vries').click();
  await expect(win.locator('.att-name')).toHaveText('Proposal-v3.pdf');
  const c = await composer(() => win.click('[data-reader="forward"]'));
  await expect(c.locator('[data-field="subject"]')).toHaveValue('Fwd: Call on Thursday');
  await expect(c.locator('.compose-attachments')).toContainText('Proposal-v3.pdf');
  await c.fill('[data-rinput="to"]', 'demo@example.com');
  await c.click('[data-c="send"]');
  await composerGone();
  await item('Fwd: Call on Thursday').click();
  await expect(win.locator('.att-name')).toHaveText('Proposal-v3.pdf');
});

test('closing a dirty editor asks and can save to Concepten', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-field="subject"]', 'Half finished');
  await c.keyboard.press('Escape');
  await c.click('.scrim .buttons button:text-is("Opslaan")');
  await composerGone();
  await expect(win.locator('#toast')).toContainText('Concepten');
  await win.click('[data-view="drafts"]');
  await expect(item('Half finished')).toBeVisible();
  await expect(win.locator('[data-view="drafts"] .count')).toHaveText('4');
});

test('Ctrl+S saves a draft; closing afterwards keeps it without asking', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-field="subject"]', 'Saved along the way');
  await expect(c.locator('.save-state')).toHaveText('');
  await c.keyboard.press('Control+s');
  await expect(c.locator('.save-state')).toHaveText('Concept opgeslagen');
  await c.keyboard.press('Escape');
  await composerGone();
  await expect(win.locator('#toast')).toContainText('Concept bewaard');
  await win.click('[data-view="drafts"]');
  await expect(item('Saved along the way')).toBeVisible();
});

test('opening another message keeps what you wrote as a draft', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-field="subject"]', 'Away for a moment');
  await item('Bencompare').click();
  await composerGone();
  await expect(win.locator('.reader-subject')).toHaveText('Sign in to Bencompare');
  await expect(win.locator('#toast')).toContainText('Concept opgeslagen');
  await win.click('[data-view="drafts"]');
  await expect(item('Away for a moment')).toBeVisible();
});

test('the editor can move into a window of its own', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-field="subject"]', 'Separate window');
  const [popped] = await Promise.all([app.waitForEvent('window'), c.click('[data-c="popout"]')]);
  await popped.waitForSelector('.compose .editor');
  await expect(popped.locator('[data-field="subject"]')).toHaveValue('Separate window');
  await expect(popped.locator('#compose-title')).toHaveText('Separate window');
  await composerGone();
});

// macOS can report a visibility change for a window that just closed. Electron's own listener then threw "Object has
// been destroyed", and its error box held up the main process and the quit.
test('a visibility change for a window that has closed is ignored', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  const [popped] = await Promise.all([app.waitForEvent('window'), c.click('[data-c="popout"]')]);
  await popped.waitForSelector('.compose .editor');
  const errors = await app.evaluate(({ BrowserWindow }) => {
    const thrown = [];
    for (const w of BrowserWindow.getAllWindows()) {
      w.destroy();
      for (const event of ['show', 'hide', 'minimize', 'maximize', 'restore']) {
        try {
          w.emit(event);
        } catch (err) {
          thrown.push(`${event}: ${err.message}`);
        }
      }
    }
    return thrown;
  });
  expect(errors).toEqual([]);
});

test('a draft opens only once', async () => {
  await win.click('[data-view="drafts"]');
  await win.locator('.item').first().click();
  await composer(() => win.click('[data-reader="edit"]'));
  await win.locator('.item').first().dblclick();
  await win.waitForTimeout(400);
  await expect(win.locator('.composer')).toHaveCount(1);
});

test('a reopened draft cannot restyle or cover the app', async () => {
  const state = await win.evaluate(() => window.mail.call('state'));
  await win.evaluate(
    (accountId) =>
      window.mail.call('saveDraft', {
        accountId,
        to: ['a@example.com'],
        subject: 'Malicious draft',
        html: '<sty<style></style>le>body{display:none}</style><div class="page scrim" style="position:fixed;inset:0;background:red">X</div><p>text</p>'
      }),
    state.accounts[0].id
  );
  await win.click('[data-view="drafts"]');
  await item('Malicious draft').click();
  await expect(win.locator('[data-reader="edit"]')).toBeVisible();
  const c = await composer(() => win.click('[data-reader="edit"]'));
  await expect(c.locator('.compose .editor')).toContainText('text');
  await expect(c.locator('.compose .editor style')).toHaveCount(0);
  await expect(c.locator('.compose .editor .scrim')).toHaveCount(0);
  const fixed = await c.locator('.compose .editor div').first().evaluate((el) => getComputedStyle(el).position);
  expect(fixed).not.toBe('fixed');
  await expect(c.locator('.sidebar')).toBeVisible();
  await expect(c.locator('[data-c="send"]')).toBeVisible();
});

test('a reopened draft keeps its attachment', async () => {
  await item('Sanne de Vries').click();
  const c = await composer(() => win.click('[data-reader="forward"]'));
  await c.fill('[data-rinput="to"]', 'joris@example.com');
  // Enter commits the address; Escape would first close the suggestions.
  await c.keyboard.press('Enter');
  await expect(c.locator('[data-rfield="to"] .recipient')).toHaveCount(1);
  await c.keyboard.press('Escape');
  await c.click('.scrim .buttons button:text-is("Opslaan")');
  await composerGone();
  await win.click('[data-view="drafts"]');
  await item('Fwd: Call on Thursday').click();
  const again = await composer(() => win.click('[data-reader="edit"]'));
  await expect(again.locator('.compose-attachments')).toContainText('Proposal-v3.pdf');
});

test('send as: add an alias, make it default, compose and send from it', async () => {
  await win.click('[data-action="settings"]');
  await win.click('.row:has-text("demo@example.com")');
  await expect(win.locator('[data-a="acc-from"] .title')).toHaveText('Verzenden als');
  await expect(win.locator('[data-a="acc-from"] .value')).toHaveText('demo@example.com');
  await win.click('[data-a="acc-aliases"]');
  await expect(win.locator('.settings h1')).toHaveText('Afzenderadressen');
  await win.click('[data-a="alias-add"]');
  await win.fill('.scrim .form-input', 'bart@bvdm.ai');
  await win.click('.scrim .buttons button:text-is("Opslaan")');
  await expect(win.locator('.settings .row', { hasText: 'bart@bvdm.ai' })).toBeVisible();
  await win.click('.settings .row:has-text("bart@bvdm.ai")');
  await win.click('.scrim .buttons button:has-text("Standaard maken")');
  await expect(win.locator('.settings .row', { hasText: 'bart@bvdm.ai' }).locator('.default-badge')).toHaveText('Standaard');
  await expect(win.locator('.settings .row', { hasText: 'demo@example.com' }).locator('.default-badge')).toHaveCount(0);
  await win.click('[data-a="back"]');
  await expect(win.locator('[data-a="acc-from"] .value')).toHaveText('bart@bvdm.ai');
  await win.click('[data-a="back"]');
  await win.click('[data-a="back"]');

  const c = await composer(() => win.click('[data-action="compose"]'));
  const from = c.locator('[data-field="account"]');
  await expect(from).toBeVisible();
  expect(await from.evaluate((s) => s.selectedOptions[0].textContent)).toContain('bart@bvdm.ai');
  // One account with an alias picks an address, not an account: there is no default account to offer.
  await expect(c.locator('[data-c="make-default"]')).toHaveCount(0);
  await c.fill('[data-rinput="to"]', 'demo@example.com');
  await c.fill('[data-field="subject"]', 'From my alias');
  await c.click('[data-c="send"]');
  await composerGone();
  await item('From my alias').click();
  await expect(win.locator('.msg-head')).toContainText('bart@bvdm.ai');

  // Replying to mail that came from one of your own addresses keeps that address.
  const reply = await composer(() => win.click('[data-reader="reply"]'));
  expect(await reply.locator('[data-field="account"]').evaluate((s) => s.selectedOptions[0].textContent)).toContain('bart@bvdm.ai');
});

// Another account beside the demo one: an IMAP account whose server these tests never reach (port 1 on this
// machine), with a cache that says it synced, so the menu, Settings and the composer show it like any other account.
async function addAccount({ id = 'w1', email = 'werk@example.com', name = 'Werk' } = {}) {
  await app.evaluate((_e, { id, email, name }) => {
    const engine = global.__semEngine;
    const server = { host: '127.0.0.1', port: 1, secure: false, user: email };
    engine.accounts.push({ id, type: 'imap', provider: 'other', email, name, label: email, color: '#ff8a3d', signature: null, secret: 'plain:', imap: server, smtp: server, createdAt: Date.now() });
    engine.caches.set(id, { folders: [], boxes: {}, lastSync: Date.now() });
    engine.emit('updated');
  }, { id, email, name });
}

// The window learns about added accounts through main's update, which may come after the first click.
async function openAccountMenu(last = 'werk@example.com') {
  await expect(async () => {
    await win.click('[data-action="accounts"]');
    await expect(win.locator('.menu button', { hasText: last })).toBeVisible({ timeout: 500 });
  }).toPass();
}

test('the account menu shows each display name with the address underneath, or the address alone', async () => {
  await addAccount({ name: '' });
  const entry = (address) => win.locator('.menu button', { hasText: address });
  await openAccountMenu();
  await expect(entry('demo@example.com').locator('.ml')).toHaveText('Demo Gebruiker');
  await expect(entry('demo@example.com').locator('.ms')).toHaveText('demo@example.com');
  await expect(entry('werk@example.com').locator('.ml')).toHaveText('werk@example.com');
  await expect(entry('werk@example.com').locator('.ms')).toHaveCount(0);
  await win.keyboard.press('Escape');

  // The display name set in Settings is the one the menu shows, as text even when it looks like markup.
  await win.click('[data-action="settings"]');
  await win.click('.settings [data-a="account:w1"]');
  await win.click('[data-a="acc-name"]');
  await win.fill('.scrim .form-input', '<b>Werk</b>');
  await win.click('.scrim .buttons button:text-is("Opslaan")');
  await expect(win.locator('[data-a="acc-name"] .value')).toHaveText('<b>Werk</b>');
  await win.click('[data-a="back"]');
  await win.click('[data-a="back"]');
  await expect(win.locator('.settings')).toHaveCount(0);
  await win.click('[data-action="accounts"]');
  await expect(entry('werk@example.com').locator('.ml')).toHaveText('<b>Werk</b>');
  await expect(entry('werk@example.com').locator('.ms')).toHaveText('werk@example.com');
  await expect(win.locator('.menu b')).toHaveCount(0);
});

test('a tall account menu stays inside a short window at 200 percent and scrolls to its last item', async () => {
  // Four named accounts take two lines each. At 1000 by 580 and 200 percent the page is 290px high.
  await addAccount();
  await addAccount({ id: 'w2', email: 'sport@example.com', name: 'Sportclub' });
  await addAccount({ id: 'w3', email: 'familie@example.com', name: 'Familie' });
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1000, 580));
  await win.evaluate(() => window.mail.call('setZoom', 200));
  await expect.poll(() => win.evaluate(() => window.innerHeight)).toBeLessThan(300);
  await openAccountMenu('familie@example.com');
  const menu = win.locator('.menu');
  const height = await win.evaluate(() => window.innerHeight);
  const box = await menu.boundingBox();
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(height);
  // The menu scrolls instead of running off the window, so its last item is there to press, also by keyboard.
  expect(await menu.evaluate((m) => m.scrollHeight > m.clientHeight)).toBe(true);
  await win.keyboard.press('End');
  const last = menu.locator('button', { hasText: 'Accounts en instellingen' });
  await expect(last).toBeFocused();
  await expect(last).toBeInViewport({ ratio: 1 });
  await win.keyboard.press('Enter');
  await expect(win.locator('.settings h1')).toHaveText('E-mailinstellingen');
});

test('Settings shows the default account and makes another one the default in place; new mail then starts there', async () => {
  await addAccount();
  await win.click('[data-action="settings"]');
  const accountRow = (address) => win.locator('.settings .acc-row', { hasText: address });
  const makeDefault = (address) => accountRow(address).locator('[data-a^="acc-make-default:"]');
  await expect(accountRow('demo@example.com').locator('.default-badge')).toHaveText('Standaard');
  await expect(makeDefault('demo@example.com')).toHaveCount(0);
  await expect(accountRow('werk@example.com').locator('.default-badge')).toHaveCount(0);
  await expect(makeDefault('werk@example.com')).toHaveText('Standaard maken');

  await makeDefault('werk@example.com').click();
  await expect(accountRow('werk@example.com').locator('.default-badge')).toHaveText('Standaard');
  await expect(accountRow('demo@example.com').locator('.default-badge')).toHaveCount(0);
  await expect(makeDefault('demo@example.com')).toBeVisible();
  // The pressed button made way for the badge. The keyboard stays on that account, also after main's update redraws
  // the page a moment later.
  const werk = win.locator('.settings [data-a="account:w1"]');
  await expect(werk).toBeFocused();
  await win.waitForTimeout(300);
  await expect(werk).toBeFocused();
  expect(JSON.parse(fs.readFileSync(path.join(dataDir, 'data', 'settings.json'), 'utf8')).defaultAccountId).toBe('w1');

  // The default account's page says so in its first row, and offers nothing to press there.
  const firstRow = () => win.locator('.settings .card').first().locator('.row').first();
  await werk.click();
  await expect(win.locator('.settings h1')).toHaveText('werk@example.com');
  await expect(firstRow()).toHaveAttribute('data-default-account', '');
  await expect(firstRow()).toContainText('Standaardaccount');
  await expect(win.locator('[data-a="acc-default"]')).toHaveCount(0);
  await win.click('[data-a="back"]');
  await win.click('[data-a="back"]');

  // Across all accounts, a new message starts from the new default.
  await win.click('[data-action="accounts"]');
  await win.click('.menu button:has-text("Alle accounts")');
  const c = await composer(() => win.click('[data-action="compose"]'));
  await expect(c.locator('[data-field="account"]')).toHaveValue('w1|werk@example.com');
  await c.click('[data-c="close"]');
  await composerGone();

  // The other account's page offers it first, above the display name.
  await win.click('[data-action="settings"]');
  await accountRow('demo@example.com').locator('.acc-main').click();
  await expect(firstRow()).toHaveAttribute('data-a', 'acc-default');
  await expect(firstRow()).toContainText('Instellen als standaardaccount');
  await firstRow().click();
  await expect(firstRow()).toHaveAttribute('data-default-account', '');
  await win.click('[data-a="back"]');
  await expect(accountRow('demo@example.com').locator('.default-badge')).toHaveText('Standaard');
  await expect(makeDefault('werk@example.com')).toBeVisible();
});

const defaultAccountId = () => JSON.parse(fs.readFileSync(path.join(dataDir, 'data', 'settings.json'), 'utf8')).defaultAccountId;
const settingsBadge = (address) => win.locator('.settings .acc-row', { hasText: address }).locator('.default-badge');

test('a new message from another account offers Use as default account beside From, and follows Settings', async () => {
  await addAccount();
  const demoId = await app.evaluate(() => global.__semEngine.accounts.find((a) => a.type === 'demo').id);
  await openAccountMenu();
  await win.click('.menu button:has-text("Alle accounts")');
  const c = await composer(() => win.click('[data-action="compose"]'));
  const from = c.locator('.composer [data-field="account"]');
  const link = c.locator('.composer [data-c="make-default"]');
  // The default account needs no offer.
  await expect(from).toHaveValue(`${demoId}|demo@example.com`);
  await expect(link).toBeHidden();
  await from.selectOption('w1|werk@example.com');
  await expect(link).toHaveText('Gebruiken als standaardaccount');

  // By keyboard: the link makes way and From keeps the keyboard.
  await link.focus();
  await win.keyboard.press('Enter');
  await expect(link).toBeHidden();
  await expect(from).toBeFocused();
  await expect(win.locator('.toast')).toHaveText('werk@example.com is nu het standaardaccount');
  await expect.poll(defaultAccountId).toBe('w1');
  await from.selectOption(`${demoId}|demo@example.com`);
  await expect(link).toBeVisible();

  // Settings, in front of the open message, shows the change. Changing the default back there reaches the link.
  await win.click('[data-action="settings"]');
  await expect(settingsBadge('werk@example.com')).toHaveText('Standaard');
  await win.locator('.settings .acc-row', { hasText: 'demo@example.com' }).locator('[data-a^="acc-make-default:"]').click();
  await expect(settingsBadge('demo@example.com')).toHaveText('Standaard');
  await win.click('[data-a="back"]');
  await expect(win.locator('.settings')).toHaveCount(0);
  await expect(link).toBeHidden();
  await from.selectOption('w1|werk@example.com');
  await expect(link).toBeVisible();
});

test('the compose window offers Use as default account too, without touching the message', async () => {
  await addAccount();
  await openAccountMenu();
  await win.keyboard.press('Escape');
  const [popup] = await Promise.all([app.waitForEvent('window'), win.evaluate(() => window.mail.call('openCompose', { mode: 'new', accountId: 'w1' }))]);
  await popup.waitForSelector('.composer .editor');
  const from = popup.locator('[data-field="account"]');
  const link = popup.locator('[data-c="make-default"]');
  await expect(from).toHaveValue('w1|werk@example.com');
  await expect(link).toHaveText('Gebruiken als standaardaccount');
  await expect(popup.locator('.composer')).toHaveAttribute('data-dirty', 'false');

  await link.click();
  await expect(link).toBeHidden();
  await expect(from).toBeFocused();
  await expect(popup.locator('.toast')).toHaveText('werk@example.com is nu het standaardaccount');
  await expect.poll(defaultAccountId).toBe('w1');
  // Nothing in the message changed, so it still closes without asking.
  await expect(popup.locator('.composer')).toHaveAttribute('data-dirty', 'false');
  await win.click('[data-action="settings"]');
  await expect(settingsBadge('werk@example.com')).toHaveText('Standaard');

  // Made the default again in the main window: back in the compose window, the link returns.
  await win.locator('.settings .acc-row', { hasText: 'demo@example.com' }).locator('[data-a^="acc-make-default:"]').click();
  await expect.poll(defaultAccountId).not.toBe('w1');
  await popup.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(link).toBeVisible();
});

test('a reply keeps its own account and offers no default', async () => {
  await addAccount();
  await openAccountMenu();
  await win.click('.menu button:has-text("Alle accounts")');
  await item('ANWB Newsletter').click();
  const c = await composer(() => win.click('[data-reader="reply"]'));
  await c.locator('.composer [data-field="account"]').selectOption('w1|werk@example.com');
  await expect(c.locator('.composer [data-c="make-default"]')).toHaveCount(0);
});

// ---------- finding and layout ----------

test('search filters the list; Escape clears it', async () => {
  await win.fill('#search', 'password');
  await expect(win.locator('.item')).toHaveCount(3);
  await expect(win.locator('.list-title h1')).toHaveText('Zoekresultaten');
  await win.keyboard.press('Escape');
  await expect(win.locator('.list-title h1')).toHaveText('Postvak IN');
});

test('search can cover all folders', async () => {
  await win.fill('#search', 'quote');
  await expect(win.locator('.empty')).toContainText('Niets gevonden');
  await win.click('.search-scope');
  await win.click('.menu button:has-text("Alle mappen")');
  await expect(win.locator('.item').first()).toBeVisible();
  await expect(win.locator('.item .pill').first()).toBeVisible();
});

// #112: mail older than the newest messages Rukoo keeps. The demo inbox keeps 6 of its 16 here instead of 300, and
// Sent 2 of 3; Load older adds 6 at a time, 8 at most.
async function smallWindow() {
  await app.evaluate(async () => {
    const engine = global.__semEngine;
    while (engine.syncs.size) await new Promise((r) => setTimeout(r, 50));
    engine.limits = { ...engine.limits, inbox: 6, default: 2, batch: 6, extra: 8 };
    await engine.syncAccount(engine.accounts[0].id);
  });
}

test('Load older emails at the end of the list brings in older mail, and the unread count stays the same', async () => {
  await smallWindow();
  const row = win.locator('.list-older');
  const inboxCount = win.locator('.nav-item[data-view="inbox"] .count');
  await expect(win.locator('.item')).toHaveCount(6);
  await expect(row).toContainText('nog 10 op de server');
  await expect(inboxCount).toHaveText('7');
  await expect(win.locator('.item', { hasText: 'Call on Thursday' })).toHaveCount(0);

  await slowDown('loadOlder', 600);
  await row.getByRole('button', { name: 'Oudere e-mails laden' }).click();
  await expect(row.getByRole('button', { name: 'Oudere e-mails laden…' })).toBeDisabled();
  await expect(win.locator('.item')).toHaveCount(12);
  await expect(row).toContainText('nog 4 op de server');
  await expect(item('Call on Thursday')).toHaveClass(/unread/);
  // Its unread mail moved from the server's count into the list's.
  await expect(inboxCount).toHaveText('7');
  await expect(win.locator('.filter[data-filter="unread"] .n')).toHaveText('7');

  // Up to the most a folder adds; then the row goes and only a search reaches further back.
  await row.getByRole('button', { name: 'Oudere e-mails laden' }).click();
  await expect(win.locator('.item')).toHaveCount(14);
  await expect(row).toHaveCount(0);
  await win.fill('#search', 'tax administration');
  await expect(item('You have a new message')).toBeVisible();
});

test('a search finds older mail on the server, which opens, deletes and comes back with undo', async () => {
  await smallWindow();
  const row = win.locator('.list-older');
  const sub = win.locator('.list-title .sub');
  // While the server is searched an empty result says so, and the header leaves out a count until it answers.
  await slowDown('searchServer', 800);
  await win.fill('#search', 'parcel');
  await expect(win.locator('.empty')).toContainText('Oudere mail op de server doorzoeken…');
  await expect(sub).toHaveText('"parcel" in Postvak IN');
  await expect(win.locator('.item')).toHaveCount(1);
  await expect(sub).toHaveText('1 voor "parcel" in Postvak IN');
  await expect(row).toHaveText('1 oudere e-mail gevonden op de server');
  await item('Your parcel is on its way').click();
  await expect(win.locator('.reader-subject')).toHaveText('Your parcel is on its way');
  await expect(win.frameLocator('.mail-frame').locator('body')).toContainText('parcel');

  await win.click('[data-reader="delete"]');
  await expect(win.locator('#toast')).toContainText('Prullenbak');
  await expect(win.locator('.item', { hasText: 'Your parcel' })).toHaveCount(0);
  await win.click('#toast button:has-text("Ongedaan maken")');
  await expect(win.locator('#toast')).toContainText('Teruggezet');
  await expect(item('Your parcel is on its way')).toBeVisible();
  // Back in the inbox it is among the newest uids, so the list holds it from now on.
  await win.locator('#search').press('Escape');
  await expect(win.locator('.list-title h1')).toHaveText('Postvak IN');
  await expect(item('Your parcel is on its way')).toBeVisible();
});

test('a search of all folders finds older sent mail, and a server that fails says so and can be tried again', async () => {
  await smallWindow();
  // One folder per call to main: the older contract mail is in Sent, the second folder, so the list asks for the next.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    engine.search = { ...engine.search, folders: 1 };
  });
  const row = win.locator('.list-older');
  await win.fill('#search', 'contract');
  await expect(row).toHaveText('Geen oudere mail op de server gevonden');
  await expect(win.locator('.item')).toHaveCount(2);
  await win.click('.search-scope');
  await win.click('.menu button:has-text("Alle mappen")');
  await expect(item('Question about my contract')).toBeVisible();
  await expect(win.locator('.item')).toHaveCount(3);
  await expect(row).toHaveText('1 oudere e-mail gevonden op de server');

  await app.evaluate(() => {
    const engine = global.__semEngine;
    engine.realSearchServer = engine.searchServer;
    engine.searchServer = async () => ({ more: 0, errors: ['demo@example.com: De server is niet bereikbaar.'] });
  });
  await win.fill('#search', 'dinner');
  await expect(row).toContainText('Rukoo kon oudere mail op de server niet doorzoeken. demo@example.com: De server is niet bereikbaar.');
  await expect(item('Dinner on Saturday')).toBeVisible();
  await app.evaluate(() => {
    const engine = global.__semEngine;
    engine.searchServer = engine.realSearchServer;
  });
  await row.getByRole('button', { name: 'Opnieuw proberen' }).click();
  await expect(item('Re: Dinner on Saturday')).toBeVisible();
  await expect(row).toHaveText('1 oudere e-mail gevonden op de server');
});

test('user folders load on demand', async () => {
  await win.click('[data-folder="Invoices"]');
  await expect(win.locator('.list-title h1')).toHaveText('Invoices');
  await expect(item('Ziggo')).toBeVisible();
});

test('quick filters narrow the list', async () => {
  await win.click('[data-filter="attachments"]');
  await expect(win.locator('.item')).toHaveCount(3);
  await win.click('[data-filter="all"]');
  await expect(win.locator('.item').nth(5)).toBeVisible();
});

test('the Unread filter keeps an opened message until another opens, then folds it away', async () => {
  await win.emulateMedia({ reducedMotion: 'no-preference' });
  await win.click('[data-filter="unread"]');
  await expect(win.locator('.item')).toHaveCount(7);
  await item('Forward Future').click();
  await expect(win.locator('.reader-subject')).toHaveText("The music industry can't agree on AI");
  // Read and selected, but still in the list, so the reader can step from it.
  await expect(item('Forward Future')).not.toHaveClass(/unread/);
  await expect(item('Forward Future')).toHaveClass(/selected/);
  await expect(win.locator('.item')).toHaveCount(7);
  await expect(win.locator('[data-filter="unread"] .n')).toHaveText('6');
  await expect(win.locator('[data-reader="prev"]')).toBeEnabled();
  await expect(win.locator('[data-reader="next"]')).toBeEnabled();
  // Opening the next one folds the read row away before the list drops it.
  const folding = await win.evaluate(() => {
    document.querySelector('[data-reader="next"]').click();
    const wrap = [...document.querySelectorAll('.list-scroll .item-wrap')].find((w) => w.textContent.includes('Forward Future'));
    return Boolean(wrap && wrap.getAnimations().length);
  });
  expect(folding).toBe(true);
  await expect(win.locator('.reader-subject')).toHaveText('Here is another copy of your contract');
  await expect(win.locator('.item.selected')).not.toHaveClass(/unread/);
  await expect(win.locator('.item', { hasText: 'Forward Future' })).toHaveCount(0);
  await expect(win.locator('.item')).toHaveCount(6);
  // With reduced motion the row goes at once.
  await win.emulateMedia({ reducedMotion: 'reduce' });
  const left = await win.evaluate(() => {
    [...document.querySelectorAll('.list-scroll .item')].find((i) => i.textContent.includes('Google')).click();
    return document.querySelectorAll('.list-scroll .item').length;
  });
  expect(left).toBe(5);
});

test('in a narrow window, Back returns to the Unread list with the read message still in it', async () => {
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(820, 700));
  // The newest message opened at the start and now covers the list.
  await win.click('[data-reader="back"]');
  await win.click('[data-filter="unread"]');
  await item('Forward Future').click();
  await expect(win.locator('.reader-subject')).toHaveText("The music industry can't agree on AI");
  await expect(win.locator('.item.selected')).not.toHaveClass(/unread/);
  await win.click('[data-reader="back"]');
  await expect(item('Forward Future')).toBeVisible();
  await expect(item('Forward Future')).not.toHaveClass(/unread/);
  await expect(win.locator('.item')).toHaveCount(7);
  // Opening another one lets it go.
  await item('Google').click();
  await expect(win.locator('.reader-subject')).toHaveText('You updated some of your Google Account details');
  await expect(win.locator('.item.selected')).not.toHaveClass(/unread/);
  await win.click('[data-reader="back"]');
  await expect(win.locator('.item', { hasText: 'Forward Future' })).toHaveCount(0);
  await expect(item('Google')).not.toHaveClass(/unread/);
  await expect(win.locator('.item')).toHaveCount(6);
});

test('the message list can be resized from the keyboard', async () => {
  const before = (await win.locator('.listpane').boundingBox()).width;
  await win.locator('.divider').focus();
  await win.keyboard.press('ArrowRight');
  await win.keyboard.press('ArrowRight');
  await expect.poll(async () => Math.round((await win.locator('.listpane').boundingBox()).width)).toBe(Math.round(before + 48));
  const saved = await win.evaluate(() => JSON.parse(localStorage.getItem('rukoo.layout')).listWidth);
  expect(saved).toBe(Math.round(before + 48));
});

test('at the narrowest list every button for checked emails is inside it and can be clicked, in Dutch and English', async () => {
  await item('Bencompare').click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+ArrowDown');
  await expect(win.locator('.sel-count')).toHaveText('2 geselecteerd');
  // As narrow as the keyboard makes the list.
  await win.locator('.divider').focus();
  let width = 0;
  for (let i = 0; i < 30; i++) {
    await win.keyboard.press('ArrowLeft');
    const now = Math.round((await win.locator('.listpane').boundingBox()).width);
    if (now === width) break;
    width = now;
  }
  // The buttons on screen that sit outside the list, or under something else at their centre.
  const covered = () =>
    win.evaluate(() => {
      const pane = document.querySelector('.listpane').getBoundingClientRect();
      return [...document.querySelectorAll('.list-tools button')]
        .filter((b) => b.offsetParent !== null)
        .filter((b) => {
          const r = b.getBoundingClientRect();
          const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          return r.left < pane.left || r.right > pane.right || !(hit === b || b.contains(hit));
        })
        .map((b) => b.dataset.bulk || b.dataset.action);
    });
  for (const [language, count, move] of [
    ['nl', '2 geselecteerd', 'Verplaatsen...'],
    ['en', '2 selected', 'Move...']
  ]) {
    await win.evaluate((language) => window.mail.call('updateSettings', { language }), language);
    await expect(win.locator('.sel-count')).toHaveText(count);
    expect(await covered(), language).toEqual([]);
    // What no longer fits is in More, which the keyboard reaches.
    await win.locator('.list-tools [data-bulk="more"]').focus();
    await win.keyboard.press('Enter');
    await expect(win.locator('.menu [role="menuitem"]', { hasText: move })).toBeVisible();
    // The language change also sends 'updated', so the list draws once more, a moment later. Here it draws while the
    // menu is open, as after a sync: the list's rows are new, but its buttons stay, so Escape can go back to More.
    await win.evaluate(() => (document.querySelector('.list-scroll').firstElementChild.dataset.drawn = 'before'));
    await win.evaluate(() => window.mail.call('updateSettings', {}));
    await expect(win.locator('.list-scroll [data-drawn]')).toHaveCount(0);
    await win.keyboard.press('Escape');
    await expect(win.locator('.list-tools [data-bulk="more"]')).toBeFocused();
  }
});

// RUK-89: a sync that changes the list's toolbar while one of its menus is open draws a new button in place of the
// one that opened it. Escape goes to the new button, and the menu's items still act on the checked emails.
test('after a sync redraws the toolbar under an open menu, Escape goes to the new More button and items still work', async () => {
  await item('Bencompare').click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+ArrowDown');
  await expect(win.locator('.sel-count')).toHaveText('2 geselecteerd');
  const ids = await win.locator('.item.checked').evaluateAll((rows) => rows.map((r) => r.dataset.id));
  expect(ids).toHaveLength(2);
  const setUnread = (id, unread) => win.evaluate(([id, unread]) => window.mail.call('setFlags', id, { unread }), [id, unread]);
  for (const id of ids) await setUnread(id, false);
  await expect(win.locator('.list-tools [data-bulk="unread"]')).toBeVisible();

  const more = win.locator('.list-tools [data-bulk="more"]');
  // Opens More's menu from the keyboard, then has the toolbar drawn anew under it, as when a sync changes the
  // checked emails: one of them turns unread or read, and the toolbar offers the other action.
  const openThenRedraw = async (unread) => {
    await more.evaluate((b) => (window.__opener = b));
    await more.focus();
    await win.keyboard.press('Enter');
    await expect(win.locator('.menu [role="menuitem"]').first()).toBeFocused();
    await setUnread(ids[0], unread);
    await expect(win.locator(`.list-tools [data-bulk="${unread ? 'read' : 'unread'}"]`)).toBeVisible();
    expect(await win.evaluate(() => window.__opener.isConnected)).toBe(false);
    await expect(win.locator('.menu')).toBeVisible();
  };

  await openThenRedraw(true);
  await win.keyboard.press('Escape');
  await expect(win.locator('.menu')).toHaveCount(0);
  await expect(more).toBeFocused();

  // Star, the second item, picked from a menu whose button was redrawn: both checked emails get the star.
  await openThenRedraw(false);
  await win.keyboard.press('ArrowDown');
  await expect(win.locator('.menu [role="menuitem"]', { hasText: 'Ster toevoegen' })).toBeFocused();
  await win.keyboard.press('Enter');
  await expect(win.locator('.menu')).toHaveCount(0);
  await expect(win.locator('.item.checked .star-btn.on')).toHaveCount(2);
});

// Tab closes a menu and goes on from its button, as from any other button, instead of from the end of the page.
test('Tab and Shift+Tab in a menu go on from the button that opened it', async () => {
  await item('Bencompare').click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+ArrowDown');
  await expect(win.locator('.sel-count')).toHaveText('2 geselecteerd');
  const more = win.locator('.list-tools [data-bulk="more"]');
  await more.focus();
  await win.keyboard.press('Enter');
  await expect(win.locator('.menu')).toBeVisible();
  await win.keyboard.press('Tab');
  await expect(win.locator('.menu')).toHaveCount(0);
  await expect(win.locator('.list-tools [data-bulk="clear"]')).toBeFocused();
  await more.focus();
  await win.keyboard.press('Enter');
  await expect(win.locator('.menu')).toBeVisible();
  await win.keyboard.press('Shift+Tab');
  await expect(win.locator('.menu')).toHaveCount(0);
  await expect(win.locator('.list-tools [data-bulk="delete"]')).toBeFocused();
});

// When the redraw has no copy of the button, Escape puts the focus on the toolbar that held it.
test('Escape in a menu whose button a sync removed goes to the toolbar that held it', async () => {
  await item('Bencompare').click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+ArrowDown');
  await expect(win.locator('.sel-count')).toHaveText('2 geselecteerd');
  const ids = await win.locator('.item.checked').evaluateAll((rows) => rows.map((r) => r.dataset.id));
  await win.locator('.list-tools [data-bulk="more"]').focus();
  await win.keyboard.press('Enter');
  await expect(win.locator('.menu')).toBeVisible();
  // Deleted on another device: the checked emails leave the list, and the toolbar shows the filters again.
  for (const id of ids) await win.evaluate((id) => window.mail.call('remove', id), id);
  await expect(win.locator('.list-tools .filter').first()).toBeVisible();
  await win.keyboard.press('Escape');
  await expect(win.locator('.menu')).toHaveCount(0);
  await expect(win.locator('.list-tools .filter').first()).toBeFocused();
});

test('settings: theme switch and account page', async () => {
  await win.click('[data-action="settings"]');
  await expect(win.locator('.settings h1')).toHaveText('E-mailinstellingen');
  await win.click('[data-a="theme"]');
  await win.click('.radio-row:has-text("Licht")');
  await expect(win.locator('html')).toHaveClass(/light/);
  await win.click('[data-a="theme"]');
  await win.click('.radio-row:has-text("Donker")');
  await expect(win.locator('html')).not.toHaveClass(/light/);
  await win.click('.row:has-text("demo@example.com")');
  await expect(win.locator('.settings h1')).toHaveText('demo@example.com');
  await win.click('[data-a="back"]');
  await win.click('[data-a="back"]');
  await expect(win.locator('.settings')).toHaveCount(0);
});

test('swipe on a touch screen toggles read and deletes', async () => {
  const row = () => item('Your password has changed');
  const swipe = async (from, to) => {
    await expect(row()).toBeVisible();
    // The list may re-render after a sync; wait for a stable row.
    let box = null;
    await expect.poll(async () => (box = await row().boundingBox())).not.toBeNull();
    const y = box.y + box.height / 2;
    const el = row();
    const fire = (type, x) =>
      el.dispatchEvent(type, { pointerType: 'touch', pointerId: 7, isPrimary: true, button: 0, buttons: type === 'pointerup' ? 0 : 1, clientX: x, clientY: y });
    await fire('pointerdown', box.x + from);
    for (let i = 1; i <= 8; i++) await fire('pointermove', box.x + from + ((to - from) * i) / 8);
    await fire('pointerup', box.x + to);
  };
  await expect(row()).toHaveClass(/unread/);
  await swipe(60, 240);
  await expect(row()).not.toHaveClass(/unread/);
  await swipe(300, 60);
  await expect(win.locator('.item', { hasText: 'Your password has changed' })).toHaveCount(0);
  await expect(win.locator('#toast')).toContainText('Prullenbak');
});

// Real touch input through the DevTools protocol, which the dispatched pointer events above can't stand in for: the
// touch starts with the pointer captured by the element under the finger, and the swipe moves that capture to the
// row. A swipe must survive that move (lostpointercapture on the subject) and still reach its action.
test('a real touch swipe that starts on the subject toggles read', async () => {
  const row = () => item('Your password has changed');
  await expect(row()).toHaveClass(/unread/);
  let box = null;
  await expect.poll(async () => (box = await row().locator('.subject').boundingBox())).not.toBeNull();
  const x = box.x + 8;
  const y = box.y + box.height / 2;
  const cdp = await win.context().newCDPSession(win);
  const touch = (type, at) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' ? [] : [{ x: at, y, id: 1 }] });
  await touch('touchStart', x);
  for (let dx = 15; dx <= 180; dx += 15) await touch('touchMove', x + dx);
  await touch('touchEnd', x + 180);
  await cdp.detach();
  await expect(row()).not.toHaveClass(/unread/);
});

test('keyboard: arrows navigate, Ctrl+N composes, Escape closes', async () => {
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('ArrowDown');
  await expect(win.locator('.reader-subject')).toContainText('music industry');
  await win.keyboard.press('ArrowUp');
  await expect(win.locator('.reader-subject')).toHaveText('Traffic fines in 2027: what you will pay');
  await win.keyboard.press('Shift+ArrowDown');
  await expect(win.locator('.sel-count')).toHaveText('2 geselecteerd');
  await win.keyboard.press('Escape');
  await expect(win.locator('.list-tools.selecting')).toHaveCount(0);
  await composer(() => win.keyboard.press('Control+n'));
  await win.keyboard.press('Escape');
  await composerGone();
});

// ---------- regressions ----------

test('after Shift+Down and Escape, Delete removes the message on screen', async () => {
  await item('Bencompare').click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+ArrowDown');
  await expect(win.locator('.sel-count')).toHaveText('2 geselecteerd');
  await win.keyboard.press('Escape');
  await expect(win.locator('.reader-subject')).toHaveText('Sign in to Bencompare');
  await win.keyboard.press('Delete');
  await expect(win.locator('.item', { hasText: 'Bencompare' })).toHaveCount(0);
  await expect(item('You updated some of your Google Account details')).toBeVisible();
});

test('shrinking a selection to one message leaves the multi-message panel', async () => {
  await item('Bencompare').click();
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Shift+ArrowDown');
  await expect(win.locator('.reader .multi')).toBeVisible();
  await win.keyboard.press('Shift+ArrowUp');
  await expect(win.locator('.sel-count')).toHaveText('1 geselecteerd');
  await expect(win.locator('.reader .multi')).toHaveCount(0);
  await expect(win.locator('.reader-subject')).toHaveText('Sign in to Bencompare');
});

test('clicking Send twice during a save sends once', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-rinput="to"]', 'demo@example.com');
  await c.keyboard.press('Enter');
  await c.fill('[data-field="subject"]', 'Only once');
  await slowDown('saveDraft', 1500);
  await slowDown('send', 0);
  await c.keyboard.press('Control+s');
  await c.locator('[data-c="send"]').click({ force: true, noWaitAfter: true });
  await c.locator('[data-c="send"]').click({ force: true, noWaitAfter: true }).catch(() => {});
  await composerGone();
  expect(await calls('send')).toBe(1);
});

test('removing a recipient during a save keeps the editor dirty', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-rinput="to"]', 'a@example.com, b@example.com,');
  await expect(c.locator('[data-rfield="to"] .recipient')).toHaveCount(2);
  await c.fill('[data-field="subject"]', 'Recipients');
  await slowDown('saveDraft', 1200);
  await c.keyboard.press('Control+s');
  await expect(c.locator('.save-state')).toHaveText('Opslaan...');
  await c.locator('[data-rfield="to"] .recipient').first().locator('button').click();
  await expect(c.locator('[data-rfield="to"] .recipient')).toHaveCount(1);
  await expect(c.locator('.save-state')).not.toHaveText('Opslaan...', { timeout: 5000 });
  await expect(c.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  await c.keyboard.press('Escape');
  await expect(c.locator('.scrim')).toContainText('Concept opslaan?');
});

test('an attachment removed during a save stays removed', async () => {
  await item('Sanne de Vries').click();
  const c = await composer(() => win.click('[data-reader="forward"]'));
  await expect(c.locator('.att-chip')).toHaveCount(1);
  await slowDown('saveDraft', 1200);
  await c.keyboard.press('Control+s');
  await c.locator('.att-chip button').click();
  await expect(c.locator('.att-chip')).toHaveCount(0);
  await expect(c.locator('.save-state')).not.toHaveText('Opslaan...', { timeout: 5000 });
  await expect(c.locator('.composer')).toHaveAttribute('data-dirty', 'true');
  await c.waitForTimeout(300);
  await expect(c.locator('.att-chip')).toHaveCount(0);
});

test('a failed read-back after saving is retried before sending', async () => {
  await item('Sanne de Vries').click();
  const c = await composer(() => win.click('[data-reader="forward"]'));
  await c.fill('[data-rinput="to"]', 'demo@example.com');
  await c.keyboard.press('Enter');
  // The first read of the new draft fails, as on a dropped connection.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    const orig = engine.getMessage.bind(engine);
    let failed = false;
    engine.getMessage = async (id) => {
      if (!failed && decodeURIComponent(String(id)).includes('Drafts')) {
        failed = true;
        throw new Error('Verbinding verbroken');
      }
      return orig(id);
    };
  });
  await c.keyboard.press('Control+s');
  await expect(c.locator('.save-state')).toHaveText('Concept opgeslagen');
  await c.click('[data-c="send"]');
  await composerGone();
  await item('Fwd: Call on Thursday').click();
  await expect(win.locator('.att-name')).toHaveText('Proposal-v3.pdf');
});

test('the window cannot be closed while the editor is sending', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-rinput="to"]', 'demo@example.com');
  await c.keyboard.press('Enter');
  await c.fill('[data-field="subject"]', 'Do not close');
  await c.keyboard.press('Control+s');
  await expect(c.locator('.save-state')).toHaveText('Concept opgeslagen');
  await slowDown('send', 1500);
  await c.click('[data-c="send"]');
  // The window's close button fires beforeunload; a prevented one keeps an Electron window open.
  const blocksClose = () =>
    win.evaluate(() => {
      const e = new Event('beforeunload', { cancelable: true });
      window.dispatchEvent(e);
      return e.defaultPrevented;
    });
  expect(await blocksClose()).toBe(true);
  await composerGone();
  expect(await calls('send')).toBe(1);
});

test('saving before closing locks the editor until the save is done', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-field="subject"]', 'Latest version');
  await c.click('.editor');
  await c.keyboard.type('first line');
  await c.keyboard.press('Escape');
  await slowDown('saveDraft', 1200);
  await c.click('.scrim .buttons button:text-is("Opslaan")');
  expect(await c.evaluate(() => document.querySelector('.compose').inert)).toBe(true);
  await composerGone();
  await win.click('[data-view="drafts"]');
  await item('Latest version').click();
  await expect(win.frameLocator('.mail-frame').locator('body')).toContainText('first line');
});

test('Ctrl+Enter during save-and-close sends nothing and keeps the editor locked', async () => {
  const c = await composer(() => win.click('[data-action="compose"]'));
  await c.fill('[data-field="subject"]', 'No recipient');
  await c.keyboard.press('Escape');
  await slowDown('saveDraft', 1200);
  await slowDown('send', 0);
  await c.click('.scrim .buttons button:text-is("Opslaan")');
  await c.keyboard.press('Control+Enter');
  expect(await c.evaluate(() => document.querySelector('.compose').inert)).toBe(true);
  await composerGone();
  expect(await calls('send')).toBe(0);
});

test('without a Prullenbak, delete asks before removing for good', async () => {
  const state = await win.evaluate(() => window.mail.call('state'));
  // Simulate a server whose trash folder was not recognised.
  await app.evaluate((_, id) => {
    const engine = global.__semEngine;
    const cache = engine.caches.get(id);
    cache.folders = cache.folders.filter((f) => f.role !== 'trash');
  }, state.accounts[0].id);
  await item('Bencompare').click();
  await win.click('[data-reader="delete"]');
  await expect(win.locator('.scrim')).toContainText('geen Prullenbak');
  await win.click('.scrim .buttons button:has-text("Annuleren")');
  await expect(item('Bencompare')).toBeVisible();
  await item('Bencompare').click();
  await win.click('[data-reader="delete"]');
  await win.click('.scrim .buttons button:has-text("Verwijderen")');
  await expect(win.locator('#toast')).toContainText('Verwijderd');
  await expect(win.locator('.item', { hasText: 'Bencompare' })).toHaveCount(0);
});

test('a mail frame settles at its content height and keeps the scroll position', async () => {
  const height = () => win.locator('.mail-frame').evaluate((f) => f.offsetHeight);
  const settled = async () => {
    await win.waitForTimeout(400);
    const a = await height();
    await win.waitForTimeout(800);
    expect(await height()).toBe(a);
    return a;
  };
  await item('ANWB Newsletter').click();
  await win.waitForSelector('.mail-frame');
  const h = await settled();
  expect(await win.frameLocator('.mail-frame').locator('html').evaluate((d) => d.scrollHeight)).toBeLessThanOrEqual(h);

  // Mail that sizes itself to the window must not feed on the frame's own height either.
  const state = await win.evaluate(() => window.mail.call('state'));
  await win.evaluate(
    (accountId) =>
      window.mail.call('saveDraft', {
        accountId,
        to: ['a@example.com'],
        subject: 'Full window',
        html: '<table height="100%" width="100%"><tr><td>a</td></tr></table><div style="min-height:100vh">b</div><p id="footer" style="height:50px">footer</p>'
      }),
    state.accounts[0].id
  );
  await win.click('[data-view="drafts"]');
  await item('Full window').click();
  await win.waitForSelector('.mail-frame');
  const frameHeight = await settled();
  expect(frameHeight).toBeLessThan(400);
  // Such mail has no height that fits, so it scrolls inside the frame and its end stays reachable.
  const reach = await win.frameLocator('.mail-frame').locator('html').evaluate((d) => ({
    overflow: d.scrollHeight > d.clientHeight + 1,
    scrollable: getComputedStyle(d).overflowY === 'auto'
  }));
  expect(!reach.overflow || reach.scrollable).toBe(true);

  // Resizing the window refits the mail without moving the reader's scroll position.
  await win.click('[data-view="inbox"]');
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1500, 600));
  await item('ANWB Newsletter').click();
  await win.waitForSelector('.mail-frame');
  await settled();
  await win.locator('.reader-scroll').evaluate((s) => (s.scrollTop = 300));
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1380, 600));
  await settled();
  expect(await win.locator('.reader-scroll').evaluate((s) => s.scrollTop)).toBe(300);
});

// Images a local server holds back until the test lets them through, like a slow tracking pixel.
async function heldImages() {
  const held = new Map();
  const server = http.createServer((req, res) => held.set(req.url, res));
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return {
    base: `http://127.0.0.1:${server.address().port}`,
    held,
    release(url, width, height) {
      held.get(url).writeHead(200, { 'content-type': 'image/svg+xml' });
      held.get(url).end(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"/>`);
    },
    close() {
      for (const res of held.values()) res.destroy();
      server.closeAllConnections();
      server.close();
    }
  };
}

// Saves a draft with this html and opens it in the reader.
async function openDraft(subject, html) {
  const state = await win.evaluate(() => window.mail.call('state'));
  await win.evaluate(
    ([accountId, subject, html]) => window.mail.call('saveDraft', { accountId, to: ['a@example.com'], subject, html }),
    [state.accounts[0].id, subject, html]
  );
  await win.click('[data-view="drafts"]');
  await item(subject).click();
}

test('a mail frame takes its height before slow images arrive, and grows when they do', async () => {
  const images = await heldImages();
  try {
    const { base } = images;
    await openDraft(
      'Slow images',
      `<p>Above the images.</p><p><img src="${base}/sized" width="400" height="300" alt=""></p><p><img src="${base}/unsized" alt=""></p><p>Below the images.</p><img src="${base}/pixel" width="1" height="1" alt="">`
    );
    const height = () => win.locator('.mail-frame').evaluate((f) => f.offsetHeight).catch(() => 0);
    // With every image still on its way, the text and the declared image size are in view.
    await expect.poll(() => images.held.size).toBe(3);
    await expect(win.frameLocator('.mail-frame').locator('body')).toContainText('Below the images.');
    await expect.poll(height).toBeGreaterThan(330);
    const before = await height();
    // The image without a size adds its height when it arrives, while the pixel is still loading.
    images.release('/sized', 400, 300);
    images.release('/unsized', 400, 500);
    await expect.poll(height).toBeGreaterThan(before + 450);
  } finally {
    images.close();
  }
});

test('a newsletter canvas that spans the frame only once its image arrives is still cleared', async () => {
  const images = await heldImages();
  try {
    await openDraft('Late canvas', `<table cellpadding="0" cellspacing="0" style="background:#eeeeee"><tr><td><img src="${images.base}/banner" alt=""></td></tr></table>`);
    const canvas = () =>
      win
        .frameLocator('.mail-frame')
        .locator('table')
        .evaluate((t) => getComputedStyle(t).backgroundColor)
        .catch(() => null);
    // Without its image the table is a few pixels wide: no canvas yet.
    await expect.poll(() => images.held.size).toBe(1);
    await expect.poll(canvas).toBe('rgb(238, 238, 238)');
    images.release('/banner', 2000, 200);
    await expect.poll(canvas).toBe('rgba(0, 0, 0, 0)');
  } finally {
    images.close();
  }
});

test('wide mail shrinks to no less than 80% and scrolls sideways, also with fitting off', async () => {
  // The reader swaps in a new frame when it redraws; a read before the new one is fitted tries again.
  const look = () =>
    win
      .locator('.mail-frame')
      .evaluate((f) => {
        const d = f.contentDocument;
        if (!d || !d.body || !f.style.height) return null;
        const root = d.documentElement;
        return {
          width: f.clientWidth,
          zoom: Number(d.body.style.zoom || 1),
          wide: root.scrollWidth > root.clientWidth,
          sideways: getComputedStyle(root).overflowX === 'auto',
          // The sideways scrollbar must not take the last line's place.
          cutOff: root.scrollHeight > root.clientHeight
        };
      })
      .catch(() => null);
  // The user's wheel, not a script, has to reach the right edge. Returns how far that is, once there.
  const scrollRight = async () => {
    const box = await win.locator('.mail-frame').boundingBox().catch(() => null);
    if (!box) return 0;
    await win.mouse.move(box.x + 100, box.y + 20);
    await win.mouse.wheel(2000, 0);
    return win
      .locator('.mail-frame')
      .evaluate((f) => {
        const s = f.contentDocument && f.contentDocument.scrollingElement;
        // Offsets can be fractional on a scaled display.
        return s && s.scrollLeft >= s.scrollWidth - s.clientWidth - 1 ? Math.round(s.scrollLeft) : 0;
      })
      .catch(() => 0);
  };

  // A newsletter that fits the reading pane keeps its size.
  await item('ANWB Newsletter').click();
  await expect.poll(look).toMatchObject({ zoom: 1, wide: false, cutOff: false });

  // A newsletter built on a 1000 px table, in a reading pane about 550 px wide.
  const state = await win.evaluate(() => window.mail.call('state'));
  await win.evaluate(
    (accountId) =>
      window.mail.call('saveDraft', {
        accountId,
        to: ['a@example.com'],
        subject: 'Wide newsletter',
        html: '<table width="1000" style="width:1000px"><tr><td>Weekly digest</td><td style="width:300px">end of the row</td></tr></table><p style="font-size:11px">Small print, the last line.</p>'
      }),
    state.accounts[0].id
  );
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0].setSize(1244, 950));
  await win.click('[data-view="drafts"]');
  await item('Wide newsletter').click();
  await expect.poll(look).toMatchObject({ zoom: 0.8, wide: true, sideways: true, cutOff: false });
  await expect.poll(async () => (await look())?.width).toBeGreaterThan(530);
  await expect.poll(async () => (await look())?.width).toBeLessThan(570);
  await expect.poll(scrollRight).toBeGreaterThan(200);

  // With "Fit content to window" off the mail keeps its size, and still nothing is cut off.
  await win.click('[data-action="settings"]');
  await win.click('[data-a="toggle:fitContent"]');
  await win.click('[data-a="back"]');
  await expect.poll(look).toMatchObject({ zoom: 1, wide: true, sideways: true, cutOff: false });
  await expect.poll(scrollRight).toBeGreaterThan(400);

  // A few pixels over is no reason for a scrollbar.
  await win.evaluate(
    (accountId) =>
      window.mail.call('saveDraft', {
        accountId,
        to: ['a@example.com'],
        subject: 'Nearly fits',
        html: '<div style="width:calc(100vw + 1px)">A border too far.</div>'
      }),
    state.accounts[0].id
  );
  await item('Nearly fits').click();
  await expect.poll(look).toMatchObject({ zoom: 1, wide: true, sideways: false, cutOff: false });
});

test('newsletter canvases are cleared so the mail sits on the reader', async () => {
  await win.waitForSelector('.mail-frame');
  await expect
    .poll(() => win.frameLocator('.mail-frame').locator('body').evaluate((b) => getComputedStyle(b).backgroundColor))
    .toBe('rgba(0, 0, 0, 0)');
});

test('in dark mode a cleared newsletter canvas keeps its text recoloured', async () => {
  // Clearing a white "background" shorthand makes Chromium write "background-image: initial" into the
  // style attribute. That must not count as an image, which would turn the text back to dark.
  await win.click('[data-action="settings"]');
  await win.click('[data-a="theme"]');
  await win.click('.radio-row:has-text("Donker")');
  await expect(win.locator('html')).not.toHaveClass(/light/);
  await win.click('[data-a="back"]');
  await expect(win.locator('.settings')).toHaveCount(0);
  const state = await win.evaluate(() => window.mail.call('state'));
  await win.evaluate(
    (accountId) =>
      window.mail.call('saveDraft', {
        accountId,
        to: ['a@example.com'],
        subject: 'White canvas',
        html:
          '<table width="100%" style="width:100%;background:#ffffff"><tr><td style="color:#222222">Dark text on white.' +
          '<div id="pointer" style="cursor:url(hand.cur),auto;color:#222222">A custom pointer is no image.</div>' +
          '<div id="photo" style="background-image:url(data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==);height:20px"></div>' +
          '</td></tr></table>'
      }),
    state.accounts[0].id
  );
  await win.click('[data-view="drafts"]');
  await item('White canvas').click();
  const look = (selector) =>
    win
      .frameLocator('.mail-frame')
      .locator(selector)
      .evaluate((el) => ({ background: getComputedStyle(el).backgroundColor, filter: getComputedStyle(el).filter }))
      .catch(() => null);
  await expect.poll(() => look('table')).toEqual({ background: 'rgba(0, 0, 0, 0)', filter: 'none' });
  expect(await win.frameLocator('.mail-frame').locator('html').evaluate((h) => getComputedStyle(h).filter)).toBe('invert(1) hue-rotate(180deg)');
  // A url() in another property is no image either; a real background image is still turned back.
  expect((await look('#pointer')).filter).toBe('none');
  expect((await look('#photo')).filter).toBe('invert(1) hue-rotate(180deg)');
});

test('a reopened reply keeps the earlier mail behind the pill, and still sends it', async () => {
  await item('Sanne de Vries').click();
  const c = await composer(() => win.click('[data-reader="reply"]'));
  await c.click('.editor');
  await c.keyboard.type('See you on Thursday!');
  await c.keyboard.press('Escape');
  await c.click('.scrim .buttons button:text-is("Opslaan")');
  await composerGone();
  await win.click('[data-view="drafts"]');
  await item('Re: Call on Thursday').click();
  await composer(() => win.click('[data-reader="edit"]'));
  await expect(win.locator('.composer .editor')).toContainText('See you on Thursday!');
  await expect(win.locator('.composer .editor')).not.toContainText('Oorspronkelijk bericht');
  await expect(win.locator('.quote-pill')).toBeVisible();
  await win.click('.quote-pill');
  await expect(win.locator('.quote-head')).toContainText('Van: Sanne de Vries');
  await win.fill('[data-rinput="to"]', 'demo@example.com');
  await win.keyboard.press('Enter');
  await win.click('[data-c="send"]');
  await composerGone();
  await win.click('[data-view="inbox"]');
  await item('Re: Call on Thursday').click();
  await expect(win.frameLocator('.mail-frame').locator('body')).toContainText('Oorspronkelijk bericht');
  await expect(win.frameLocator('.mail-frame').locator('body')).toContainText('Shall we have a call on Thursday');
});

test('a folder that fails to sync leaves the folders after it current, and the new mail is announced', async () => {
  await app.evaluate(async ({ Notification, BrowserWindow }) => {
    // Notifications are recorded instead of shown, as for a window in the background.
    BrowserWindow.getAllWindows()[0].isFocused = () => false;
    global.__shown = [];
    Notification.prototype.show = function () {
      global.__shown.push(this);
    };
    const engine = global.__semEngine;
    const s = engine.session(engine.accounts[0]);
    const raw = (subject) =>
      ['From: Sanne de Vries <sanne@example.com>', 'To: demo@example.com', `Subject: ${subject}`, `Date: ${new Date().toUTCString()}`, '', 'Hallo'].join('\r\n');
    // New mail in the inbox, which syncs before Verzonden, and in Archief, which syncs after it.
    await s.append('INBOX', raw('Nieuw in het postvak'));
    await s.append('Archive', raw('Nieuw in het archief'));
    // The server refuses Verzonden, as imapflow reports a folder it cannot open.
    global.__syncFolder = s.syncFolder;
    s.syncFolder = function (folder, options) {
      if (folder === 'Sent') return Promise.reject(Object.assign(new Error('Command failed'), { responseText: 'Sent is temporarily unavailable' }));
      return global.__syncFolder.call(this, folder, options);
    };
  });
  const cached = (view, subject) =>
    app.evaluate((_, [view, subject]) => global.__semEngine.listMessages({ view }).some((m) => m.subject === subject), [view, subject]);
  const announced = () => app.evaluate(() => global.__shown.map((n) => n.body.split('\n')[0]));

  await win.click('.sync-status');
  await expect(win.locator('.sync-status')).toHaveText('Synchronisatie mislukt');
  await expect(win.locator('.sync-status')).toHaveAttribute('title', 'demo@example.com: Sent is temporarily unavailable');
  // The toast for the one account says it as the sidebar does (#135).
  await expect(win.locator('#toast')).toHaveText('Sent is temporarily unavailable');
  await expect(item('Nieuw in het postvak')).toBeVisible();
  await expect.poll(announced).toEqual(['Nieuw in het postvak']);
  // Archief synced in the same run, not when it was opened.
  expect(await cached('archive', 'Nieuw in het archief')).toBe(true);
  await win.click('[data-view="archive"]');
  await expect(item('Nieuw in het archief')).toBeVisible();

  // Once the server opens Verzonden again, the next sync clears the error and announces nothing twice.
  await app.evaluate(() => {
    const engine = global.__semEngine;
    engine.session(engine.accounts[0]).syncFolder = global.__syncFolder;
  });
  await win.click('.sync-status');
  await expect(win.locator('#toast')).toContainText('Gesynchroniseerd');
  await expect(win.locator('.sync-status')).toHaveText('Zojuist bijgewerkt');
  expect(await announced()).toEqual(['Nieuw in het postvak']);
});

test('a sync that times out says so in the toast, in Dutch and in English', async () => {
  await app.evaluate(() => {
    const engine = global.__semEngine;
    engine.session(engine.accounts[0]).syncFolder = () => Promise.reject(new Error('connect ETIMEDOUT 192.0.2.1:993'));
  });
  await win.click('.sync-status');
  await expect(win.locator('.sync-status')).toHaveText('Synchronisatie mislukt');
  await expect(win.locator('#toast')).toHaveText('Er is een time-out opgetreden bij het verbinden met de server.');
  await win.evaluate(() => window.mail.call('updateSettings', { language: 'en' }));
  await expect(win.locator('.sync-status')).toHaveText('Sync failed');
  await win.click('.sync-status');
  await expect(win.locator('#toast')).toHaveText('The connection to the server timed out.');
});

// ---------- regressions from the critique review ----------

test('a reply that is still loading cannot replace a new message', async () => {
  await item('Sanne de Vries').click();
  await expect(win.locator('.reader-subject')).toHaveText('Call on Thursday');
  await slowDown('getMessage', 1500);
  await win.click('[data-reader="reply"]');
  await composer(() => win.click('[data-action="compose"]'));
  await win.fill('[data-field="subject"]', 'Not saved yet');
  await win.waitForTimeout(2000);
  await expect(win.locator('.composer')).toHaveCount(1);
  await expect(win.locator('.composer-title')).toHaveText('Nieuw bericht');
  await expect(win.locator('[data-field="subject"]')).toHaveValue('Not saved yet');
});

test('a draft open in its own window is not opened again inline', async () => {
  await win.click('[data-view="drafts"]');
  await win.locator('.item').first().click();
  await composer(() => win.click('[data-reader="edit"]'));
  const [popped] = await Promise.all([app.waitForEvent('window'), win.click('[data-c="popout"]')]);
  await popped.waitForSelector('.compose .editor');
  await composerGone();
  await win.locator('.item').first().click();
  await win.click('[data-reader="edit"]');
  await win.waitForTimeout(500);
  await expect(win.locator('.composer')).toHaveCount(0);
  expect(app.windows().length).toBe(2);
});

test('while writing, Delete in the list does not touch the hidden message', async () => {
  await expect(win.locator('.reader-subject')).toHaveText('Traffic fines in 2027: what you will pay');
  await composer(() => win.click('[data-action="compose"]'));
  await win.locator('.list-scroll').focus();
  await win.keyboard.press('Delete');
  await win.waitForTimeout(400);
  await expect(item('ANWB Newsletter')).toBeVisible();
  await expect(win.locator('.composer')).toHaveCount(1);
});

test('the newest message opens even when the list is sorted oldest first', async () => {
  await win.evaluate(() => window.mail.call('updateSettings', { sort: 'date-asc' }));
  await win.click('[data-view="drafts"]');
  await win.click('[data-view="inbox"]');
  await expect(win.locator('.reader-subject')).toHaveText('Traffic fines in 2027: what you will pay');
});

test('the newest message opens even when it is folded into a stack', async () => {
  const state = await win.evaluate(() => window.mail.call('state'));
  for (const subject of ['Stack 1', 'Stack 2', 'Stack 3']) {
    // A Date header counts whole seconds; apart, the three have a clear newest.
    if (subject !== 'Stack 1') await win.waitForTimeout(1100);
    await win.evaluate(
      ([accountId, subject]) => window.mail.call('send', { accountId, to: ['demo@example.com'], subject, html: '<p>x</p>' }),
      [state.accounts[0].id, subject]
    );
  }
  await win.evaluate(() => window.mail.call('sync'));
  // Read, because unread mail never folds into a stack.
  await win.evaluate(async () => {
    const list = await window.mail.call('list', { scope: 'all', view: 'inbox' });
    for (const m of list.filter((x) => x.subject.startsWith('Stack '))) await window.mail.call('setFlags', m.id, { unread: false });
  });
  await win.evaluate(() => window.mail.call('updateSettings', { sort: 'date-asc', stackSenders: true }));
  await win.click('[data-view="drafts"]');
  await win.click('[data-view="inbox"]');
  await expect(win.locator('.reader-subject')).toHaveText('Stack 3');
  await expect(item('Stack 3')).toHaveClass(/selected/);
  // Stack 1 heads the stack; the selection keeps it open.
  await expect(win.locator('.item-wrap.stacked .item', { hasText: 'Stack 3' })).toHaveCount(1);
});
