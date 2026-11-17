'use strict';

const { t } = require('../i18n');

const path = require('path');
const fs = require('fs');
const os = require('os');
const {
  app,
  BrowserWindow,
  ipcMain,
  dialog,
  shell,
  safeStorage,
  nativeTheme,
  nativeImage,
  Notification,
  Menu,
  screen,
  session
} = require('electron');
const { Engine } = require('./engine');
const { Outbox, Handoff, savedToDrafts, askBeforeQuit } = require('./outbox');
const { cleanSignatureHtml } = require('./signature');
const { Calendars } = require('./calendar');
const { GoogleAuth } = require('./google');
const { VideoServices } = require('./video');
const { RISKY, safeName, markOfTheWeb } = require('./files');
const { WindowState } = require('./windowstate');
const { Logos, siteOf } = require('./logos');
const { oneClickUnsubscribe } = require('./net');
const { mailImage } = require('./mailimage');
const { nextZoom, zoomKey } = require('./zoom');
const { TrayIcon } = require('./tray');
const mailto = require('./mailto');
const { MailClient, SETTINGS_URL } = require('./mailclient');
const { LinuxMailClient, DESKTOP } = require('./linux-mailclient');
const { windowChrome, MailtoQueue, FileQueue, icsFromArgs, MacMailClient, macMenu, testNotice } = require('./desktop');
const { Invitations } = require('./calendar/invitations');
const invite = require('./calendar/invite');
// ---- agents ----
const { clipboard, ClipboardItem, powerMonitor } = require('electron');
const { AgentHub } = require('./agents');
const { AccountService } = require('./account');
const { Categories } = require('./categories');
const { MailNotices } = require('./categories/notices');
const { DiagLog } = require('./diaglog');
const { rukoo: PACKAGE_RUKOO = {} } = require('../../package.json');
const { UpdateChecker, channelFor } = require('./updates');
const { UpdateInstaller, installKind } = require('./installer');
// The version whose "What's new" guide everyone sees once, on the first start of it.
const WHATS_NEW = '2.1.0';
const { PanelGate } = require('./agents/gate');
// ---- /agents ----
const { free: freeTime } = require('./calendar/free');
const { Booking } = require('./booking');
const { ReminderRunner, reminderNotice } = require('./calendar/reminder-runner');
// ---- voice ----
const { VoiceService, VoiceError, bridgePathHere } = require('./voice');
const { isMeetingId, SCREENS: MEETING_SCREENS } = require('./voice/protocol');
// ---- /voice ----

// Windows takes the taskbar icon and the handler for notification clicks from the Start menu shortcut with this ID,
// and Electron writes such a shortcut the first time it shows a notification. A run from a checkout gets its own ID,
// or its shortcut (named Electron, pointing at the checkout) takes over the installed app's icon.
const APP_ID = app.isPackaged ? 'nl.bvdm.rukoo-mail' : 'nl.bvdm.rukoo-mail.dev';
const ICON = path.join(__dirname, '..', '..', 'build', 'icon.png');
const DARK_BAR = { color: '#141519', symbolColor: '#e6e8ee', height: 40 };
const LIGHT_BAR = { color: '#f4f5f7', symbolColor: '#1a1c22', height: 40 };
const RENDERER = path.join(__dirname, '..', 'renderer');
const WEB_PREFERENCES = {
  preload: path.join(__dirname, 'preload.js'),
  contextIsolation: true,
  nodeIntegration: false,
  sandbox: true,
  spellcheck: true
};

// Before the rename to Rukoo Mail the app kept its data in %APPDATA%\E-mail. Electron has already created an
// empty folder under the new name by now, so swap it for the old one. If the old folder is in use, keep using it
// for this run and try again next launch.
function adoptLegacyData() {
  const legacy = path.join(app.getPath('appData'), 'E-mail');
  const target = app.getPath('userData');
  if (!fs.existsSync(path.join(legacy, 'data'))) return;
  if (fs.existsSync(target) && fs.readdirSync(target).length) return;
  try {
    if (fs.existsSync(target)) fs.rmdirSync(target);
    fs.renameSync(legacy, target);
  } catch {
    app.setPath('userData', legacy);
  }
}

if (process.env.SEM_DATA_DIR) app.setPath('userData', process.env.SEM_DATA_DIR);
else adoptLegacyData();
if (process.platform === 'win32') app.setAppUserModelId(APP_ID);
if (process.platform === 'linux') app.setDesktopName(DESKTOP);

let win = null;
// Open compose windows by webContents id, with the options they were opened with.
const composeWindows = new Map();
let windowState = null;
let logos = null;
let engine = null;
let calendars = null;
let google = null;
// Zoom, Microsoft Teams and Jitsi Meet for booked meetings (video/index.js).
let video = null;
let account = null;
let categories = null;
let mailNotices = null;
// The log in the profile, data/logs/rukoo.log (diaglog.js): each sync, and what goes wrong in the windows.
let diag = null;
let updates = null;
let installer = null;
// The user chose Restart to update: the quit installs the new version and starts it.
let restartForUpdate = false;
let syncTimer = null;
// The emails that came in since the main window last had focus, by id; badgeCount() counts the ones still new.
let newSinceFocus = new Set();
// The count the badge shows, so an update that leaves it as it is doesn't draw it again.
let badgeShown = null;
let tray = null;
// Set once Rukoo really quits, so the main window closes instead of hiding in the tray. A window with unsaved
// changes holds the quit while it asks about them, and can call it off; see watchQuit().
let quitting = false;
// The windows (by webContents id) that hold the quit while they ask.
const quitAsks = new Set();
// This quit already asked about the email that waits out its Undo notice, and the answer was Send and quit
// (askQuitWithHeldMail).
let quitAsked = false;
// Emails that wait out their Undo notice after Send (outbox.js).
let outbox = null;
// ---- agents ----
let hub = null;
// Holds the agent events the chat panel must not miss until it listens; see agents/gate.js.
let agentGate = null;
// ---- /agents ----
// The booking service and the reminder runner. Null until they are made after the categories, so calendarFree can already ask.
let booking = null;
let reminders = null;
// Invitations in email and .ics files (calendar/invitations.js).
let invitations = null;
// ---- voice ----
// Rukoo Voice's meetings and the bridge to it (voice/index.js). Made before the windows, so a meeting that records
// hides them from the start.
let voice = null;
// Whether Rukoo's windows should be left out of screen capture now; new windows follow it. windowsHidden: whether
// every window took it, which the meeting view and Settings report.
let captureHidden = false;
let windowsHidden = false;
// ---- /voice ----

if (!process.env.SEM_DATA_DIR && !app.requestSingleInstanceLock()) {
  app.quit();
}

const secrets = require('./secrets').createSecrets(safeStorage);

// Events for the main window only.
function send(type, payload) {
  if (win && !win.isDestroyed()) win.webContents.send('mail:event', { type, payload });
}

function windows() {
  return [win, ...[...composeWindows.values()].map((c) => c.win)].filter((w) => w && !w.isDestroyed());
}

function broadcast(type, payload) {
  for (const w of windows()) w.webContents.send('mail:event', { type, payload });
}

// ---- voice ----
// Every Rukoo window while Rukoo Voice records a meeting, when the user wants that (Settings > Meetings). Windows 10
// 2004 and later leave the window out of capture and screen shares; macOS leaves it out of capture by Rukoo Voice
// and older screen recorders. Linux has no way to, so nothing changes there. Every BrowserWindow, not only
// windows(): a window that isn't listed must not end up in a recording either. false when a window didn't take it:
// setContentProtection doesn't say when Windows refused, so each window is asked what it has now. VoiceService then
// tells Voice nothing and tries again.
function protectWindows(on) {
  captureHidden = Boolean(on) && (process.platform === 'win32' || process.platform === 'darwin');
  let ok = true;
  for (const w of BrowserWindow.getAllWindows()) {
    if (w.isDestroyed()) continue;
    try {
      w.setContentProtection(captureHidden);
      if (w.isContentProtected() !== captureHidden) ok = false;
    } catch (_) {
      ok = false;
    }
  }
  windowsHidden = ok && captureHidden;
  return ok;
}

// What the meeting view and Settings show about Voice. Never the discovery file: its token stays in main.
function voiceState() {
  const base = voice ? voice.state() : { status: 'absent', reason: 'none', voiceVersion: null, protocol: null, older: null, recording: null, dictating: false, screenDefault: null };
  return { ...base, hidden: windowsHidden, canHide: process.platform === 'win32' || process.platform === 'darwin' };
}

// While Rukoo Voice records the screen, Rukoo's own mail and booking notifications would land in its frames, and
// from there with the agent and on disk. Rukoo doesn't show them then, and doesn't show them later either: the badge
// still counts the mail, and calendar reminders still show, because a missed reminder costs more than its title.
// Other apps' notifications aren't ours.
function screenRecording() {
  return Boolean(voice && voice.screenRecording());
}

// Test mode counts the notifications held back, so a test can see that they were.
function heldNotices(n) {
  if (process.env.SEM_HIDDEN) global.__semHeldNotices = (global.__semHeldNotices || 0) + n;
}
// ---- /voice ----

// What the update check found, and how far this install got with it: install is null where Rukoo can't update
// itself, and the sidebar's button opens the release page instead.
function updatesState() {
  return { ...updates.state(), install: installer ? installer.state() : null };
}

function themeColors() {
  const dark = nativeTheme.shouldUseDarkColors;
  return { bar: dark ? DARK_BAR : LIGHT_BAR, background: dark ? DARK_BAR.color : LIGHT_BAR.color };
}

function applyTheme() {
  nativeTheme.themeSource = engine.settings.theme === 'light' ? 'light' : engine.settings.theme === 'dark' ? 'dark' : 'system';
  const { bar, background } = themeColors();
  for (const w of windows()) {
    if (process.platform !== 'darwin') w.setTitleBarOverlay(bar);
    w.setBackgroundColor(background);
  }
}

function scheduleSync() {
  clearInterval(syncTimer);
  const minutes = Number(engine.settings.syncInterval) || 0;
  // Errors are stored per account and shown in the list and the calendar; nothing to do with them here.
  if (minutes > 0) {
    syncTimer = setInterval(() => {
      engine.syncAll().catch(() => {});
      calendars.syncAll().catch(() => {});
    }, minutes * 60000);
  }
}

// The main window coming forward (shown, restored or focused) syncs each account whose last sync started over a
// minute ago, in the background. With only the timer, the inbox could be a whole interval behind right when the user
// looks. Not with manual sync (interval 0).
const RETURN_SYNC_AFTER = 60000;
function syncOnReturn() {
  if (!engine || !(Number(engine.settings.syncInterval) > 0)) return;
  engine.syncStale(RETURN_SYNC_AFTER);
}

function badgeCount() {
  if (engine.settings.badge === 'none') return 0;
  if (engine.settings.badge === 'unread') return engine.unreadCount();
  // Read, moved or put in a category Inbox doesn't show, new mail stops counting.
  return engine.newCount(newSinceFocus);
}

function refreshBadge(count = badgeCount()) {
  badgeShown = count;
  if (process.platform === 'darwin') app.dock?.setBadge(count ? String(count) : '');
  send('badge', count);
  refreshTray(count);
}

// The tray icon is there while closing or minimizing hides the main window, so the window can always come back.
function updateTray() {
  const wanted = engine.settings.closeToTray || engine.settings.minimizeToTray;
  if (wanted && !tray) {
    tray = new TrayIcon({
      icon: ICON,
      platform: process.platform,
      scaleFactor: screen.getPrimaryDisplay().scaleFactor,
      test: Boolean(process.env.SEM_HIDDEN),
      actions: {
        open: showWindow,
        compose: () => openComposeWindow({ mode: 'new' }),
        sync: () => engine.syncAll().catch(() => {}),
        quit: () => app.quit()
      }
    });
    refreshTray();
  }
  if (!wanted && tray) destroyTray();
  if (process.env.SEM_HIDDEN) global.__semTray = tray;
}

// The tooltip counts unread mail. The icon gets a dot while the taskbar badge has a count, because the badge
// isn't visible while the window is hidden.
function refreshTray(badge = badgeCount()) {
  if (tray) tray.update({ unread: engine.unreadCount(), dot: badge > 0 });
}

// Removed before Rukoo exits; an icon left behind stays in the notification area until the mouse passes over it.
function destroyTray() {
  if (tray) tray.destroy();
  tray = null;
  if (process.env.SEM_HIDDEN) global.__semTray = null;
}

// The badge counts first. The notifications get the whole batch: they skip the categories muted on this computer, and
// mail Jev is about to sort waits for its category (categories/notices.js). The main window's cue and sound for new
// mail go the same way, so a muted category stays quiet in the window too.
function notify(messages) {
  for (const m of messages) newSinceFocus.add(m.id);
  refreshBadge();
  if (screenRecording()) {
    heldNotices(messages.length);
    return;
  }
  mailNotices.add(messages);
}

// The new-mail sound (Settings > General), once for mail that comes in together, such as from several accounts in one
// sync. Never while Rukoo Voice records a meeting: the meeting's call track records what the computer plays.
const SOUND_GAP = 2000;
let soundAt = 0;
function newMailSound() {
  if (!engine.settings.newMailSound || !win || win.isDestroyed() || (voice && voice.recording())) return false;
  if (Date.now() - soundAt < SOUND_GAP) return false;
  soundAt = Date.now();
  return true;
}

// A new email's inbox category now: Jev may have sorted it since it came in with one. null while categories are off.
function categoryNow(m) {
  try {
    const { acc, msg } = engine.locate(m.id);
    return engine.categoryOf ? engine.categoryOf(acc, msg) : null;
  } catch (_) {
    return m.category || null;
  }
}

// Now, or once Jev sorted the mail, so the window may have come forward meanwhile, and the screen recording may have
// started. messages: the emails these notices are for. The main window hears of them too (app.js onNewMail): its list
// shows them, or a toast does while the window is in front, where the desktop notification stays away.
function showMailNotices(list, messages = []) {
  if (screenRecording()) {
    heldNotices(list.length);
    return;
  }
  const focused = Boolean(win && !win.isDestroyed() && win.isFocused());
  if (messages.length) {
    send('new-mail', {
      // Each email, so the window can say who one of them is from and where several of them are (newMailTab).
      mails: messages.map((m) => ({
        id: m.id,
        from: String((m.from && (m.from.name || m.from.address)) || '').slice(0, 200),
        subject: String(m.subject || '').slice(0, 200),
        category: categoryNow(m)
      })),
      toast: Boolean(engine.settings.notifications) && focused,
      sound: newMailSound()
    });
  }
  if (!engine.settings.notifications || focused || !Notification.isSupported()) return;
  // With the new-mail sound on, Rukoo's sound plays instead of the system's.
  const silent = engine.settings.newMailSound === true && Boolean(win && !win.isDestroyed());
  for (const { id, title, body } of list) {
    if (!id) {
      new Notification({ title, body, silent }).show();
      continue;
    }
    // nativeImage can read from inside app.asar; a plain path cannot.
    const n = new Notification({ title, body, icon: nativeImage.createFromPath(ICON), silent });
    n.on('click', () => {
      showWindow();
      send('open-message', id);
    });
    n.show();
  }
}

// Shown notifications stay referenced until they close, or Windows can drop their click handler with the object.
const liveNotices = new Set();

// A desktop notification whose click brings the main window forward on the calendar, at the event or request.
// Test mode records them instead of showing them, so a test run never puts toasts on the screen.
function calendarNotice({ title, body, open }) {
  if (process.env.SEM_HIDDEN) {
    (global.__semNotices ||= []).push({ title, body, open });
    return;
  }
  if (!Notification.isSupported()) return;
  const n = new Notification({ title: String(title).slice(0, 200), body: String(body).slice(0, 400), icon: nativeImage.createFromPath(ICON), silent: false });
  liveNotices.add(n);
  const drop = () => liveNotices.delete(n);
  n.on('click', () => {
    drop();
    showWindow();
    if (open) send('calendar-open', open);
  });
  n.on('close', drop);
  n.on('failed', drop);
  n.show();
}

// Calendar reminders: always a desktop notification, also while Rukoo is focused (design 2.8).
function showReminders(list) {
  if (list.length) calendarNotice(reminderNotice(list));
}

// Booking news (design 1.2): a toast while the main window is focused, a desktop notification otherwise. open:
// { id, start } for an event, { request } for a request, { start } for the day a cancelled booking was on. The toast
// is inside Rukoo's window, which a meeting keeps out of the recording; the desktop notification waits while the
// screen records.
function showBookingNotification({ title, body, open }) {
  if (win && !win.isDestroyed() && win.isFocused() && !process.env.SEM_HIDDEN) {
    send('toast', String(body).slice(0, 200));
    return;
  }
  if (screenRecording()) {
    heldNotices(1);
    return;
  }
  calendarNotice({ title, body, open });
}

function showWindow() {
  if (!win || win.isDestroyed()) {
    createWindow();
    return;
  }
  // showInactive() does not restore minimized windows on Linux. Restore in both launch modes.
  if (win.isMinimized()) win.restore();
  // Test mode keeps the window off-screen and leaves the focus alone; see reveal().
  if (process.env.SEM_HIDDEN) {
    win.showInactive();
    return;
  }
  win.show();
  win.focus();
}

function safeUrl(url) {
  return /^(https?:|mailto:)/i.test(String(url || ''));
}

// A mailto: link, from a message, an agent or another app, opens a filled-in compose window; the user sends it.
// Without an account the compose window says so, and the main window comes forward to add one.
function openMailto(url) {
  if (!engine.accounts.length) showWindow();
  openComposeWindow({ mode: 'new', ...mailto.parse(url) });
}

function openUrl(url) {
  if (mailto.isMailto(url)) {
    openMailto(url);
    return;
  }
  if (safeUrl(url)) shell.openExternal(url);
}

// Windows can only offer the installed or portable app for mailto: links. A run from a checkout or a test profile
// leaves the registration alone.
function mailClient() {
  if (process.platform !== 'win32' || !app.isPackaged || process.env.SEM_DATA_DIR) return null;
  // The portable build runs from a temporary copy; Windows has to start the .exe the user keeps.
  const exe = process.env.PORTABLE_EXECUTABLE_FILE || process.execPath;
  return new MailClient({ exe, description: t('settings.general.mailtoAppDescription') });
}

// See MailClient.refresh(): the installed app offers itself to Windows at startup, a portable copy when asked.
async function refreshMailClient() {
  const client = mailClient();
  if (client) await client.refresh({ portable: Boolean(process.env.PORTABLE_EXECUTABLE_FILE) });
}

// Links never navigate an app window; they open in the browser or, for mailto:, in a compose window.
function harden(w) {
  w.webContents.setWindowOpenHandler(({ url }) => {
    openUrl(url);
    return { action: 'deny' };
  });
  w.webContents.on('will-navigate', (e, url) => {
    if (url !== w.webContents.getURL()) {
      e.preventDefault();
      openUrl(url);
    }
  });
  w.webContents.on('will-frame-navigate', (e) => {
    // Clicks inside a mail frame must never replace the frame contents.
    if (!e.isMainFrame && !/^(about:|data:)/.test(e.url)) {
      e.preventDefault();
      openUrl(e.url);
    }
  });
  // Electron's own listener for these events asks the window whether it is visible, which throws once the window is
  // destroyed. macOS can still report a visibility change for a window that just closed, and the error then opens
  // Electron's error box, which holds up the main process, a quit included, until it is closed. A closed window needs
  // none of these events.
  w.once('closed', () => {
    for (const event of ['show', 'hide', 'minimize', 'maximize', 'restore']) w.removeAllListeners(event);
  });
}

// The spelling a window's last right-click in a text field found, by its webContents: { word, suggestions }.
const spelling = new WeakMap();

// Right-click in a text field. Only this event knows whether the word under the pointer is misspelled and what could
// replace it, so it goes to the window, which draws its own menu (the composer's text, editmenu.js); other fields
// ignore it. The word and its suggestions stay here for the pick: a window can replace the word with one of them or
// add that word to the dictionary, and nothing else. An email's frame stops its contextmenu event for its own menu,
// so its right-clicks never get here.
function forwardEditMenu(w) {
  const contents = w.webContents;
  contents.on('context-menu', async (_event, params) => {
    if (!params.isEditable) return;
    const word = params.misspelledWord || '';
    const found = { word, suggestions: word ? (params.dictionarySuggestions || []).slice(0, 5) : [] };
    spelling.set(contents, found);
    // Paste is on when the clipboard holds something. A clipboard that can't be read leaves it on, as Ctrl+V would
    // still try.
    const types = await Promise.resolve()
      .then(() => clipboard.read())
      .then(
        (items) => items.flatMap((item) => item.types),
        () => null
      );
    // A later right-click came in while the clipboard was read: only its answer goes, so the window never pairs this
    // word with the place it was right-clicked last.
    if (contents.isDestroyed() || spelling.get(contents) !== found) return;
    contents.send('mail:event', {
      type: 'edit-menu',
      payload: { ...found, canPaste: !types || types.length > 0, canPastePlain: !types || types.includes('text/plain') }
    });
  });
}

// One zoom level for the main window and every compose window. Settings follows it through the 'zoom' event;
// nothing else in the windows depends on it, so it is saved without the 'updated' redraw.
function setZoom(level) {
  if (level === engine.settings.zoomLevel) return level;
  const { zoomLevel } = engine.updateSettings({ zoomLevel: level }, { quiet: true });
  for (const w of windows()) w.webContents.setZoomFactor(zoomLevel / 100);
  broadcast('zoom', zoomLevel);
  return zoomLevel;
}

// The main process sees the zoom keys before the page does, also with the focus inside an email's frame,
// whose keydown events never reach the page's own handler. Stopping them here keeps them from the page.
// Ctrl+mouse wheel only raises zoom-changed: Electron doesn't zoom by itself. A page starts at the zoom
// Chromium keeps for its file, so the stored level is applied once the page commits.
function watchZoom(w) {
  const contents = w.webContents;
  contents.on('before-input-event', (event, input) => {
    const action = zoomKey(input);
    if (!action) return;
    event.preventDefault();
    setZoom(nextZoom(engine.settings.zoomLevel, action));
  });
  contents.on('zoom-changed', (_event, direction) => setZoom(nextZoom(engine.settings.zoomLevel, direction)));
  contents.on('did-navigate', () => contents.setZoomFactor(engine.settings.zoomLevel / 100));
}

// How a window takes part in a quit. A window with unsaved changes stops a quit from its beforeunload and asks
// what to do with them; Electron then stops quitting. Rukoo holds the quit until every window that asks has
// answered (see answered()) and keeps syncing meanwhile. The main window shows, because its question could
// otherwise wait in the tray.
function watchQuit(w) {
  const id = w.webContents.id;
  w.webContents.on('will-prevent-unload', () => {
    if (!quitting) return;
    quitAsks.add(id);
    scheduleSync();
    if (w === win) showWindow();
  });
  // A window that closes has answered: its changes are saved or let go.
  w.on('closed', () => answered(id, true));
  // A crashed page can't answer any more, and its unsaved changes are gone with it.
  w.webContents.on('render-process-gone', () => answered(id, true));
  w.on('session-end', endSession);
}

// closed: the editor closed after Save or Don't save, so the quit goes on once no other window still asks.
// Otherwise (Cancel, or a save that failed) the window stays open and Rukoo stays running.
function answered(id, closed) {
  if (!quitAsks.delete(id)) return;
  if (!closed) {
    cancelQuit();
    // The main window closed before the quit was called off. Rukoo stays in the tray with a hidden main window, as
    // after any close: the agents' tools need it, and only a window hears that the Windows session ends.
    if (!win && (process.platform === 'darwin' || engine.settings.closeToTray)) createWindow({ hidden: true });
    return;
  }
  // Without windows left, window-all-closed has quit already; a second quit would cut will-quit's wait short.
  if (!quitAsks.size) setImmediate(() => (process.platform === 'darwin' || windows().length) && app.quit());
}

// The quit is called off, by Cancel in a window that asked (answered()) or by Don't quit (askQuitWithHeldMail), and
// Rukoo stays as it was before the quit: closing the main window hides it in the tray again, a refused email opens
// again with its error, and the next quit asks afresh.
function cancelQuit() {
  quitAsks.clear();
  quitting = false;
  restartForUpdate = false;
  quitAsked = false;
  // Held emails that failed while the quit waited, and couldn't go to Drafts, open now that Rukoo stays.
  openUnsent(unsent.splice(0));
}

// Windows is signing out or shutting down. No before-quit or will-quit follows, and Windows ends Rukoo soon after,
// so save now, stop waiting for answers and take the tray icon away. Every window hears it, and the main window may
// already be gone; this runs once, for the first.
let sessionEnded = false;
function endSession() {
  if (sessionEnded) return;
  sessionEnded = true;
  quitting = true;
  quitAsks.clear();
  clearInterval(syncTimer);
  // Refused emails the main window hasn't opened yet go to the list for the next start (quitting is set), so one the
  // window opens now, in a session about to end, can't drop out of that list between the two writes below.
  refused.letGo();
  // Email that waits out its Undo notice goes now, as far as Windows leaves Rukoo the time. It is written down first,
  // so the next start opens what Windows cut off, and again once the sends are done, with only what didn't go.
  keepUnsent();
  if (outbox && outbox.size) {
    outbox
      .flush()
      .then(() => {
        keepUnsent();
        engine.flush();
      })
      .catch(() => {});
  }
  engine.flush();
  if (calendars) calendars.flush();
  if (account) account.close();
  if (categories) categories.close();
  if (mailNotices) mailNotices.close();
  if (reminders) reminders.stop();
  if (booking) booking.close();
  if (updates) updates.stop();
  // Writes what Rukoo keeps of the meetings. Voice keeps recording.
  if (voice) voice.close();
  destroyTray();
  // ---- agents ----
  // dispose() saves the chats before its first await; see will-quit.
  if (hub) closeAgents();
  // ---- /agents ----
}

// Test mode: a never-shown window does not paint, so park it off-screen without focus. hidden: the window starts in
// the tray and shows later, through showWindow().
function reveal(w, maximized = false, hidden = false) {
  w.once('ready-to-show', () => {
    if (process.env.SEM_HIDDEN) {
      w.setPosition(-5000, -5000);
      if (!hidden) w.showInactive();
      return;
    }
    if (hidden) {
      if (maximized) w.once('show', () => w.maximize());
      return;
    }
    if (maximized) w.maximize();
    w.show();
  });
}

const COMPOSE_MODES = new Set(['new', 'reply', 'replyAll', 'forward', 'draft']);

function composeOptions(input = {}) {
  const text = (v, max) => String(v || '').slice(0, max);
  const people = (list) =>
    (Array.isArray(list) ? list : []).slice(0, 100).map((a) => ({ name: text(a && a.name, 200), address: text(a && a.address, 320) }));
  return {
    mode: COMPOSE_MODES.has(input.mode) ? input.mode : 'new',
    id: input.id ? text(input.id, 2000) : null,
    accountId: input.accountId ? text(input.accountId, 200) : null,
    to: people(input.to),
    cc: people(input.cc),
    bcc: people(input.bcc),
    subject: text(input.subject, 1000),
    body: text(input.body, 20000),
    // An editor that opens again after Undo send, or after the server refused an email that waited: what it held
    // (composer.js undoState, which checks each field again), and the server's error.
    restore: input.restore && typeof input.restore === 'object' && !Array.isArray(input.restore) ? input.restore : null,
    sendError: input.sendError ? text(input.sendError, 2000) : null
  };
}

function openComposeWindow(input) {
  const opts = composeOptions(input);
  // A draft that opens again after Undo send had its window closed; it can't be open in another one.
  const restoredDraft = opts.restore && opts.restore.state && typeof opts.restore.state.draftId === 'string' ? opts.restore.state.draftId : null;
  if (opts.mode === 'draft' && opts.id && !opts.restore) {
    const open = [...composeWindows.values()].find((c) => c.draftId === opts.id && !c.win.isDestroyed());
    if (open) {
      open.win.show();
      open.win.focus();
      return true;
    }
  }
  const b = windowState.bounds('compose', { width: 880, height: 780, minWidth: 560, minHeight: 460 });
  // A second compose window opens slightly offset, so the first stays visible behind it.
  const others = windows().filter((w) => w !== win);
  const last = others[others.length - 1];
  const pos = last ? { x: last.getBounds().x + 28, y: last.getBounds().y + 28 } : Number.isFinite(b.x) ? { x: b.x, y: b.y } : {};
  const { bar, background } = themeColors();
  const w = new BrowserWindow({
    width: b.width,
    height: b.height,
    ...pos,
    minWidth: 560,
    minHeight: 460,
    show: false,
    title: t('composer.titles.new'),
    icon: fs.existsSync(ICON) ? ICON : undefined,
    backgroundColor: background,
    ...windowChrome(process.platform, bar),
    webPreferences: WEB_PREFERENCES
  });
  // Before it first shows, so a meeting that records never captures it (protectWindows).
  if (captureHidden) w.setContentProtection(true);
  const id = w.webContents.id;
  composeWindows.set(id, { win: w, opts, draftId: opts.restore ? restoredDraft : opts.mode === 'draft' ? opts.id : null });
  w.on('closed', () => composeWindows.delete(id));
  harden(w);
  forwardEditMenu(w);
  watchZoom(w);
  watchQuit(w);
  if (!process.env.SEM_HIDDEN) windowState.track('compose', w);
  reveal(w);
  w.loadFile(path.join(RENDERER, 'compose.html'));
  return true;
}

async function saveAllAttachments(id) {
  const full = await engine.getMessage(id);
  const list = full.attachments || [];
  if (!list.length) return 0;
  const res = await dialog.showOpenDialog(win, {
    title: t('native.attachments.directory'),
    defaultPath: app.getPath('downloads'),
    properties: ['openDirectory', 'createDirectory']
  });
  if (res.canceled || !res.filePaths[0]) return 0;
  const dir = res.filePaths[0];
  for (const att of list) {
    const a = await engine.attachment(id, att.index);
    const name = safeName(a.filename);
    const ext = path.extname(name);
    let file = path.join(dir, name);
    for (let n = 2; fs.existsSync(file); n++) file = path.join(dir, `${path.basename(name, ext)} (${n})${ext}`);
    fs.writeFileSync(file, a.content);
    markOfTheWeb(file);
  }
  return list.length;
}

async function tempAttachment(id, index) {
  const a = await engine.attachment(id, index);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rukoo-mail-'));
  const file = path.join(dir, safeName(a.filename));
  fs.writeFileSync(file, a.content);
  markOfTheWeb(file);
  return file;
}

// An image in an email, fetched through the windows' session (see mailimage.js).
const emailImage = (src) => mailImage(src, { fetch: (url, opts) => session.defaultSession.fetch(url, opts) });

async function printHtml(html) {
  const printer = new BrowserWindow({ show: false, webPreferences: { javascript: false, sandbox: true } });
  await printer.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`);
  await new Promise((resolve) => printer.webContents.print({}, () => resolve()));
  printer.destroy();
}

// The calendar view's arguments. The calendar store checks the fields of an event again.
const plainObject = (v) => (v && typeof v === 'object' && !Array.isArray(v) ? v : {});
// Text from the renderer for the account calls; anything else reads as empty, which the account refuses.
const accountText = (v, max) => (typeof v === 'string' ? v.slice(0, max) : '');
// The list a call about older mail is for: scope, view, folder and search, as the renderer lists them. resume: the
// server search goes on with the next folders (engine.searchServer()).
const olderOpts = (o) => {
  const v = o && typeof o === 'object' ? o : {};
  return {
    scope: accountText(v.scope, 200) || 'all',
    view: accountText(v.view, 50) || 'inbox',
    folder: accountText(v.folder, 2000) || null,
    query: accountText(v.query, 500),
    resume: v.resume === true
  };
};
// The calendar calls' arguments, checked in calendar/ipc.js (which has its own tests).
const {
  calendarRange,
  calendarInput,
  calendarScope,
  calendarEtag,
  calendarSeriesEtag,
  calendarEventId,
  calendarCalendarId,
  calendarConflicts,
  calendarFreeOpts
} = require('./calendar/ipc');

// ---- undo send ----
// How long Send waits for its Undo notice, in ms (Settings > General > Undo send; outbox.js). Test runs send at once
// unless SEM_UNDO_SEND=1, so the suites that only write mail don't wait out each notice.
function undoSendDelay() {
  if (process.env.SEM_HIDDEN && process.env.SEM_UNDO_SEND !== '1') return 0;
  return (Number(engine.settings.undoSend) || 0) * 1000;
}
// After Don't quit, each email that waits gets what it had left, and at least this, to undo (undoNotices).
const UNDO_AFTER_QUIT_MS = 5000;

// An email went. Sending removed its draft, so an agent's draft cards that pointed at it keep none. id: the email in
// the outbox, whose "Email sent" the main window holds back while a newer email's Undo notice shows.
function mailSent(payload, id = null) {
  if (hub && payload && payload.draftId) hub.draftMoved(String(payload.draftId), null);
  if (id) send('send-done', { id });
  else send('toast', t('composer.status.sent'));
}

// Emails a quit could neither send nor keep in Drafts, and refused emails the main window hadn't taken when the quit
// closed it: { restore, error }. They are written to unsent.json in the profile, and the next start opens each in a
// compose window with its error (reopenUnsent). A quit that is called off opens them right away (answered()).
const unsent = [];
const unsentFile = () => path.join(app.getPath('userData'), 'data', 'unsent.json');
// Whether this run wrote the file, so a later write with nothing left removes only its own.
let unsentWritten = false;

// Refused emails that wait for the main window's reading pane: { id, restore, error }. 'send-failed' only says that
// one is here; the window asks for them (sendFailures) once it listens and has drawn its first state, so a main
// window that is still loading when the server refuses an email, as after a quit called off in a compose window,
// gets it all the same. Main keeps each one until the window says its editor is open with it (refusedOpened). A main
// window that closes or crashes before that, or hasn't claimed one within 15 seconds, leaves it to a compose window,
// and during a quit to the next start. outbox.js Handoff has the rules.
const refused = new Handoff({
  ms: 15000,
  elsewhere: (list) => {
    for (const { restore, error } of list) {
      if (quitting) unsent.push({ restore, error });
      else openComposeWindow({ ...(restore.opts || {}), restore, sendError: error });
    }
  }
});

// Test mode only: lets end-to-end tests see what is held, and shorten the 15 seconds.
if (process.env.SEM_HIDDEN) global.__semRefused = refused;

function holdRefused(item) {
  refused.hold(item);
  send('send-failed', { id: item.id });
}

// Only the main window's page opens held refused emails; a compose window asking gets none.
const fromMainWindow = (sender) => Boolean(win && !win.isDestroyed() && sender === win.webContents);

// Writes down what the quit leaves: the emails refused and not saved or not yet opened, and those still on their
// way, whose fate nobody can know now. They aren't sent again by themselves: the user checks Sent first.
function keepUnsent() {
  const unsure = outbox ? outbox.left().map((e) => ({ restore: e.restore, error: t('composer.undoSend.quitUnsure') })) : [];
  const waiting = refused.list().map(({ restore, error }) => ({ restore, error }));
  const list = [...unsent, ...waiting, ...unsure].filter((u) => u.restore);
  try {
    if (list.length) {
      fs.writeFileSync(unsentFile(), JSON.stringify(list));
      unsentWritten = true;
    } else if (unsentWritten) {
      fs.rmSync(unsentFile(), { force: true });
      unsentWritten = false;
    }
  } catch (_) {
    // A profile that can't be written to keeps nothing.
  }
}

function openUnsent(list) {
  for (const u of list) {
    if (u && u.restore && typeof u.restore === 'object') openComposeWindow({ ...(u.restore.opts || {}), restore: u.restore, sendError: String(u.error || '') });
  }
}

function reopenUnsent() {
  let list;
  try {
    list = JSON.parse(fs.readFileSync(unsentFile(), 'utf8'));
    fs.rmSync(unsentFile(), { force: true });
  } catch (_) {
    return;
  }
  openUnsent(Array.isArray(list) ? list : []);
}

// The server refused an email that waited. Its editor closed after Send, so it opens again with the email and the
// error: in the main window's reading pane when it was written there, otherwise in a compose window. The main
// window comes forward from the tray for it. During a quit nobody would see that, so the email goes to Drafts, and
// when that fails too, to the next start.
async function waitedMailFailed(entry, err) {
  const error = (err && err.message) || String(err);
  let restore = entry.restore || {};
  if (quitting) {
    let saved = null;
    try {
      saved = { id: await api.saveDraft(entry.payload) };
    } catch (_) {
      // Kept below for the next start, or opened now when the quit was called off meanwhile.
    }
    if (quitting) {
      if (!saved) unsent.push({ restore: entry.restore, error: t('composer.undoSend.quitFailed', { error }) });
      return;
    }
    // The quit was called off while the email went to Drafts, and answered() has already opened what the quit kept.
    // Rukoo stays, so this one opens like any refused email; when the save stored a draft, its editor goes on with it,
    // attachments included. The restore travels as it is from here: to the main window, a compose window, or
    // unsent.json when a later quit comes first.
    if (saved) restore = savedToDrafts(restore, saved.id);
  }
  if (entry.origin === 'main' && win && !win.isDestroyed()) {
    if (!win.isVisible() || win.isMinimized()) showWindow();
    // Held until the window has opened it; the failed email's notice goes with it (the renderer's takeSendFailures).
    holdRefused({ id: entry.id, restore, error });
    return;
  }
  openComposeWindow({ ...(restore.opts || {}), restore, sendError: error });
}
// ---- /undo send ----

// Every renderer call goes through this table; nothing else is reachable from the UI.
const api = {
  // The booking page's address while it is on, and its language, for the footer and the composer (contracts f).
  state: () => ({ ...engine.state(), bookingUrl: booking ? booking.url() : null, bookingLocale: booking ? booking.locale() : null }),
  list: (opts) => engine.listMessages(opts),
  counts: (scope) => engine.counts(scope),
  // Mail older than the cache keeps: what Load older and the server search can reach from the list on screen, Load
  // older itself, and the server search under a search's results. opts as for list.
  olderState: (opts) => engine.olderState(olderOpts(opts)),
  loadOlder: (opts) => engine.loadOlder(olderOpts(opts)),
  searchServer: (opts) => engine.searchServer(olderOpts(opts)),
  contacts: () => engine.contacts(),
  threadContacts: (accountId, ids) => engine.threadContacts(accountText(accountId, 200), Array.isArray(ids) ? ids.slice(0, 200).map((id) => accountText(id, 1000)) : []),
  // fast: the reader's, which shows a large message before its attachments are in; downloaded says when they are.
  get: (id, opts) => engine.getMessage(id, { fast: Boolean(opts && opts.fast === true) }),
  downloaded: (id, opts) => engine.downloaded(id, { now: Boolean(opts && opts.now === true) }),
  // A long thread's References can name many emails; the newest are at the end.
  related: (id, references) => engine.related(accountText(id, 2000), Array.isArray(references) ? references.slice(-200).map((r) => accountText(r, 1000)) : []),
  setFlags: (id, flags) => engine.setFlags(id, flags),
  archive: (id) => engine.archive(id),
  canUndo: (id) => engine.canUndo(id),
  undoMove: (id) => engine.undoMove(id),
  setFlagsMany: (ids, flags) => engine.setFlagsMany((Array.isArray(ids) ? ids : []).slice(0, 100000).map((id) => accountText(id, 4000)), plainObject(flags)),
  move: (id, dest) => engine.move(id, dest),
  remove: (id, opts) => (String(id).startsWith('saved:') ? engine.deleteSaved(id) : engine.remove(id, opts)),
  emptyFolder: (accountId, folder) => engine.emptyFolder(accountId, folder),
  toggleVip: (address) => engine.toggleVip(address),
  markSpam: (id) => engine.markSpam(id),
  removeSpam: (address) => engine.updateSettings({ spam: engine.settings.spam.filter((a) => a !== address) }),
  saveToDevice: (id) => engine.saveToDevice(id),
  openCompose: (opts) => openComposeWindow(opts),
  // A draft already open in a compose window is brought forward there instead of opening twice.
  draftWindow: (id) => {
    const open = [...composeWindows.values()].find((c) => c.draftId && c.draftId === id && !c.win.isDestroyed());
    if (!open) return false;
    open.win.show();
    open.win.focus();
    return true;
  },
  toastMain: (message) => send('toast', String(message || '').slice(0, 200)),
  // Settings > General: whether this system offers Rukoo notifications, and a test notification that shows also while
  // Rukoo is in front, with what came of it (desktop.js testNotice).
  notificationState: () => ({ supported: Notification.isSupported() }),
  testNotification: () =>
    testNotice(
      Notification,
      { title: t('native.notifications.testTitle'), body: t('native.notifications.testBody'), icon: nativeImage.createFromPath(ICON), silent: false },
      { keep: liveNotices, onClick: showWindow }
    ),
  // A save replaces the stored draft; an agent's draft cards follow it, also from a compose window.
  saveDraft: async (payload) => {
    const id = await engine.saveDraft(payload);
    if (hub && payload && payload.draftId && id) hub.draftMoved(String(payload.draftId), id);
    return id;
  },
  discardDraft: async (id) => {
    const result = await engine.discardDraft(id);
    if (hub && id) hub.draftMoved(String(id), null);
    return result;
  },
  addAccount: (input) => engine.addAccount(input),
  updateAccount: (id, patch) => engine.updateAccount(id, patch),
  // The signature editor in Settings shows pasted HTML the way it will be stored.
  cleanSignature: (html) => cleanSignatureHtml(typeof html === 'string' ? html : ''),
  removeAccount: (id) => engine.removeAccount(id),
  updateSettings: (patch) => {
    const s = engine.updateSettings(patch);
    if ('theme' in patch) applyTheme();
    if ('language' in patch) {
      broadcast('language');
      if (tray) tray.relabel();
    }
    if ('syncInterval' in patch) scheduleSync();
    if ('closeToTray' in patch || 'minimizeToTray' in patch) updateTray();
    if ('badge' in patch) refreshBadge();
    // The other channel's newest release, if any; switched off, the next Check now looks.
    if ('updateChannel' in patch && updates) {
      updates.channelChanged();
      broadcast('updates', updatesState());
      updates.check().catch(() => {});
    }
    // Turned off during a meeting, the windows show in capture again at once; turned on, they hide.
    if ('meetingHideWindow' in patch && voice) {
      voice.refreshProtection();
      send('meeting', { kind: 'state', state: voiceState() });
    }
    return s;
  },
  // Settings → General resets it; the keys and the wheel go through watchZoom.
  setZoom: (level) => setZoom(level),
  sync: (accountId) => engine.syncNow(accountId).then(() => true),
  openFolder: (accountId, folder) => engine.openFolder(accountId, folder),
  pickFiles: async () => {
    const res = await dialog.showOpenDialog(win, { properties: ['openFile', 'multiSelections'], title: t('composer.attachments.pick') });
    if (res.canceled) return [];
    return res.filePaths.map((p) => ({ path: p, filename: path.basename(p), size: fs.statSync(p).size }));
  },
  pickImage: async () => {
    const res = await dialog.showOpenDialog(win, {
      properties: ['openFile'],
      title: t('composer.attachments.image'),
      filters: [{ name: t('native.attachments.images'), extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }]
    });
    if (res.canceled || !res.filePaths[0]) return null;
    const file = res.filePaths[0];
    const ext = path.extname(file).slice(1).toLowerCase().replace('jpg', 'jpeg');
    return `data:image/${ext};base64,${fs.readFileSync(file).toString('base64')}`;
  },
  openAttachment: async (id, index) => {
    const a = await engine.attachment(id, index);
    // An .ics file opens as an invitation card in Rukoo itself; one that holds no event goes to the system's app.
    if (isIcs(a)) {
      try {
        const acc = engine.messageAccount(id);
        showInviteFile(invitations.openText(a.content.toString('utf8'), safeName(a.filename), acc ? acc.id : null));
        return true;
      } catch (_) {
        // Not an event Rukoo can show.
      }
    }
    if (RISKY.test(safeName(a.filename))) {
      const res = await dialog.showMessageBox(win, {
        type: 'warning',
        title: t('native.attachments.open'),
        message: t('native.attachments.riskyTitle', { filename: safeName(a.filename) }),
        detail: t('native.attachments.riskyHelp'),
        buttons: [t('common.actions.cancel'), t('native.attachments.save')],
        defaultId: 0,
        cancelId: 0
      });
      if (res.response === 1) return api.saveAttachment(id, index);
      return false;
    }
    const file = await tempAttachment(id, index);
    const err = await shell.openPath(file);
    if (err) throw new Error(err);
    return true;
  },
  saveAttachment: async (id, index) => {
    const a = await engine.attachment(id, index);
    const res = await dialog.showSaveDialog(win, { defaultPath: path.join(app.getPath('downloads'), safeName(a.filename)) });
    if (res.canceled || !res.filePath) return false;
    fs.writeFileSync(res.filePath, a.content);
    markOfTheWeb(res.filePath);
    return res.filePath;
  },
  saveAllAttachments: (id) => saveAllAttachments(id),
  // A thumbnail for image attachments; other types (and SVG, which can carry script) get none.
  attachmentPreview: async (id, index) => {
    const a = await engine.attachment(id, index);
    const type = String(a.contentType || '').toLowerCase();
    if (!/^image\/(png|jpe?g|gif|webp)$/.test(type) || a.content.length > 8 * 1024 * 1024) return null;
    return `data:${type};base64,${a.content.toString('base64')}`;
  },
  unsubscribe: async (id) => {
    const m = await engine.getMessage(id);
    const u = m.unsubscribe;
    if (!u) throw new Error(t('errors.unsubscribe.missing'));
    const acc = engine.messageAccount(id);
    if (acc && acc.type === 'demo') return { done: true };
    // Only a 2xx from the endpoint itself counts; a redirect or failure falls back to the page in the browser.
    if (u.oneClick && (await oneClickUnsubscribe(u.url))) return { done: true };
    if (u.url) {
      shell.openExternal(u.url);
      return { opened: true };
    }
    // The link for agents, which open it themselves; its fields for the reader's composer.
    return { mailto: u.mail, message: mailto.parse(u.mail) };
  },
  createFolder: (accountId, name) => engine.createFolder(accountId, name),
  senderLogos: (addresses) => {
    if (engine.settings.senderLogos === false) return {};
    const bySite = {};
    for (const a of (Array.isArray(addresses) ? addresses : []).slice(0, 300)) {
      const site = siteOf(a);
      if (site) bySite[a] = site;
    }
    return logos.get(Object.values(bySite)).then((found) => {
      const out = {};
      for (const [address, site] of Object.entries(bySite)) out[address] = found[site] || null;
      return out;
    });
  },
  exportEml: async (id) => {
    const full = await engine.getMessage(id);
    const name = `${(full.subject || t('native.export.defaultName')).replace(/[<>:"/\\|?*]/g, '_').slice(0, 80)}.eml`;
    const res = await dialog.showSaveDialog(win, { defaultPath: path.join(app.getPath('documents'), name) });
    if (res.canceled || !res.filePath) return false;
    fs.writeFileSync(res.filePath, await engine.rawSource(id));
    return res.filePath;
  },
  openExternal: (url) => openUrl(url),
  updatesState: () => updatesState(),
  updatesCheck: async () => {
    await updates.check({ force: true });
    return updatesState();
  },
  // Restart to update: an ordinary quit, which installs at its end (see will-quit) and starts the new version. A
  // window that calls the quit off calls the restart off too.
  updatesInstall: () => {
    if (!installer || installer.status !== 'ready' || !updates.available || installer.version !== updates.available.version) return false;
    restartForUpdate = true;
    app.quit();
    return true;
  },
  // The context menu in an email. Copy image: the image as a data: URL, which the reader turns into a PNG for
  // copyImage, since PNG is the one image type the clipboard takes. Open image: a copy in the default image viewer.
  copyText: async (text) => {
    if (typeof text !== 'string' || !text || text.length > 100000) throw new Error('invalid');
    await clipboard.writeText(text);
    return true;
  },
  imageData: async (src) => {
    const img = await emailImage(src);
    return `data:${img.type};base64,${img.content.toString('base64')}`;
  },
  copyImage: async (png) => {
    const image = /^data:image\/png;base64,/.test(String(png || '')) ? nativeImage.createFromDataURL(png) : null;
    if (!image || image.isEmpty()) throw new Error(t('errors.image.unsupported'));
    await clipboard.write([new ClipboardItem({ 'image/png': new Blob([image.toPNG()], { type: 'image/png' }) })]);
    return true;
  },
  openImage: async (src) => {
    const img = await emailImage(src);
    const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'rukoo-mail-')), img.name);
    fs.writeFileSync(file, img.content);
    markOfTheWeb(file);
    const err = await shell.openPath(file);
    if (err) throw new Error(err);
    return true;
  },
  // Only the release page that was found opens; the renderer can't pass another address.
  updatesOpen: () => {
    const found = updates.available;
    if (found && /^https:\/\/github\.com\//.test(found.url)) shell.openExternal(found.url);
  },
  // The "What's new" guide shows once per version. Hidden test windows skip it unless they ask (SEM_WHATS_NEW=1);
  // visible ones, such as the macOS tests, say SEM_WHATS_NEW=0.
  whatsNewState: () => ({
    version: WHATS_NEW,
    show:
      engine.settings.whatsNewSeen !== WHATS_NEW &&
      process.env.SEM_WHATS_NEW !== '0' &&
      (!process.env.SEM_HIDDEN || process.env.SEM_WHATS_NEW === '1')
  }),
  whatsNewDone: () => {
    engine.updateSettings({ whatsNewSeen: WHATS_NEW }, { quiet: true });
    return true;
  },
  // Which app opens mailto: links now, for Settings. available: false where Rukoo can't offer itself.
  mailtoStatus: async () => {
    if (process.platform === 'darwin') return new MacMailClient(app, Boolean(process.env.SEM_DATA_DIR)).status();
    if (process.platform === 'linux') {
      const client = new LinuxMailClient();
      const status = await client.status();
      return { ...status, available: status.available && app.isPackaged && !process.env.SEM_DATA_DIR, linux: true };
    }
    const client = mailClient();
    const own = client ? path.resolve(client.exe).toLowerCase() : null;
    let handler = null;
    try {
      handler = await app.getApplicationInfoForProtocol('mailto:');
    } catch (_) {
      // No app opens them.
    }
    return {
      available: Boolean(client),
      app: handler && handler.name ? String(handler.name) : null,
      rukoo: Boolean(own && handler && handler.path && path.resolve(handler.path).toLowerCase() === own)
    };
  },
  // Offers Rukoo for mailto: links and opens the Windows page where the user picks it.
  useForMailto: async () => {
    if (process.platform === 'darwin') {
      const client = new MacMailClient(app, Boolean(process.env.SEM_DATA_DIR));
      if (!client.available) throw new Error(t('settings.general.mailtoMacUnavailable'));
      if (!client.register()) throw new Error(t('settings.general.mailtoMacFailed'));
      return true;
    }
    if (process.platform === 'linux') {
      if (!app.isPackaged || process.env.SEM_DATA_DIR) throw new Error(t('settings.general.mailtoLinuxUnavailable'));
      if (!await new LinuxMailClient().register()) throw new Error(t('settings.general.mailtoLinuxFailed'));
      return true;
    }
    const client = mailClient();
    if (!client) throw new Error(t('settings.general.mailtoUnavailable'));
    await client.register();
    await shell.openExternal(SETTINGS_URL);
    return true;
  },
  print: (html) => printHtml(html),
  setBadge: (dataUrl, count) => {
    if (!win || process.platform !== 'win32') return;
    if (!dataUrl || !count) win.setOverlayIcon(null, '');
    else win.setOverlayIcon(nativeImage.createFromDataURL(dataUrl), t('native.notifications.newCount', { count }));
  },
  googleSignIn: async () => {
    const grant = await google.signIn();
    showWindow();
    return engine.addGoogleAccount(grant);
  },
  googleReauth: async (accountId) => {
    const grant = await google.signIn(engine.account(accountId).email);
    showWindow();
    const acc = await engine.addGoogleAccount(grant, { reauthId: accountId });
    calendars.reconnected(accountId);
    return acc;
  },
  calendarView: (range) => calendars.view(calendarRange(range)),
  calendarSync: (accountId) => (accountId ? calendars.syncAccount(String(accountId)) : calendars.syncAll()).then(() => true),
  calendarEvent: (id) => calendars.event(calendarEventId(id)),
  // A booking id or uid never comes from a window: only the booking service makes booking events.
  calendarCreate: (input, opts) => calendars.create(calendarInput(input), { notify: plainObject(opts).notify === true }),
  calendarUpdate: (id, input, opts) =>
    calendars.update(calendarEventId(id), calendarInput(input), { notify: plainObject(opts).notify === true, etag: calendarEtag(opts), scope: calendarScope(opts), seriesEtag: calendarSeriesEtag(opts) }),
  calendarDelete: (id, opts) =>
    calendars.remove(calendarEventId(id), {
      notify: plainObject(opts).notify === true,
      series: plainObject(opts).series === true,
      scope: plainObject(opts).scope === undefined ? undefined : calendarScope(opts),
      etag: calendarEtag(opts),
      seriesEtag: calendarSeriesEtag(opts)
    }),
  // An invitation's card in the reader or for an .ics file. source: { id } of an email, or { file }: an opened file's
  // token. The answer goes through the calendar that has the event, or else by email (calendar/invitations.js).
  inviteFiles: () => inviteFiles.splice(0),
  inviteState: (source) => invitations.state(inviteSource(source)),
  inviteAnswer: (source, response) => invitations.answer(inviteSource(source), String(response)),
  inviteAdd: (source, target) => {
    const to = plainObject(target);
    return invitations.add(inviteSource(source), { accountId: accountText(to.accountId, 200), calendarId: calendarCalendarId(to.calendarId) });
  },
  calendarRespond: (id, response, opts) => calendars.respond(calendarEventId(id), String(response), { series: plainObject(opts).series === true }),
  calendarSeries: (id) => calendars.series(calendarEventId(id)),
  calendarMove: (id, calendarId, opts) => calendars.move(calendarEventId(id), calendarCalendarId(calendarId), { notify: plainObject(opts).notify === true }),
  calendarSearch: (query) => calendars.search(typeof query === 'string' && query.length <= 200 ? query : ''),
  calendarFree: (opts) => freeTime(calendars, engine.settings, booking ? booking.hours() : null, calendarFreeOpts(opts)),
  // Settings pages and the booking setup may open before the calendar view ever did: accounts whose calendars were
  // never read get them first, without the view's fetch.
  calendarList: () => calendars.loadCalendars().then(() => calendars.list()),
  calendarConflicts: (opts) => calendars.conflicts(calendarConflicts(opts)),
  calendarSettings: (patch) => {
    const settings = engine.calendarSettings(plainObject(patch));
    // Colours and shown calendars are drawn by the calendar view.
    calendars.emit('updated');
    return settings;
  },
  googleCancel: () => google.cancel(),
  fetchGmailAliases: (accountId) => engine.fetchGmailAliases(accountId),
  googleImportClient: async () => {
    const res = await dialog.showOpenDialog(win, {
      title: t('setup.google.import'),
      properties: ['openFile'],
      filters: [{ name: 'JSON', extensions: ['json'] }]
    });
    if (res.canceled || !res.filePaths[0]) return false;
    google.importClient(res.filePaths[0]);
    engine.emit('updated');
    return true;
  },
  appInfo: () => ({ version: app.getVersion(), dataDir: app.getPath('userData') }),
  // The Rukoo account (account/index.js). Passwords stay in main: only their keys leave, and only to the server.
  accountStatus: () => account.status(),
  accountServerInfo: () => account.serverInfo(),
  accountSetServer: (url) => account.setServer(accountText(url, 2000)),
  accountSignUp: (input) => {
    const i = plainObject(input);
    return account.signUp({ email: accountText(i.email, 320), password: accountText(i.password, 1000), invite: accountText(i.invite, 200) });
  },
  accountSignIn: (input) => {
    const i = plainObject(input);
    return account.signIn({ email: accountText(i.email, 320), password: accountText(i.password, 1000) });
  },
  accountRecover: (input) => {
    const i = plainObject(input);
    return account.recover({ email: accountText(i.email, 320), recoveryKey: accountText(i.recoveryKey, 200), password: accountText(i.password, 1000) });
  },
  accountSignOut: () => account.signOut(),
  accountDetails: () => account.details(),
  accountRemoveDevice: (id) => account.removeDevice(accountText(id, 200)),
  accountChangePassword: (current, next) => account.changePassword(accountText(current, 1000), accountText(next, 1000)),
  accountNewRecoveryKey: (password) => account.newRecoveryKey(accountText(password, 1000)),
  accountDelete: (password) => account.deleteAccount(accountText(password, 1000)),
  accountSync: () => account.sync(),
  // Inbox categories (categories/index.js).
  categoriesState: () => categories.state(),
  // Prototype for the "What's new" guide.
  categoriesPreview: () => categories.preview(),
  categoriesSeed: () => {
    categories.seed();
    return categories.state();
  },
  categoriesSetEnabled: (on) => categories.setEnabled(on === true),
  categoriesSetUnsure: (id) => categories.setUnsure(accountText(id, 100)),
  categoriesSave: (input) => {
    const i = plainObject(input);
    const optional = (v, max) => (v === null ? null : typeof v === 'string' ? v.slice(0, max) : undefined);
    return categories.saveCategory({
      id: accountText(i.id, 100) || null,
      name: optional(i.name, 100),
      description: optional(i.description, 1000),
      color: accountText(i.color, 20) || undefined,
      inbox: typeof i.inbox === 'boolean' ? i.inbox : undefined
    });
  },
  categoriesRemove: (id) => categories.removeCategory(accountText(id, 100)),
  // Notifications for a category's new mail, on this computer only.
  categoriesSetNotify: (id, on) => categories.setNotify(accountText(id, 100), on === true),
  categoriesReorder: (ids) => categories.reorder((Array.isArray(ids) ? ids : []).slice(0, 50).map((id) => accountText(id, 100))),
  categoriesMove: (ids, category, opts) =>
    // Every selected email: a unified inbox can show several hundred per account.
    categories.move((Array.isArray(ids) ? ids : []).slice(0, 100000).map((id) => accountText(id, 4000)), accountText(category, 100), { sender: plainObject(opts).sender === true }),
  categoriesForgetSender: (address) => categories.forgetSender(accountText(address, 320)),
  categoriesSetJev: (input) => {
    const i = plainObject(input);
    return categories.setJev({ on: typeof i.on === 'boolean' ? i.on : undefined, key: i.key === null ? null : accountText(i.key, 500) || undefined });
  },
  categoriesSortAgain: () => categories.sortAgain(),
  // The booking page (booking/index.js), which checks every field again.
  bookingState: (opts) => booking.load({ refresh: plainObject(opts).refresh === true }),
  bookingEnable: (input) => booking.enable(plainObject(input)),
  bookingSave: (patch) => booking.save(plainObject(patch)),
  bookingDisable: () => booking.disable(),
  bookingCheckHandle: (handle) => booking.checkHandle(accountText(handle, 64)),
  // accept, decline and cancel name a booking; keep and dismiss a notice.
  bookingAction: (id, action, opts) => {
    const kind = accountText(action, 20);
    const pattern = kind === 'keep' || kind === 'dismiss' ? /^[A-Za-z0-9_:.@-]{1,120}$/ : /^[A-Za-z0-9_-]{22}$/;
    const o = plainObject(opts);
    const options = { message: typeof o.message === 'string' ? o.message.slice(0, 2000) : undefined, email: o.email === true, notify: o.notify !== false };
    return booking.action(typeof id === 'string' && pattern.test(id) ? id : '', kind, options);
  },
  bookingDetails: (id) => booking.details(typeof id === 'string' && /^[A-Za-z0-9_-]{22}$/.test(id) ? id : ''),
  bookingSync: () => booking.sync(),
  bookingTakeover: () => booking.takeover(),
  // Video services for bookings (video/index.js), connected on this computer only.
  videoState: () => video.state(),
  videoConnect: async (id) => {
    const state = await video.connect(accountText(id, 20));
    showWindow();
    return state;
  },
  videoCancel: () => video.cancel(),
  videoDisconnect: (id) => video.disconnect(accountText(id, 20)),
  videoRoom: (id, input) => {
    const i = plainObject(input);
    return video.setRoom(accountText(id, 20), { on: typeof i.on === 'boolean' ? i.on : undefined, server: typeof i.server === 'string' ? i.server.slice(0, 400) : undefined });
  },
  videoImportClient: async (id) => {
    const service = accountText(id, 20);
    const res = await dialog.showOpenDialog(win, {
      title: t('settings.video.importTitle', { service: video.name(service) }),
      properties: ['openFile'],
      filters: [{ name: 'JSON', extensions: ['json'] }]
    });
    if (res.canceled || !res.filePaths[0]) return video.state();
    return video.importClient(service, res.filePaths[0]);
  },
  categoriesActivity: () => categories.activity(),
  accountExport: async () => {
    const res = await dialog.showSaveDialog(win, {
      title: t('settings.rukoo.export'),
      defaultPath: path.join(app.getPath('documents'), 'rukoo-account.json'),
      filters: [{ name: 'JSON', extensions: ['json'] }]
    });
    if (res.canceled || !res.filePath) return false;
    fs.writeFileSync(res.filePath, JSON.stringify(account.exportData(), null, 2));
    return res.filePath;
  }
};

// ---- agents ----
// The chat panel's calls. Arguments are checked here; the hub checks structure and values again.
const agentText = (v, max) => (typeof v === 'string' ? v.slice(0, max) : '');
const agentId = (v) => {
  if (typeof v !== 'string' || !v || v.length > 200) throw new Error('invalid');
  return v;
};
// A mail id carries its folder path URI-encoded, so it can be far longer than a chat or item id.
const agentMailId = (v) => {
  if (typeof v !== 'string' || !v || v.length > 4000) throw new Error('invalid');
  return v;
};
const agentPlain = (v) => (v && typeof v === 'object' && !Array.isArray(v) ? v : {});
// A list from the renderer, bounded; the hub checks how many it takes.
const agentList = (v) => (Array.isArray(v) ? v.slice(0, 50) : []);
function agentHub() {
  if (!hub) throw new Error('unknown');
  return hub;
}
function agentMeeting(m) {
  if (!m || typeof m !== 'object' || !isMeetingId(m.id)) return null;
  return { id: m.id, title: agentText(m.title, 300), eventId: agentText(m.eventId, 4000) || null };
}
function agentMessage(m) {
  if (!m || typeof m !== 'object' || typeof m.id !== 'string' || !m.id) return null;
  const from = m.from && typeof m.from === 'object' ? { name: agentText(m.from.name, 200), address: agentText(m.from.address, 320) } : null;
  return {
    id: m.id.slice(0, 2000),
    messageId: agentText(m.messageId, 1000) || null,
    accountId: agentText(m.accountId, 200) || null,
    subject: agentText(m.subject, 1000),
    from,
    date: Number(m.date) || null,
    unsubscribe: m.unsubscribe === true
  };
}
Object.assign(api, {
  agentConfig: () => agentHub().config(),
  agentUpdateConfig: (patch) => agentHub().updateConfig(agentPlain(patch)),
  agentSetSecret: (agent, value) => agentHub().setSecret(agentId(agent), agentText(value, 4000)),
  agentTest: (agent) => agentHub().test(agentId(agent)),
  agentStatus: () => agentHub().status(),
  agentCopyHermesSetup: () => agentHub().copyHermesSetup(),
  agentCopyOpenClawSetup: () => agentHub().copyOpenClawSetup(),
  agentAddCustom: (preset) => agentHub().addCustomAgent(agentText(preset, 20)),
  agentRemoveCustom: (id) => agentHub().removeCustomAgent(agentId(id)),
  agentCopyCustomSetup: (id) => agentHub().copyCustomSetup(agentId(id)),
  // A phone that runs Claude Code and Codex chats here (agents/phonehost.js).
  agentPhonePair: (opts) => agentHub().phonePairStart({ allowExport: agentPlain(opts).allowExport === true }),
  agentPhonePairCancel: () => agentHub().phonePairCancel(),
  agentPhoneUnpair: (id) => agentHub().phoneUnpair(agentId(id)),
  agentPhoneAllowExport: (id) => agentHub().phoneAllowExport(agentId(id)),
  agentList: () => agentHub().list(),
  agentGet: (id) => agentHub().forPanel(agentId(id)),
  agentFindFor: (ref) => {
    const r = agentPlain(ref);
    return agentHub().findFor({ id: agentText(r.id, 2000), messageId: agentText(r.messageId, 1000), accountId: agentText(r.accountId, 200) || null });
  },
  agentRelatedFor: (ref) => {
    const r = agentPlain(ref);
    // A long thread's References can name many emails; the newest are at the end.
    const references = (Array.isArray(r.references) ? r.references : []).slice(-100).map((v) => agentText(v, 1000)).filter(Boolean);
    return agentHub().relatedFor({
      id: agentText(r.id, 2000),
      messageId: agentText(r.messageId, 1000),
      accountId: agentText(r.accountId, 200) || null,
      inReplyTo: agentText(r.inReplyTo, 1000) || null,
      references,
      date: Number(r.date) || null
    });
  },
  agentContinue: (id, message) => agentHub().continueFrom(agentId(id), agentMessage(message)),
  agentLocate: (ref) => {
    const r = agentPlain(ref);
    return agentHub().locate({ id: agentText(r.id, 2000), messageHeader: agentText(r.messageHeader, 1000), accountId: agentText(r.accountId, 200) || null });
  },
  agentCreate: (input) => {
    const i = agentPlain(input);
    return agentHub().create({
      agent: agentText(i.agent, 20),
      message: agentMessage(i.message),
      // A chat about a Rukoo Voice meeting instead of an email.
      meeting: agentMeeting(i.meeting),
      model: agentText(i.model, 300) || null,
      effort: agentText(i.effort, 20) || null,
      permission: agentText(i.permission, 20) || null
    });
  },
  // The newest chat about this meeting, or null.
  agentFindForMeeting: (id) => (isMeetingId(id) ? agentHub().findForMeeting(id) : null),
  agentModels: (agent, options) => agentHub().models(agentId(agent), { refresh: agentPlain(options).refresh === true }),
  // A chat's model, effort and permission level; a key that is left out stays as it is, null goes back to the agent's
  // default or, for the level, to the one in Settings.
  agentSetChoice: (id, choice) => {
    const c = agentPlain(choice);
    const pick = {};
    if ('model' in c) pick.model = agentText(c.model, 300) || null;
    if ('effort' in c) pick.effort = agentText(c.effort, 20) || null;
    if ('permission' in c) pick.permission = agentText(c.permission, 20) || null;
    return agentHub().setChoice(agentId(id), pick);
  },
  agentSend: (id, input) => {
    const i = agentPlain(input);
    return agentHub().send(agentId(id), {
      text: agentText(i.text, 20000),
      action: agentText(i.action, 40) || null,
      display: agentText(i.display, 20000) || null,
      skill: agentText(i.skill, 64) || null,
      // Tokens of staged files (agentPickFiles, agentAddFile) and Rukoo ids of emails.
      files: agentList(i.files).map(agentId),
      emails: agentList(i.emails).map(agentMailId),
      // While the agent works: 'steer' hands the message over before its next step, 'queue' sends it after the turn.
      mode: i.mode === 'steer' || i.mode === 'queue' ? i.mode : null
    });
  },
  // A message that waits in the chat: send it now, or take it out (Remove, or Edit with restage for the input).
  agentSendQueued: (id, entryId) => agentHub().sendQueued(agentId(id), agentId(entryId)),
  agentUnqueue: (id, entryId, options) => agentHub().unqueue(agentId(id), agentId(entryId), { restage: agentPlain(options).restage === true }),
  // The chat's paperclip opens main's own dialog, so Rukoo reads only what the user picked there. room: how many
  // more files the message takes (chatfiles.stagePaths). Read one by one, in the order picked, without blocking: a
  // file on a slow or network drive must not hold up the windows or any other request.
  agentPickFiles: async (room) => {
    const res = await dialog.showOpenDialog(win, { properties: ['openFile', 'multiSelections'], title: t('agent.panel.attach') });
    if (res.canceled) return [];
    return agentHub().stagePaths(res.filePaths, room);
  },
  // A file dropped on the chat or pasted into it. The sandboxed renderer sends its bytes, never a path to open.
  agentAddFile: (input) => {
    const i = agentPlain(input);
    if (!(i.data instanceof Uint8Array)) throw new Error('invalid');
    const name = agentText(i.name, 1000);
    const refused = agentHub().refuseFile(name, i.data.byteLength);
    return refused ? { name, error: refused } : agentHub().stageFile(name, Buffer.from(i.data));
  },
  agentDropFile: (id) => agentHub().unstageFile(agentId(id)),
  agentSkills: () => agentHub().skillList(),
  // The skills of the agent itself, for the $ menu (Rukoo's own are agentSkills).
  agentOwnSkills: (agent, options) => agentHub().ownSkills(agentId(agent), { refresh: agentPlain(options).refresh === true }),
  agentStop: (id) => agentHub().stop(agentId(id)),
  agentDecide: (id, itemId, choiceId) => agentHub().decide(agentId(id), agentId(itemId), agentId(choiceId)),
  agentRemove: (id) => agentHub().remove(agentId(id)),
  agentView: (state) => agentHub().view(agentPlain(state)),
  // The chat the open panel shows, or null; showing a chat reads its answer.
  agentWatch: (id) => agentHub().watch(id == null ? null : agentId(id)),
  agentUiReply: (requestId, ok, result) => agentHub().uiReply(agentId(requestId), ok === true, result === undefined ? null : result),
  agentPanelReady: () => {
    if (agentGate) agentGate.open();
  },
  agentUndo: (id, itemId) => agentHub().undo(agentId(id), agentId(itemId)),
  agentKeepDraft: (composerKey, draftId, was) =>
    agentHub().keepDraft(agentId(composerKey), draftId == null ? null : agentMailId(draftId), was == null ? null : agentMailId(was)),
  agentPatchItem: (id, itemId, patch) => {
    const p = agentPlain(patch);
    if (Object.keys(p).length !== 1 || typeof p.undone !== 'boolean') throw new Error('invalid');
    return agentHub().patchItem(agentId(id), agentId(itemId), { undone: p.undone });
  },
  // Links in agent cards (sources, plan tasks). Besides what any link may open, an Obsidian note can open
  // here; mail frames keep using openExternal, so a link in an email never starts an Obsidian action.
  openAgentLink: (url) => {
    const u = agentText(url, 2000).trim();
    if (/^obsidian:\/\/(open|search)\b/i.test(u)) return shell.openExternal(u);
    return openUrl(u);
  }
});
// ---- /agents ----

// ---- voice ----
// The meeting view's and Settings' calls (voice/index.js). A meeting id has one fixed form, so anything else stops
// here; text is cut to what the service keeps.
const meetingId = (v) => {
  if (!isMeetingId(v)) throw new Error('invalid');
  return v;
};
const meetingText = (v, max) => (typeof v === 'string' ? v.slice(0, max) : '');
function voiceService() {
  if (!voice) throw new Error('unknown');
  return voice;
}
// A frame's image as the renderer's CSP takes it (img-src data:), or null for anything but a plain image.
function frameUrl(result) {
  const image = result && result.image;
  if (!image || !/^image\/(jpeg|png|webp)$/.test(image.type) || typeof image.data !== 'string' || !/^[A-Za-z0-9+/]+=*$/.test(image.data)) return null;
  return `data:${image.type};base64,${image.data}`;
}
Object.assign(api, {
  voiceState: () => voiceState(),
  // Newest first.
  meetingList: () => voiceService().list(),
  meetingGet: (id) => voiceService().get(meetingId(id)),
  // Hides the windows first when the user wants that, then asks Voice: {meetingId}.
  meetingStart: (opts) => {
    const o = plainObject(opts);
    return voiceService().start({
      title: meetingText(o.title, 300),
      eventId: meetingText(o.eventId, 4000) || null,
      screen: MEETING_SCREENS.includes(o.screen) ? o.screen : undefined
    });
  },
  meetingStop: (id) => voiceService().stop(meetingId(id)),
  meetingRename: (id, title) => voiceService().rename(meetingId(id), title),
  meetingDelete: (id) => voiceService().delete(meetingId(id)),
  meetingPause: (id) => voiceService().pause(meetingId(id)),
  meetingResume: (id) => voiceService().resume(meetingId(id)),
  meetingMark: (id, label) => voiceService().mark(meetingId(id), meetingText(label, 200) || null).then(() => true),
  // The calendar event the meeting was, or null; the speakers come back with the guests to pick from.
  meetingSetEvent: (id, eventId) => voiceService().setEvent(meetingId(id), eventId == null ? null : meetingText(eventId, 4000) || null),
  meetingSetSpeaker: (id, speakerId, value) => {
    const v = value === null ? null : plainObject(value);
    return voiceService().setSpeaker(meetingId(id), meetingText(speakerId, 100), v && { name: meetingText(v.name, 200), address: meetingText(v.address, 320) });
  },
  // Notes reject oversized content without clipping it. rev is the editor's starting revision.
  meetingSaveNotes: (id, text, rev) => {
    if (!Number.isInteger(rev) || rev < 0) throw new Error('invalid');
    return voiceService().saveNotes(meetingId(id), typeof text === 'string' ? text : '', rev);
  },
  // null goes back to the text Voice recognized.
  meetingCorrect: (id, passageId, text) => voiceService().correct(meetingId(id), meetingText(passageId, 100), text === null ? null : meetingText(text, 20000)),
  // The image of the frame at or before atMs (meeting time in ms, a frame's at), as a data URL, or null. A frame the
  // meeting view got without its text gets the text with the delta that follows.
  meetingFrame: async (id, atMs) => {
    if (!Number.isInteger(atMs) || atMs < 0) throw new Error('invalid');
    return frameUrl(await voiceService().frame(meetingId(id), atMs));
  },
  // The meeting's screen recording and the monitors it can move to (voice/index.js screenInfo).
  meetingScreen: (id) => voiceService().screenInfo(meetingId(id)),
  meetingScreenSet: (id, screen, monitor = null) => voiceService().setScreen(meetingId(id), screen, typeof monitor === 'string' ? monitor : null),
  // monitor: an id from meetingScreen's monitors. The id of the monitor Voice records now.
  meetingScreenSelect: (id, monitor) => {
    if (typeof monitor !== 'string' || !monitor || Array.from(monitor).length > 64) throw new Error('invalid');
    return voiceService().selectMonitor(meetingId(id), monitor);
  },
  meetingEvents: (id) => voiceService().events(meetingId(id))
});
// ---- /voice ----

// Calls that act on the window that makes them.
const windowApi = {
  // copy: the reader's context menu, for the selection in the email frame it has just focused. cut, paste and
  // pasteAndMatchStyle (paste as plain text): the composer's context menu, for its text (editmenu.js).
  nativeEdit: (sender, action) => {
    if (['undo', 'redo', 'selectAll', 'copy', 'cut', 'paste', 'pasteAndMatchStyle'].includes(action)) sender[action]();
  },
  // The spelling part of the composer's context menu: one of the suggestions forwardEditMenu sent for the word that
  // was right-clicked, or that word into the dictionary. Either one is used once.
  replaceMisspelling: (sender, word) => {
    const found = spelling.get(sender);
    if (!found || typeof word !== 'string' || !found.suggestions.includes(word)) throw new Error('invalid');
    spelling.delete(sender);
    sender.replaceMisspelling(word);
    return true;
  },
  addToDictionary: (sender) => {
    const found = spelling.get(sender);
    if (!found || !found.word) throw new Error('invalid');
    spelling.delete(sender);
    return sender.session.addWordToSpellCheckerDictionary(found.word);
  },
  // Save image as, from an email's context menu; the dialog belongs to the window it was asked in.
  saveImage: async (sender, src) => {
    const img = await emailImage(src);
    const owner = BrowserWindow.fromWebContents(sender) || win;
    const res = await dialog.showSaveDialog(owner, { defaultPath: path.join(app.getPath('downloads'), img.name) });
    if (res.canceled || !res.filePath) return false;
    fs.writeFileSync(res.filePath, img.content);
    markOfTheWeb(res.filePath);
    return res.filePath;
  },
  // Send in an editor. With Undo send on, the email waits in the outbox while the main window shows the notice, and
  // the answer is { queued, ms }; otherwise it goes now and the answer is true. restore: what opens the editor again
  // (composer.js undoState). A send without one, or without a main window for the notice, goes at once.
  send: async (sender, payload, restore) => {
    const ms = undoSendDelay();
    if (ms > 0 && restore && typeof restore === 'object' && win && !win.isDestroyed()) {
      // Refused now, while the editor is still open to show why.
      engine.checkSend(payload);
      const { id } = outbox.queue({ payload, restore, origin: sender === win.webContents ? 'main' : 'window', ms });
      send('send-queued', { id, ms });
      return { queued: id, ms };
    }
    await engine.send(payload);
    mailSent(payload);
    return true;
  },
  // Undo in the notice after Send. The email stops waiting and its editor opens again: the answer carries it for the
  // main window's reading pane, or a compose window opens for an email written in one. { late: true } once the
  // email is on its way.
  undoSend: (sender, id) => {
    const entry = outbox.undo(String(id || ''));
    if (!entry) return { late: true };
    if (entry.origin === 'main' && win && !win.isDestroyed() && sender === win.webContents) return { restore: entry.restore };
    openComposeWindow({ ...(entry.restore.opts || {}), restore: entry.restore });
    return { window: true };
  },
  // The emails Don't quit kept waiting (askQuitWithHeldMail), whose clocks stood still until the main window's page
  // could show their notices with Undo: they run again from now, and the answer is [{ id, ms }] for those notices.
  // The page asks when told ('send-paused') and once its first state is drawn. Empty for a compose window, and when
  // nothing stands still.
  undoNotices: (sender) => (fromMainWindow(sender) ? outbox.resume(UNDO_AFTER_QUIT_MS) : []),
  // The refused emails held for the main window (holdRefused) that it hasn't claimed yet: [{ id, restore, error }].
  // Main keeps them until the window says each one is open (refusedOpened), or asks main to open it (openRefused).
  sendFailures: (sender) => (fromMainWindow(sender) ? refused.waiting() : []),
  // The reading pane is free and takes this refused email now. false when it opened elsewhere meanwhile, or another
  // call of the window claimed it: the window then leaves it alone.
  claimRefused: (sender, id) => fromMainWindow(sender) && refused.claim(String(id || '')),
  // Its editor is open with it, so main lets it go. Nothing happens for one that is no longer held.
  refusedOpened: (sender, id) => fromMainWindow(sender) && refused.ack(String(id || '')),
  // The reading pane couldn't take it after all: it opens in a compose window, unless it opened elsewhere already.
  openRefused: (sender, id) => fromMainWindow(sender) && refused.release(String(id || '')),
  composeInit: (sender) => {
    const c = composeWindows.get(sender.id);
    return c ? c.opts : null;
  },
  // Lets a compose window claim its draft once autosave created one, so the draft cannot open twice.
  composeDraft: (sender, draftId) => {
    const c = composeWindows.get(sender.id);
    if (c) c.draftId = draftId ? String(draftId) : null;
  },
  // What went wrong in a window, for the log in the profile: an error nothing caught, or what app.js reports itself,
  // such as a refresh of the list that failed (src/renderer/diag.js). diag.fromWindow() takes only known fields, capped.
  logRenderer: (sender, entry) => Boolean(diag && diag.fromWindow(fromMainWindow(sender) ? 'main' : 'compose', plainObject(entry))),
  // What the question about unsaved changes after a close or quit came to: true once the editor has closed.
  unloadAnswer: (sender, closed) => answered(sender.id, closed === true),
  // Closes without asking again; the compose window has already handled unsaved changes.
  composeClose: (sender) => {
    const w = BrowserWindow.fromWebContents(sender);
    if (w && w !== win) w.destroy();
  }
};

// Error codes a window acts on, passed on with the message. 'auth': the mail server turned the sign-in down, so the
// setup form keeps its server fields closed and, with appPasswordProvider, links to that provider's app password
// page. Other codes stay here: Node's ENOSPC or a calendar's 'password' would otherwise reach the chat panel, which
// reads a code as one of the agent's own.
const WINDOW_CODES = new Set(['auth']);

// What a window gets besides the message: an allowed code, and for 'auth' the provider whose app password page helps.
// Booking errors (booking/rules.js bookingError) pass their code, and for a refused field its name (contracts f).
function windowError(err) {
  // Rukoo Voice's error codes (voice/PROTOCOL.md, plus offline and timeout) under a name of their own, so the meeting
  // view says them in its words and the chat panel never takes one for an agent's code.
  if (err instanceof VoiceError) return { voice: String(err.code).slice(0, 40), ...(typeof err.meeting === 'string' ? { meeting: err.meeting.slice(0, 100) } : {}) };
  if (err && err.booking === true && typeof err.code === 'string') {
    return typeof err.field === 'string' ? { code: err.code.slice(0, 40), field: err.field.slice(0, 200) } : { code: err.code.slice(0, 40) };
  }
  // Video errors (video/oauth.js videoError) pass their code, so a sign-in the user cancelled isn't shown as a failure.
  if (err && err.video === true && typeof err.code === 'string') return { code: err.code.slice(0, 40) };
  const code = err && WINDOW_CODES.has(err.code) ? err.code : null;
  if (!code) return {};
  return code === 'auth' && typeof err.appPasswordProvider === 'string' ? { code, appPasswordProvider: err.appPasswordProvider } : { code };
}

ipcMain.handle('mail:call', async (event, method, args) => {
  const own = Object.prototype.hasOwnProperty;
  if (!own.call(api, method) && !own.call(windowApi, method)) return { ok: false, error: `Onbekende actie: ${method}` };
  const started = Date.now();
  try {
    const result = own.call(windowApi, method) ? await windowApi[method](event.sender, ...(args || [])) : await api[method](...(args || []));
    return { ok: true, result };
  } catch (err) {
    return { ok: false, error: (err && err.message) || String(err), ...windowError(err) };
  } finally {
    if (process.env.SEM_TRACE) console.log(`[ipc] ${method} ${Date.now() - started}ms`);
  }
});

let menuLanguage = null;
function updateApplicationMenu() {
  if (process.platform !== 'darwin') { Menu.setApplicationMenu(null); return; }
  if (menuLanguage === engine.settings.language) return;
  menuLanguage = engine.settings.language;
  Menu.setApplicationMenu(Menu.buildFromTemplate(macMenu(t, (action) => {
    const focused = BrowserWindow.getFocusedWindow();
    if (focused) focused.webContents.send('mail:event', { type: 'menu-edit', payload: { action } });
  }, (action) => setZoom(nextZoom(engine.settings.zoomLevel, action)))));
}

function createWindow({ hidden = false } = {}) {
  const b = windowState.bounds('main', { width: 1440, height: 920, minWidth: 760, minHeight: 560 });
  const { bar, background } = themeColors();
  // A new window draws the badge again.
  badgeShown = null;
  win = new BrowserWindow({
    width: b.width,
    height: b.height,
    ...(Number.isFinite(b.x) ? { x: b.x, y: b.y } : {}),
    minWidth: 760,
    minHeight: 560,
    show: false,
    title: 'Rukoo Mail',
    icon: fs.existsSync(ICON) ? ICON : undefined,
    backgroundColor: background,
    ...windowChrome(process.platform, bar),
    webPreferences: WEB_PREFERENCES
  });
  // Before it first shows, so a meeting that records never captures it (protectWindows).
  if (captureHidden) win.setContentProtection(true);
  updateApplicationMenu();
  harden(win);
  forwardEditMenu(win);
  watchZoom(win);
  watchQuit(win);
  win.on('focus', () => {
    newSinceFocus.clear();
    refreshBadge();
    syncOnReturn();
  });
  win.on('show', syncOnReturn);
  win.on('restore', syncOnReturn);
  // Close to the tray: the window hides before its page hears of the close, so nothing asks about unsaved
  // changes and a draft in the reading pane stays as it is. The agents' tools keep their window too.
  win.on('close', (event) => {
    // While its page asks about unsaved changes for a quit, the window stays in view with the question.
    if (quitAsks.has(win.webContents.id)) {
      event.preventDefault();
      return;
    }
    if ((quitting && !quitAsks.size) || (process.platform !== 'darwin' && !engine.settings.closeToTray)) return;
    event.preventDefault();
    win.hide();
  });
  win.on('minimize', () => {
    if (engine.settings.minimizeToTray) win.hide();
  });
  win.on('closed', () => {
    win = null;
    if (agentGate) agentGate.close();
    // Refused emails this window never took open elsewhere (holdRefused).
    refused.letGo();
  });
  win.webContents.on('render-process-gone', () => refused.letGo());
  // A new page asks again for the refused emails the old one had claimed but not opened.
  win.webContents.on('did-navigate', () => refused.unclaim());
  // A reload starts a new panel, which says again when it listens. Only the page itself counts: an email's frame
  // loads too, and the panel that opened it keeps listening.
  win.webContents.on('did-start-navigation', (details) => {
    if (agentGate && details && details.isMainFrame && !details.isSameDocument) agentGate.close();
  });
  if (!process.env.SEM_HIDDEN) windowState.track('main', win);
  reveal(win, b.maximized, hidden);
  win.loadFile(path.join(RENDERER, 'index.html'));
}

// A mailto: link Rukoo was started with, opened once the main window is up.
const mailtoQueue = new MailtoQueue(openMailto);
mailtoQueue.accept(mailto.fromArgs(process.argv));
app.on('open-url', (event, url) => {
  if (mailtoQueue.accept(url)) event.preventDefault();
});

// An .ics file Rukoo was opened with: its card shows in the main window, once that is up.
const icsQueue = new FileQueue(openIcsFile);
icsQueue.accept(icsFromArgs(process.argv));
app.on('open-file', (event, file) => {
  if (!/[.]ics$/i.test(String(file || ''))) return;
  event.preventDefault();
  icsQueue.accept(file);
});

// The cards the main window has yet to show. It takes them when told (invite-file) and after its first state, so a
// file Rukoo starts with isn't lost when the message comes before the window listens.
const inviteFiles = [];
function showInviteFile(token) {
  inviteFiles.push(token);
  send('invite-file');
}

function openIcsFile(file) {
  showWindow();
  try {
    showInviteFile(invitations.openFile(file));
  } catch (_) {
    dialog.showMessageBox(win, { type: 'warning', message: t('errors.invite.unreadable'), detail: file, buttons: [t('common.actions.ok')] }).catch(() => {});
  }
}

const isIcs = (a) => invite.isCalendarType(a.contentType) || /[.]ics$/i.test(String(a.filename || ''));
const inviteSource = (source) => {
  const s = plainObject(source);
  return { id: accountText(s.id, 2000) || undefined, file: accountText(s.file, 100) || undefined };
};
app.on('activate', () => {
  if (engine && windowState) showWindow();
});

// Rukoo started again. For a mailto: link (Windows starts the app with it) only its compose window comes up, and
// the main window stays where it was, in the tray too.
app.on('second-instance', (e, argv, workingDirectory) => {
  const link = mailto.fromArgs(argv);
  const file = link ? null : icsFromArgs(argv, workingDirectory);
  if (link) mailtoQueue.accept(link);
  else if (file) icsQueue.accept(file);
  else if (engine && windowState) showWindow();
});

app.whenReady().then(() => {
  windowState = new WindowState(path.join(app.getPath('userData'), 'window-state.json'));
  // Tests run offline: no logo downloads unless asked for.
  logos = new Logos(path.join(app.getPath('userData'), 'logos'), { enabled: !process.env.SEM_HIDDEN || Boolean(process.env.SEM_LOGOS) });
  google = new GoogleAuth({
    configPath: path.join(app.getPath('userData'), 'google-oauth.json'),
    openBrowser: (url) => shell.openExternal(url)
  });
  diag = new DiagLog(path.join(app.getPath('userData'), 'data', 'logs'));
  diag.info('app', `start ${app.getVersion()} ${process.platform} ${process.arch}`);
  if (process.env.SEM_HIDDEN) global.__semDiag = diag;
  engine = new Engine({ dataDir: path.join(app.getPath('userData'), 'data'), secrets, google, log: (level, target, message) => diag.write(level, target, message) }).init();
  // Test mode only: lets end-to-end tests simulate server conditions.
  if (process.env.SEM_HIDDEN) global.__semEngine = engine;
  outbox = new Outbox({ send: (payload) => engine.send(payload), sent: (entry) => mailSent(entry.payload, entry.id), failed: waitedMailFailed });
  if (process.env.SEM_HIDDEN) global.__semOutbox = outbox;
  engine.on('updated', () => {
    // The Mac menu follows the language; elsewhere the menu stays off and needs no update.
    if (process.platform === 'darwin') updateApplicationMenu();
    send('updated');
    // The badge follows the mail in both counts: an email read, moved or put in a category Inbox doesn't show.
    const count = badgeCount();
    if (count !== badgeShown) refreshBadge(count);
    else refreshTray(count);
  });
  engine.on('new-mail', notify);
  calendars = new Calendars({ engine });
  invitations = new Invitations({ engine, calendars, dataDir: path.join(app.getPath('userData'), 'data') });
  if (process.env.SEM_HIDDEN) global.__semInvitations = invitations;
  if (process.env.SEM_HIDDEN) global.__semCalendars = calendars;
  calendars.on('updated', () => send('calendar'));
  // A fork points the app at its own account server with package.json's rukoo.accountServer; tests and development
  // use RUKOO_ACCOUNT_SERVER.
  account = new AccountService({
    dataDir: path.join(app.getPath('userData'), 'data'),
    secrets,
    defaultServer: process.env.RUKOO_ACCOUNT_SERVER || PACKAGE_RUKOO.accountServer || ''
  }).init();
  if (process.env.SEM_HIDDEN) global.__semAccount = account;
  account.on('status', (status) => broadcast('account', status));
  account.sync().catch(() => {});
  categories = new Categories({ engine, account, secrets, dataDir: path.join(app.getPath('userData'), 'data') }).init();
  if (process.env.SEM_HIDDEN) global.__semCategories = categories;
  categories.on('changed', () => broadcast('categories', categories.state()));
  categories.on('progress', (activity) => broadcast('categories-progress', activity));
  mailNotices = new MailNotices({ categories, show: showMailNotices });
  // Sign-ins to Zoom and Microsoft Teams stay on this computer; their OAuth clients come from SEM_* variables or a
  // client file next to google-oauth.json (docs/booking.md).
  video = new VideoServices({ dataDir: path.join(app.getPath('userData'), 'data'), configDir: app.getPath('userData'), secrets, openBrowser: (url) => shell.openExternal(url) }).init();
  if (process.env.SEM_HIDDEN) global.__semVideo = video;
  video.on('changed', (s) => broadcast('video', s));
  booking = new Booking({ account, calendars, engine, secrets, video, dataDir: path.join(app.getPath('userData'), 'data'), notify: showBookingNotification }).init();
  if (process.env.SEM_HIDDEN) global.__semBooking = booking;
  booking.on('state', (s) => broadcast('booking', s));
  // Deleting the Rukoo account turns the booking page off first, and stops when that fails (decision 22).
  account.beforeDelete = () => booking.disableForDelete();
  reminders = new ReminderRunner({ calendars, engine, dataDir: path.join(app.getPath('userData'), 'data'), notify: showReminders, powerMonitor });
  if (process.env.SEM_HIDDEN) global.__semReminders = reminders;
  reminders.start();
  // New releases on GitHub (package.json's rukoo.releases). Tests and development profiles (SEM_HIDDEN, SEM_DATA_DIR)
  // don't ask GitHub; a test points it at its own server with RUKOO_RELEASES_URL.
  updates = new UpdateChecker({
    current: app.getVersion(),
    repo: (process.env.SEM_HIDDEN || process.env.SEM_DATA_DIR) && !process.env.RUKOO_RELEASES_URL ? null : PACKAGE_RUKOO.releases,
    url: process.env.RUKOO_RELEASES_URL,
    enabled: () => engine.settings.checkUpdates !== false,
    channel: () => channelFor(engine.settings.updateChannel, app.getVersion())
  }).start();
  // Downloads what a check finds, where this install can replace itself (installer.js). Hidden test windows don't;
  // a test sets the installer's state itself.
  installer = new UpdateInstaller({ kind: process.env.SEM_HIDDEN ? null : installKind({ packaged: app.isPackaged }) });
  if (process.env.SEM_HIDDEN) {
    global.__semUpdates = updates;
    global.__semInstaller = installer;
  }
  updates.on('changed', () => broadcast('updates', updatesState()));
  // After every check, so a download that failed tries again with the next one.
  updates.on('checked', (state) => installer.download(state.available).catch(() => {}));
  installer.on('changed', () => broadcast('updates', updatesState()));
  // ---- voice ----
  // Before the hub, whose meeting tools use it, and before the first window, which it may have to hide. A development
  // or test profile (SEM_DATA_DIR) only finds a Voice paired through RUKOO_VOICE_BRIDGE, never the installed one.
  voice = new VoiceService({
    dataDir: path.join(app.getPath('userData'), 'data'),
    discovery: bridgePathHere(process.env),
    appVersion: app.getVersion(),
    calendars,
    hide: () => engine.settings.meetingHideWindow !== false,
    protect: protectWindows,
    log: (scope, line) => {
      if (!process.env.SEM_HIDDEN) console.warn(`[voice:${scope}] ${line}`);
    }
  }).init();
  if (process.env.SEM_HIDDEN) global.__semVoice = voice;
  voice.on('state', () => send('meeting', { kind: 'state', state: voiceState() }));
  voice.on('meeting', (payload) => send('meeting', payload));
  // ---- /voice ----
  // ---- agents ----
  // Starts in the background: the panel shows "starting" until the first 'agents' status event.
  hub = new AgentHub({
    engine,
    dataDir: path.join(app.getPath('userData'), 'data'),
    secrets,
    deps: {
      // The calendar tools: list_events and create_event use the store, find_free_time the same free time as the
      // availability panel, get_booking_link the booking page while it is on (link() and hours() are null when off).
      calendars,
      calendarFree: (opts) => freeTime(calendars, engine.settings, booking ? booking.hours() : null, opts),
      bookingHours: () => (booking ? booking.hours() : null),
      bookingLink: () => (booking ? booking.link() : null),
      // The meeting tools read what Rukoo keeps of Rukoo Voice's meetings.
      voice,
      unsubscribe: (id) => api.unsubscribe(id),
      // A mailto-only unsubscribe opens a filled-in compose window; the user sends it.
      openMailto: (url) => openUrl(url),
      appVersion: app.getVersion(),
      clipboard,
      // With Rukoo open on several devices, Clark's bridge picks the one the user touched last.
      idleSeconds: () => powerMonitor.getSystemIdleTime(),
      hasWindow: () => Boolean(win && !win.isDestroyed()),
      workspace: path.join(app.getPath('userData'), 'agent-workspace'),
      // Rukoo's own skills ship inside the app (package.json build.files); the user's own replace them by name.
      skills: { bundled: path.join(app.getAppPath(), 'skills'), user: path.join(app.getPath('userData'), 'skills') }
    }
  });
  agentGate = new PanelGate((payload) => send('agent', payload));
  hub.on('event', (payload) => agentGate.event(payload));
  if (process.env.SEM_HIDDEN) global.__semAgents = hub;
  hub.start().catch((err) => console.error('[agents] start failed:', err));
  // ---- /agents ----
  nativeTheme.on('updated', () => {
    applyTheme();
    broadcast('theme');
  });
  applyTheme();
  createWindow();
  // After the main window shows, so the compose window comes up in front of it.
  win.once('ready-to-show', () => {
    mailtoQueue.start();
    icsQueue.start();
    // Emails the last quit couldn't send (keepUnsent).
    reopenUnsent();
  });
  refreshMailClient().catch(() => {});
  updateTray();
  scheduleSync();
  engine.syncAll().catch(() => {});
  calendars.syncAll().catch(() => {});
});

// With "Close to the tray" the main window only closes in a quit. If a compose window then cancels that quit,
// Rukoo stays in the tray after the compose window closes, and the tray opens a new main window.
app.on('window-all-closed', () => {
  if (process.platform !== 'darwin' && (quitting || !engine || !engine.settings.closeToTray)) app.quit();
});

// Quit in the tray menu, the last window closing, and the e2e teardown all come through here.
// Quitting while email still waits out its Undo notice asks first: Send and quit (the default) or Don't quit, which
// keeps the emails waiting (outbox.js askBeforeQuit). Emails already on their way can't be undone: the quit waits for
// them without asking (will-quit). Asked once per quit; a quit from the Windows session ending never asks
// (endSession). Test runs answer through global.__semQuitAnswer (0 send, 1 stay) instead of a native dialog, and
// count the questions in global.__semQuitAsked.
function askQuitWithHeldMail() {
  if (quitAsked || !outbox) return true;
  const answer = askBeforeQuit(outbox, (count) => {
    if (process.env.SEM_HIDDEN) {
      global.__semQuitAsked = (global.__semQuitAsked || 0) + 1;
      return global.__semQuitAnswer || 0;
    }
    const parent = win && !win.isDestroyed() && win.isVisible() ? win : null;
    return dialog.showMessageBoxSync(parent, {
      type: 'question',
      title: 'Rukoo Mail',
      message: t('composer.undoSend.quitTitle', { count }),
      detail: t('composer.undoSend.quitHelp'),
      buttons: [t('composer.undoSend.quitSend'), t('composer.undoSend.quitStay')],
      defaultId: 0,
      cancelId: 1
    });
  });
  if (answer === 'stay') {
    // Don't quit calls off the whole quit, also an earlier one that a window held and then let go on (answered()).
    // The main window comes forward with the notices: out of the tray, or made anew after the last window closed.
    // The emails' clocks stand still until its page can show those notices with Undo and asks for them
    // (undoNotices); a window that still loads would otherwise let an email go without one.
    cancelQuit();
    showWindow();
    send('send-paused');
    return false;
  }
  if (answer === 'send') quitAsked = true;
  return true;
}

app.on('before-quit', (event) => {
  if (!askQuitWithHeldMail()) {
    event.preventDefault();
    return;
  }
  quitting = true;
  // Every quit asks the windows afresh; one that still has a question open says so again from its beforeunload.
  quitAsks.clear();
  clearInterval(syncTimer);
  if (engine) engine.flush();
  if (calendars) calendars.flush();
  if (account) account.close();
  if (categories) categories.close();
  if (mailNotices) mailNotices.close();
  if (reminders) reminders.stop();
  if (booking) booking.close();
});

// Every window has closed, so the quit goes through.
app.on('will-quit', destroyTray);
// ---- voice ----
// Not in before-quit, which a compose window can still call off; synchronous, before the agents' wait. Never
// meeting.stop: Voice keeps recording when Rukoo quits.
app.on('will-quit', () => {
  if (voice) voice.close();
});
// ---- /voice ----

// ---- agents ----
// dispose() stops turns and flushes the conversations before its first await, and starts the kills of the
// agent processes right after, before any timer can fire; none of that blocks. A kill only finishes while
// Rukoo runs: Node puts its child processes, taskkill included, in a job that Windows ends together with
// Rukoo, and that would leave the agents' own commands running. Clark's runs only end with a stop request
// (each with a 2 s timeout), which a process that exits right away never sends. So the quit waits once,
// until dispose is done or QUIT_WAIT has passed, the time each taskkill had when it still blocked.
// will-quit, not before-quit: a window can still cancel the quit after before-quit (a pop-out composer that
// asks to save), and the agents must keep running then. will-quit comes once every window has closed.
const QUIT_WAIT = 5000;
// Closing the agents starts once, at the end of the Windows session or in will-quit, and a quit waits for it
// either way: dispose() a second time returns at once.
let agentsClosing = null;
function closeAgents() {
  if (!agentsClosing) agentsClosing = hub.dispose().catch(() => {});
  return agentsClosing;
}
// Email that still waits out its Undo notice goes before Rukoo exits, once before-quit asked (askQuitWithHeldMail):
// the windows that could take it back have closed. The sends get SEND_QUIT_WAIT, so a server that stops answering can't hold the quit for ever;
// what is still on its way then, or was refused and couldn't go to Drafts, opens again at the next start.
const SEND_QUIT_WAIT = 30000;
let quitWaited = false;
// While those sends run, a second quit (Quit again in the tray, or the last window closing after a quit from a test)
// waits for them too. Cut short, the emails on their way would be neither surely sent nor written down.
let quitSending = false;
app.on('will-quit', (event) => {
  if (quitSending) return event.preventDefault();
  if (quitWaited || (!hub && !(outbox && outbox.size) && !unsent.length)) return;
  quitWaited = true;
  event.preventDefault();
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  // The sends change the mail cache after before-quit saved it, so it is saved again. What they leave is written
  // down, also when they failed earlier in the quit, while a window still asked about its changes.
  const sending = Boolean(outbox && outbox.size);
  quitSending = sending;
  const mail = (sending ? Promise.race([outbox.flush(), wait(SEND_QUIT_WAIT)]) : Promise.resolve()).then(() => {
    keepUnsent();
    if (sending) engine.flush();
    quitSending = false;
  });
  const agents = hub ? Promise.race([closeAgents(), wait(QUIT_WAIT)]) : null;
  // Every window is closed and the engine flushed by now, so exit outright: app.quit() after a prevented
  // will-quit does not finish quitting.
  // A downloaded update installs now, with nothing of Rukoo left running that it could interrupt. Only while it is
  // still what the check offers: not after a switch back to Stable. With the check turned off, only Restart to update
  // installs, since Check now can still download one.
  Promise.all([mail, agents]).finally(() => {
    const offered = updates && updates.available;
    if (installer && offered && installer.version === offered.version && (restartForUpdate || engine.settings.checkUpdates !== false)) {
      installer.install({ restart: restartForUpdate });
    }
    app.exit(0);
  });
});
// ---- /agents ----
