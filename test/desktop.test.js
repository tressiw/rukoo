'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('events');
const { windowChrome, MailtoQueue, MacMailClient, macMenu, testNotice } = require('../src/main/desktop');
const { t } = require('../src/i18n');

test('Mac windows reserve native traffic lights and do not request unsupported title overlays', () => {
  const bar = { color: '#000000', height: 40 };
  assert.deepEqual(windowChrome('darwin', bar), { titleBarStyle: 'hidden', trafficLightPosition: { x: 14, y: 13 } });
  for (const platform of ['win32', 'linux']) assert.equal(windowChrome(platform, bar).titleBarOverlay, bar);
});

test('email URLs arriving before ready are queued once and later URLs open immediately', () => {
  const opened = [];
  const queue = new MailtoQueue((url) => opened.push(url));
  assert.equal(queue.accept('https://example.com'), false);
  assert.equal(queue.accept(null), false);
  assert.equal(queue.accept('mailto:first@example.com'), true);
  queue.accept('mailto:second@example.com?subject=Hello');
  assert.deepEqual(opened, []);
  queue.start();
  queue.start();
  queue.accept('mailto:third@example.com');
  assert.deepEqual(opened, ['mailto:first@example.com', 'mailto:second@example.com?subject=Hello', 'mailto:third@example.com']);
});

test('Mac default mail registration is explicit and never changes a source or isolated installation', () => {
  let writes = 0;
  const app = { isPackaged: false, isInApplicationsFolder: () => true, getApplicationNameForProtocol: () => 'Mail', isDefaultProtocolClient: () => false, setAsDefaultProtocolClient: (scheme) => { assert.equal(scheme, 'mailto'); writes++; return true; } };
  assert.equal(new MacMailClient(app).register(), false);
  app.isPackaged = true;
  assert.equal(new MacMailClient(app, true).register(), false);
  assert.equal(writes, 0);
  const client = new MacMailClient(app);
  assert.deepEqual(client.status(), { available: true, app: 'Mail', rukoo: false, platform: 'darwin' });
  assert.equal(client.register(), true);
  assert.equal(writes, 1);
});

test('only a packaged copy in Applications may become the Mac email app', () => {
  let writes = 0;
  const app = { isPackaged: true, getApplicationNameForProtocol: () => 'Mail', isDefaultProtocolClient: () => false, setAsDefaultProtocolClient: () => { writes++; return true; } };
  // Run from the mounted DMG or from Downloads; or a check that fails, or an Electron without it.
  for (const check of [() => false, () => { throw new Error('no bundle'); }, () => undefined, undefined]) {
    app.isInApplicationsFolder = check;
    const client = new MacMailClient(app);
    assert.equal(client.status().available, false);
    assert.equal(client.register(), false);
  }
  assert.equal(writes, 0);
  // /Applications or ~/Applications: Electron's check covers both.
  app.isInApplicationsFolder = () => true;
  const client = new MacMailClient(app);
  assert.equal(client.status().available, true);
  assert.equal(client.register(), true);
  assert.equal(writes, 1);
});

test('Mac menus retain native quit, editing and window actions while routing mail shortcuts', () => {
  const actions = [];
  const zooms = [];
  const menu = macMenu(t, (action) => actions.push(action), (action) => zooms.push(action));
  assert.equal(menu[0].role, 'appMenu');
  const edit = menu[1].submenu;
  for (const item of edit.filter((item) => item.click)) item.click();
  assert.deepEqual(actions, ['undo', 'redo', 'selectAll']);
  assert.ok(edit.some((item) => item.role === 'copy'));
  for (const item of menu[2].submenu) item.click();
  assert.deepEqual(zooms, ['in', 'in', 'out', 'reset']);
  assert.ok(menu[3].submenu.some((item) => item.role === 'close'));
});

// A stand-in for Electron's Notification: show() does what the test says the system does.
function fakeNotification({ supported = true, onShow = () => {} } = {}) {
  return class extends EventEmitter {
    static isSupported() {
      return supported;
    }
    constructor(options) {
      super();
      this.options = options;
    }
    show() {
      onShow(this);
    }
  };
}

test('the test notification says whether the system showed it, refused it or has none', async () => {
  const keep = new Set();
  const opts = { title: 'Test', body: 'Body' };
  assert.deepEqual(await testNotice(fakeNotification({ supported: false }), opts, { keep }), { result: 'unavailable' });
  assert.equal(keep.size, 0);

  let shown = null;
  let clicks = 0;
  const onShow = (n) => {
    shown = n;
    n.emit('show');
  };
  assert.deepEqual(await testNotice(fakeNotification({ onShow }), opts, { keep, onClick: () => clicks++ }), { result: 'sent' });
  assert.deepEqual(shown.options, opts);
  // It stays referenced while it's on screen, and a click brings Rukoo forward.
  assert.ok(keep.has(shown));
  shown.emit('click');
  assert.equal(clicks, 1);
  assert.equal(keep.size, 0);

  // Windows says why it refused; the reason comes back, and nothing keeps the notification.
  const refuse = (n) => n.emit('failed', {}, 'Notifications are turned off for this app');
  assert.deepEqual(await testNotice(fakeNotification({ onShow: refuse }), opts, { keep }), { result: 'failed', error: 'Notifications are turned off for this app' });
  assert.equal(keep.size, 0);
  const crash = () => {
    throw new Error('No notification service');
  };
  assert.deepEqual(await testNotice(fakeNotification({ onShow: crash }), opts, { keep }), { result: 'failed', error: 'No notification service' });
  assert.equal(keep.size, 0);
});

test('a test notification the system never answers for counts as sent after the wait', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let settled = false;
  const answer = testNotice(fakeNotification(), { title: 'Test' }, { wait: 5000 }).then((r) => {
    settled = true;
    return r;
  });
  t.mock.timers.tick(4999);
  await Promise.resolve();
  assert.equal(settled, false);
  t.mock.timers.tick(1);
  assert.deepEqual(await answer, { result: 'sent' });
});
