'use strict';

const fs = require('fs');
const path = require('path');
const { fileURLToPath } = require('url');
const mailto = require('./mailto');

function windowChrome(platform, bar) {
  return platform === 'darwin'
    ? { titleBarStyle: 'hidden', trafficLightPosition: { x: 14, y: 13 } }
    : { titleBarStyle: 'hidden', titleBarOverlay: bar };
}

// macOS delivers open-url before ready as well as while the app is running.
// Keep every early link until the services and the main window are ready.
class MailtoQueue {
  constructor(open) { this.open = open; this.pending = []; this.started = false; }
  accept(url) {
    if (!mailto.isMailto(url)) return false;
    if (this.started) this.open(url);
    else this.pending.push(url);
    return true;
  }
  start() {
    this.started = true;
    for (const url of this.pending.splice(0)) this.open(url);
  }
}

// An .ics file that Rukoo is started with ("Open with" on Windows, a second instance): the first argument after the
// program that names an existing .ics file. Switches and the "." of a run from a checkout are skipped; a relative path
// is the starting instance's, from its working directory. Linux desktops may pass a file:// URL instead.
function icsFromArgs(args, cwd = process.cwd(), isFile = fileExists) {
  for (const arg of (args || []).slice(1)) {
    let s = String(arg || '');
    if (/^file:\/\//i.test(s)) {
      try {
        s = fileURLToPath(s);
      } catch (_) {
        continue;
      }
    }
    if (s.startsWith('-') || !/\.ics$/i.test(s)) continue;
    const file = path.resolve(cwd || process.cwd(), s);
    if (isFile(file)) return file;
  }
  return null;
}

function fileExists(file) {
  try {
    return fs.statSync(file).isFile();
  } catch (_) {
    return false;
  }
}

// The same wait for .ics files, which macOS sends with open-file, also before ready.
class FileQueue {
  constructor(open) { this.open = open; this.pending = []; this.started = false; }
  accept(file) {
    if (!file) return false;
    if (this.started) this.open(file);
    else this.pending.push(file);
    return true;
  }
  start() {
    this.started = true;
    for (const file of this.pending.splice(0)) this.open(file);
  }
}

// Launch Services records the mailto: handler by bundle ID and opens links with a copy of the app it knows about.
// A copy run from the mounted DMG or from Downloads may be gone later, ejected or deleted, which leaves email links
// with nowhere to go. So only a copy in /Applications or ~/Applications may register, as Settings already says.
function inApplications(app) {
  try { return app.isInApplicationsFolder() === true; } catch { return false; }
}

class MacMailClient {
  constructor(app, isolated = false) { this.app = app; this.available = app.isPackaged && !isolated && inApplications(app); }
  status() {
    let name = null;
    try { name = this.app.getApplicationNameForProtocol('mailto:') || null; } catch { /* No registered handler. */ }
    return { available: this.available, app: name, rukoo: this.app.isDefaultProtocolClient('mailto'), platform: 'darwin' };
  }
  register() { return this.available && this.app.setAsDefaultProtocolClient('mailto'); }
}

function macMenu(t, edit, zoom) {
  return [
    { role: 'appMenu' },
    { label: t('native.menu.edit'), submenu: [
      { label: t('native.menu.undo'), accelerator: 'Command+Z', click: () => edit('undo') },
      { label: t('native.menu.redo'), accelerator: 'Command+Shift+Z', click: () => edit('redo') },
      { type: 'separator' },
      { role: 'cut' }, { role: 'copy' }, { role: 'paste' }, { role: 'pasteAndMatchStyle' },
      { type: 'separator' },
      { label: t('native.menu.selectAll'), accelerator: 'Command+A', click: () => edit('selectAll') }
    ] },
    { label: t('native.menu.view'), submenu: [
      { label: t('native.menu.zoomIn'), accelerator: 'Command+=', click: () => zoom('in') },
      { label: t('native.menu.zoomIn'), accelerator: 'Command+Plus', visible: false, acceleratorWorksWhenHidden: true, click: () => zoom('in') },
      { label: t('native.menu.zoomOut'), accelerator: 'Command+-', click: () => zoom('out') },
      { label: t('native.menu.actualSize'), accelerator: 'Command+0', click: () => zoom('reset') }
    ] },
    { role: 'windowMenu', submenu: [{ role: 'minimize' }, { role: 'zoom' }, { role: 'close' }, { type: 'separator' }, { role: 'front' }] }
  ];
}

// Settings > General's test notification: shows one and says what came of it. 'unavailable' where the system offers
// Rukoo no notifications, 'failed' with the system's reason (only Windows gives one), else 'sent' once the system shows
// it, or after wait ms without an answer. Sent means the system took it: a Mac that doesn't allow Rukoo's notifications,
// or an unsigned build, can still drop it without a word. keep holds the notification until it closes, so Windows
// doesn't drop its click handler with it.
function testNotice(Notification, options, { keep = new Set(), onClick = () => {}, wait = 5000 } = {}) {
  if (!Notification.isSupported()) return Promise.resolve({ result: 'unavailable' });
  return new Promise((resolve) => {
    const n = new Notification(options);
    keep.add(n);
    const drop = () => keep.delete(n);
    let timer = null;
    const answer = (value) => {
      clearTimeout(timer);
      resolve(value);
    };
    const failed = (error) => {
      drop();
      answer({ result: 'failed', error: String(error || '').slice(0, 300) });
    };
    n.on('show', () => answer({ result: 'sent' }));
    n.on('failed', (_event, error) => failed(error));
    n.on('close', drop);
    n.on('click', () => {
      drop();
      onClick();
    });
    timer = setTimeout(() => answer({ result: 'sent' }), wait);
    if (timer.unref) timer.unref();
    try {
      n.show();
    } catch (err) {
      failed(err && err.message);
    }
  });
}

module.exports = { windowChrome, MailtoQueue, FileQueue, icsFromArgs, MacMailClient, macMenu, testNotice };
