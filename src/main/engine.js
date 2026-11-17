'use strict';

const { t, setLanguage, normalizeLanguage, getLocale } = require('../i18n');

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { EventEmitter } = require('events');
const { simpleParser } = require('mailparser');
const { ImapAccount, friendlyError, errorCode } = require('./imap');
const { DemoAccount, DEMO_EMAIL } = require('./demo');
const { serverDefaults, publicProviders } = require('./providers');
const { discoverCalDav, serverUrl } = require('./calendar/caldav');
const invite = require('./calendar/invite');
const util = require('./mailutil');
const { cleanSignatureHtml, signatureText } = require('./signature');
const { normalizeZoom } = require('./zoom');

const COLORS = ['#2fd6c0', '#4a7dff', '#ff8a3d', '#c56cf0', '#f5c542', '#ff5d73', '#5ec2ff'];
const SYNC_ROLES = ['inbox', 'sent', 'drafts', 'trash', 'junk', 'archive'];
// The newest messages the cache keeps of the inbox and of every other folder. Load older (loadOlder()) widens a
// folder's window by batch messages at a time, to at most extra beyond these.
const LIMITS = { inbox: 300, default: 100, batch: 100, extra: 1000 };
// The views Load older can widen: each lists whole folders, so what it says is on the server is what it can bring in.
// Unread and VIPs pick from the inbox, and their header names the older unread mail instead.
const OLDER_VIEWS = ['inbox', 'folder', 'sent', 'drafts', 'trash', 'junk', 'archive'];
// Searching the server for older mail (searchServer()): from this many characters, this many folders per account at a
// time and the newest hits matches per folder, with up to kept hits per folder held for opening them.
const SEARCH = { min: 2, folders: 12, hits: 50, kept: 300 };
// After a sync, the newest inbox messages up to this size are downloaded ahead of time; see prefetch(). A larger one
// shows its text before its attachments when it is opened; see readableSource().
const PREFETCH = { count: 30, maxBytes: 1024 * 1024 };
// An empty picture without a size of its own, in place of an inline picture that is still on its way. The reader
// sizes pictures by their own proportions (mailframe.js), so a transparent pixel would turn a 560 x 120 banner into a
// square. This one keeps the box the picture's width and height give it, and takes no room without them. Encoded
// without quotes or spaces, so it fits any attribute or url() the empty address stood in.
const BLANK_IMAGE = 'data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%220%22%20height=%220%22/%3E';
const VIEW_ROLES = { drafts: 'drafts', sent: 'sent', trash: 'trash', junk: 'junk', archive: 'archive' };
// Earlier versions put this signature under every mail by default; it is now opt-in.
const OLD_DEFAULT_SIGNATURE = 'Verzonden vanaf mijn pc';
// How long a move or delete can be undone.
const UNDO_MS = 10 * 60 * 1000;
// How many moves per account are remembered by Message-ID, so a reference can find the mail again.
const MOVES_KEPT = 500;
// How many earlier folders a move record keeps for its copy.
const MOVE_PAST = 5;
// Folders whose mail is not archived, even though Gmail's All Mail also holds it.
const SYSTEM_ROLES = ['inbox', 'sent', 'drafts', 'trash', 'junk'];
// How many related emails the reader gets under an email; see related().
const RELATED_LIMIT = 20;
// Settings > General > Undo send: how many seconds Send waits before the email goes (main.js and outbox.js), as
// Apple Mail and Gmail offer it. 0 is off. The renderer's list in settings.js offers the same values.
const UNDO_SEND_SECONDS = [0, 5, 10, 20, 30];
const UNDO_SEND_DEFAULT = 5;

const DEFAULT_SETTINGS = {
  language: 'en',
  theme: 'system',
  swipeActions: true,
  fitContent: true,
  darkEmails: true,
  notifications: true,
  // A short sound for new mail, played by the main window (main.js showMailNotices). Off by default.
  newMailSound: false,
  badge: 'new',
  density: 'standard',
  sort: 'date-desc',
  syncInterval: 5,
  hiddenViews: [],
  vips: [],
  spam: [],
  defaultAccountId: null,
  signature: '',
  // The formatted signature, already cleaned; '' for a plain-text one. signature then holds its text, which is what
  // earlier versions show. See signature.js.
  signatureHtml: '',
  // The booking page's link under the signature, opt-in. bookingFooterText: the footer's own text with {url} where
  // the link goes, or null for the default text in the booking page's language. Accounts can override both.
  bookingFooter: false,
  bookingFooterText: null,
  // Calendar choices and defaults; see normalizeCalendar(). Changed only through calendarSettings().
  calendar: calendarDefaults(),
  senderLogos: true,
  // Three or more emails in a row from one sender, on one day, fold into one row in the list (renderer stackLayout).
  // Off by default, so every email has its own row.
  stackSenders: false,
  // Seconds an email waits after Send, so the notice that follows can take it back; one of UNDO_SEND_SECONDS.
  undoSend: UNDO_SEND_DEFAULT,
  // Percent, shared by the main window and the compose windows; see zoom.js.
  zoomLevel: 100,
  // Closing the main window hides it and Rukoo keeps running in the tray; minimizing can do the same. See main.js.
  closeToTray: process.platform === 'win32',
  minimizeToTray: false,
  // What a swipe on a list row does, to the right and to the left (touch and pen). See SWIPE_ACTIONS.
  swipeRight: 'read',
  swipeLeft: 'delete',
  // The buttons on a list row under the mouse, in order. See QUICK_ACTIONS.
  quickActions: ['archive', 'read', 'delete'],
  // Whose keyboard shortcuts the main window uses. See src/renderer/keymap.js.
  keymap: 'rukoo',
  // Ask GitHub for new releases now and then. See updates.js.
  checkUpdates: true,
  // 'stable' or 'nightly'; null follows the installed build (updates.js channelFor).
  updateChannel: null,
  // The version whose "What's new" guide was last shown.
  whatsNewSeen: '',
  // Settings > Meetings. Rukoo's windows are left out of screen capture while Rukoo Voice records a meeting
  // (voice/index.js), and the agents' meeting tools answer (agents/tools.js).
  meetingHideWindow: true,
  meetingAgentsCanRead: true
};

const SWIPE_ACTIONS = ['none', 'read', 'archive', 'delete', 'star', 'move', 'category'];
const QUICK_ACTIONS = ['archive', 'read', 'delete', 'star', 'move', 'category'];
const KEYMAPS = ['rukoo', 'gmail', 'outlook', 'apple', 'thunderbird'];

// Values a hand-edited or older settings.json may get wrong. The tray switches are booleans with their default
// for anything else.
function normalizeSettings(s) {
  s.language = normalizeLanguage(s.language);
  s.zoomLevel = normalizeZoom(s.zoomLevel);
  s.closeToTray = typeof s.closeToTray === 'boolean' ? s.closeToTray : DEFAULT_SETTINGS.closeToTray;
  s.minimizeToTray = s.minimizeToTray === true;
  s.stackSenders = s.stackSenders === true;
  s.undoSend = UNDO_SEND_SECONDS.includes(s.undoSend) ? s.undoSend : UNDO_SEND_DEFAULT;
  s.newMailSound = s.newMailSound === true;
  s.signatureHtml = typeof s.signatureHtml === 'string' ? s.signatureHtml : '';
  s.swipeRight = SWIPE_ACTIONS.includes(s.swipeRight) ? s.swipeRight : DEFAULT_SETTINGS.swipeRight;
  s.swipeLeft = SWIPE_ACTIONS.includes(s.swipeLeft) ? s.swipeLeft : DEFAULT_SETTINGS.swipeLeft;
  s.quickActions = Array.isArray(s.quickActions) ? [...new Set(s.quickActions.filter((a) => QUICK_ACTIONS.includes(a)))].slice(0, 4) : DEFAULT_SETTINGS.quickActions;
  s.keymap = KEYMAPS.includes(s.keymap) ? s.keymap : DEFAULT_SETTINGS.keymap;
  s.checkUpdates = s.checkUpdates !== false;
  s.updateChannel = ['stable', 'nightly'].includes(s.updateChannel) ? s.updateChannel : null;
  s.whatsNewSeen = typeof s.whatsNewSeen === 'string' ? s.whatsNewSeen : '';
  s.meetingHideWindow = s.meetingHideWindow !== false;
  s.meetingAgentsCanRead = s.meetingAgentsCanRead !== false;
  s.calendar = normalizeCalendar(s.calendar);
  // A footer text that can't carry the link ({url}), or is longer than 300 characters, falls back to the default.
  const footerText = (v) => (typeof v === 'string' && v.trim() && v.trim().length <= 300 && v.includes('{url}') ? v.trim() : null);
  s.bookingFooter = s.bookingFooter === true;
  s.bookingFooterText = footerText(s.bookingFooterText);
}

// The calendar settings with nothing chosen yet. A new object each time: settings objects share it by reference.
function calendarDefaults() {
  return {
    // Per calendar ("accountId\ncalendarId"): the eye toggle (missing: the provider's own "shown"), counted toward
    // availability (missing: the account's primary and the user's own calendars) and a colour.
    visible: {},
    availability: {},
    colors: {},
    // null: the locale's first day; 0 Sunday, 1 Monday, 6 Saturday.
    weekStart: null,
    weekends: true,
    workDays: [1, 2, 3, 4, 5],
    workStart: '09:00',
    workEnd: '17:00',
    // Minutes.
    duration: 60,
    // Minutes before timed events whose reminders aren't known; null for none.
    reminder: 10,
    notify: true,
    notifiedUpgrade: false
  };
}

const CALENDAR_MAPS = ['visible', 'availability', 'colors'];
const CALENDAR_KEYS = 500;
const CALENDAR_TIME = /^(?:[01]\d|2[0-3]):[0-5]\d$|^24:00$/;

// What each per-calendar map may hold.
const CALENDAR_VALUES = {
  visible: (x) => typeof x === 'boolean',
  availability: (x) => typeof x === 'boolean',
  colors: (x) => typeof x === 'string' && /^#[0-9a-f]{6}$/i.test(x)
};

// A complete calendar settings object: what is missing or wrong gets its default, and the maps keep only valid
// entries (booleans, '#rrggbb' colours), at most 500 each. fallback, a complete object itself, replaces the defaults:
// a change keeps what the user had where it doesn't fit. Never changes its arguments.
function normalizeCalendar(value, fallback = null) {
  const v = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  const out = { ...calendarDefaults(), ...(fallback || {}) };
  out.workDays = [...out.workDays];
  const map = (m, ok) => {
    const kept = {};
    if (!m || typeof m !== 'object' || Array.isArray(m)) return kept;
    for (const [k, x] of Object.entries(m)) {
      if (Object.keys(kept).length >= CALENDAR_KEYS) break;
      if (k && k.length <= 2000 && ok(x)) kept[k] = x;
    }
    return kept;
  };
  for (const name of CALENDAR_MAPS) out[name] = map(v[name], CALENDAR_VALUES[name]);
  if ([null, 0, 1, 6].includes(v.weekStart)) out.weekStart = v.weekStart;
  if (typeof v.weekends === 'boolean') out.weekends = v.weekends;
  if (Array.isArray(v.workDays) && v.workDays.every((d) => Number.isInteger(d) && d >= 0 && d <= 6)) out.workDays = [...new Set(v.workDays)].sort((a, b) => a - b);
  // Both ends together, so a pair that doesn't make a range falls back as a pair.
  const workStart = CALENDAR_TIME.test(v.workStart) && v.workStart !== '24:00' ? v.workStart : out.workStart;
  const workEnd = CALENDAR_TIME.test(v.workEnd) ? v.workEnd : out.workEnd;
  if (workEnd > workStart) {
    out.workStart = workStart;
    out.workEnd = workEnd;
  }
  if (Number.isInteger(v.duration) && v.duration >= 15 && v.duration <= 480 && v.duration % 5 === 0) out.duration = v.duration;
  if (v.reminder === null || (Number.isInteger(v.reminder) && v.reminder >= 0 && v.reminder <= 10080)) out.reminder = v.reminder;
  if (typeof v.notify === 'boolean') out.notify = v.notify;
  if (typeof v.notifiedUpgrade === 'boolean') out.notifiedUpgrade = v.notifiedUpgrade;
  return out;
}

// A stored signature is cleaned again on loading, in case settings.json or accounts.json was edited by hand.
function reclean(html) {
  try {
    return cleanSignatureHtml(html);
  } catch (_) {
    return '';
  }
}

// What a signature patch stores: a formatted signature with its text, or plain text without HTML.
function signatureFields(patch) {
  if (patch.signatureHtml !== undefined && patch.signatureHtml !== null) {
    const html = cleanSignatureHtml(patch.signatureHtml);
    return { signature: signatureText(html), signatureHtml: html };
  }
  return { signature: String(patch.signature ?? ''), signatureHtml: '' };
}

function readJson(file, fallback) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (_) {
    return fallback;
  }
}

function writeJson(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(data));
  fs.renameSync(tmp, file);
}

function encodeId(accountId, folder, uid) {
  return [accountId, folder, uid].map(encodeURIComponent).join(':');
}

function decodeId(id) {
  const [accountId, folder, uid] = String(id).split(':').map(decodeURIComponent);
  return { accountId, folder, uid: Number(uid) };
}

function sha(text) {
  return crypto.createHash('sha1').update(text).digest('hex');
}

function lowerAddr(a) {
  return String((a && a.address) || a || '').trim().toLowerCase();
}

// A Message-ID header as a comparable key: no angle brackets, lower case.
function messageKey(v) {
  return String(v || '').trim().replace(/[<>]/g, '').toLowerCase();
}

// Recipients of an email to send: { name, address } objects or bare addresses, without empty ones.
function addressList(list) {
  return (list || []).map((a) => (typeof a === 'string' ? { name: '', address: a.trim() } : a)).filter((a) => a && a.address);
}

class Engine extends EventEmitter {
  // limits: other window sizes than LIMITS, and search: other sizes than SEARCH, for tests. log(level, target,
  // message): where each sync's line goes (main's diaglog.js); none in tests.
  constructor({ dataDir, secrets, google = null, fetchImpl = null, limits = null, search = null, log = null }) {
    super();
    this.limits = { ...LIMITS, ...(limits || {}) };
    this.search = { ...SEARCH, ...(search || {}) };
    this.fetch = fetchImpl || ((...args) => fetch(...args));
    this.dataDir = dataDir;
    this.secrets = secrets || { encrypt: (s) => s, decrypt: (s) => s };
    this.google = google;
    this.log = log;
    // Short-lived OAuth access tokens per account, kept in memory only.
    this.tokens = new Map();
    // Per account, how many times it signed in again; see grant().
    this.grants = new Map();
    this.accounts = [];
    this.settings = { ...DEFAULT_SETTINGS };
    this.caches = new Map();
    this.sessions = new Map();
    this.syncs = new Map();
    // When each account's last sync started, by id; see syncStale().
    this.syncStarts = new Map();
    this.saved = [];
    this.saveTimers = new Map();
    // Local changes whose IMAP command has not finished yet, per email in the order they were made; re-applied over
    // sync results. See holdPending().
    this.pending = new Map();
    // What those changes did to an inbox's count of older unread mail, per email; see recountOf().
    this.recounts = new Map();
    // Recent moves, keyed by the id the message had before it moved, so they can be reversed.
    this.undoable = new Map();
    // Message bodies being downloaded, by cache file; see download().
    this.downloads = new Map();
    // Accounts whose newest messages are being downloaded ahead of time.
    this.prefetching = new Set();
    // Older mail the server search found below a folder's window (searchServer()), by account and folder (foundKey()):
    // { uidValidity, hits }, where hits maps a uid to { msg, query }, query being the search it matched last. Kept in
    // memory only, so it can be opened, moved and deleted like cached mail.
    this.found = new Map();
    // Load older runs, by the view they widen; see loadOlder().
    this.loading = new Map();
    // The latest server search, which a call with resume continues: { id, queue }, queue holding the folders still to
    // search by account. See searchServer().
    this.searching = null;
    this.closed = false;
  }

  file(...parts) {
    return path.join(this.dataDir, ...parts);
  }

  init() {
    fs.mkdirSync(this.dataDir, { recursive: true });
    this.accounts = readJson(this.file('accounts.json'), []);
    const fresh = !fs.existsSync(this.file('settings.json'));
    this.settings = { ...DEFAULT_SETTINGS, ...readJson(this.file('settings.json'), {}) };
    normalizeSettings(this.settings);
    // The notice that Rukoo now reminds you of events is for profiles from before it did. A new one has never seen
    // Rukoo without reminders; the first saved setting (adding an account saves one) keeps this.
    if (fresh) this.settings.calendar.notifiedUpgrade = true;
    setLanguage(this.settings.language);
    // Once only, so a user who sets this signature again keeps it.
    if (!this.settings.signatureOptIn) {
      if (this.settings.signature === OLD_DEFAULT_SIGNATURE) this.settings.signature = '';
      this.settings.signatureOptIn = true;
      if (fs.existsSync(this.file('settings.json'))) this.persistSettings();
    }
    if (this.settings.signatureHtml) this.settings.signatureHtml = reclean(this.settings.signatureHtml);
    for (const acc of this.accounts) if (acc.signatureHtml) acc.signatureHtml = reclean(acc.signatureHtml);
    this.saved = readJson(this.file('saved.json'), []);
    for (const acc of this.accounts) {
      this.caches.set(acc.id, readJson(this.file('cache', `${acc.id}.json`), { folders: [], boxes: {}, lastSync: null }));
    }
    return this;
  }

  // ---------- persistence ----------

  persistAccounts() {
    writeJson(this.file('accounts.json'), this.accounts);
  }

  persistSettings() {
    writeJson(this.file('settings.json'), this.settings);
  }

  persistCache(accountId) {
    clearTimeout(this.saveTimers.get(accountId));
    this.saveTimers.set(
      accountId,
      setTimeout(() => {
        const cache = this.caches.get(accountId);
        // A save that fails here is made again with the next change and by flush() at quit. Thrown from a timer, the
        // error would open Electron's error box, which holds up the main process, a quit included, until it is closed.
        // The Electron tests quit Rukoo through the inspector, which can run flush() between this save's write and
        // its rename: the flush then has moved the temp file already.
        try {
          if (cache) writeJson(this.file('cache', `${accountId}.json`), cache);
        } catch (_) {
          // Saved later.
        }
      }, 250)
    );
  }

  flush() {
    for (const [accountId, timer] of this.saveTimers) {
      clearTimeout(timer);
      const cache = this.caches.get(accountId);
      if (cache) writeJson(this.file('cache', `${accountId}.json`), cache);
    }
    this.saveTimers.clear();
  }

  changed(accountId) {
    if (accountId) this.persistCache(accountId);
    this.emit('updated');
  }

  // ---------- accounts ----------

  account(id) {
    const acc = this.accounts.find((a) => a.id === id);
    if (!acc) throw new Error(t('errors.account.missing'));
    return acc;
  }

  session(acc) {
    if (this.sessions.has(acc.id)) return this.sessions.get(acc.id);
    const s =
      acc.type === 'demo' ? new DemoAccount(acc, this.file('demo-server.json')) : new ImapAccount(acc, this.credentials(acc));
    this.sessions.set(acc.id, s);
    return s;
  }

  credentials(acc) {
    if (acc.auth === 'oauth2') return { accessToken: () => this.accessToken(acc) };
    return this.secrets.decrypt(acc.secret);
  }

  // Returns a valid Google access token, refreshing it with the stored refresh token when needed.
  accessToken(acc) {
    const cached = this.tokens.get(acc.id);
    if (cached && cached.accessToken && cached.expiresAt - Date.now() > 60000) return Promise.resolve(cached.accessToken);
    if (cached && cached.refreshing) return cached.refreshing;
    if (!this.google) return Promise.reject(new Error(t('errors.google.unavailable')));
    // A refresh only writes back while it still owns the cache entry: signing in again during it replaces the
    // entry, and the old grant's token and scopes must not overwrite the new ones.
    const owns = () => this.tokens.get(acc.id)?.refreshing === refreshing;
    const refreshing = this.google
      .refresh(this.secrets.decrypt(acc.secret))
      .then((t) => {
        if (!owns()) return t.accessToken;
        this.tokens.set(acc.id, t);
        if (t.scopes) this.setScopes(acc.id, t.scopes);
        return t.accessToken;
      })
      .catch((err) => {
        if (owns()) this.tokens.delete(acc.id);
        throw err;
      });
    this.tokens.set(acc.id, { refreshing });
    return refreshing;
  }

  // How many times the account signed in again in this run. Work started under an earlier grant can tell from
  // this that its failures are about that grant, not the current one.
  grant(id) {
    return this.grants.get(id) || 0;
  }

  // What the account's Google grant covers. Accounts added before Rukoo stored this have none until the next
  // token refresh, which is also when access withdrawn in the Google account shows up.
  setScopes(id, scopes) {
    const acc = this.accounts.find((a) => a.id === id);
    const next = [...new Set((scopes || []).map(String))].sort();
    if (!acc || (acc.scopes && acc.scopes.join(' ') === next.join(' '))) return;
    acc.scopes = next;
    this.persistAccounts();
    this.emit('updated');
  }

  // Adds a Google account after the browser sign-in, or refreshes the grant of an existing one.
  async addGoogleAccount(grant, { reauthId = null } = {}) {
    const email = String(grant.email || '').toLowerCase();
    if (reauthId) {
      const acc = this.account(reauthId);
      if (acc.email.toLowerCase() !== email) {
        throw new Error(t('errors.google.wrongAccount', { signedInEmail: grant.email, accountEmail: acc.email }));
      }
      // Encrypted before the account changes: a keyring that can't store it leaves the account signing in as before.
      const secret = this.secrets.encrypt(grant.refreshToken);
      acc.auth = 'oauth2';
      acc.secret = secret;
      acc.scopes = [...new Set(grant.scopes || [])].sort();
      this.tokens.set(acc.id, { accessToken: grant.accessToken, expiresAt: grant.expiresAt });
      this.grants.set(acc.id, this.grant(acc.id) + 1);
      const old = this.sessions.get(acc.id);
      this.sessions.delete(acc.id);
      if (old) await old.close();
      this.persistAccounts();
      this.syncAccount(acc.id).catch(() => {});
      return this.publicAccount(acc);
    }
    const defaults = serverDefaults('google', email);
    const acc = {
      id: crypto.randomUUID(),
      type: 'imap',
      provider: 'google',
      auth: 'oauth2',
      email,
      name: grant.name || email.split('@')[0],
      label: email,
      color: COLORS[this.accounts.length % COLORS.length],
      signature: null,
      imap: { ...defaults.imap, user: email },
      smtp: { ...defaults.smtp, user: email },
      secret: this.secrets.encrypt(grant.refreshToken),
      scopes: [...new Set(grant.scopes || [])].sort(),
      createdAt: Date.now()
    };
    this.tokens.set(acc.id, { accessToken: grant.accessToken, expiresAt: grant.expiresAt });
    return this.finishAdd(acc);
  }

  // Every address this account can send from: the account address first, then its aliases.
  identities(acc) {
    const own = { address: acc.email, name: acc.name || '', primary: true };
    const aliases = (acc.aliases || [])
      .filter((a) => a.address && a.address.toLowerCase() !== acc.email.toLowerCase())
      .map((a) => ({ address: a.address, name: a.name || acc.name || '', primary: false }));
    return [own, ...aliases];
  }

  identity(acc, address) {
    const list = this.identities(acc);
    if (!address) return list.find((i) => i.address.toLowerCase() === String(acc.defaultFrom || '').toLowerCase()) || list[0];
    return list.find((i) => i.address.toLowerCase() === String(address).toLowerCase()) || null;
  }

  // Reads verified "Send mail as" addresses from Gmail. The https://mail.google.com/ scope covers this call.
  async fetchGmailAliases(id) {
    const acc = this.account(id);
    if (acc.auth !== 'oauth2') throw new Error(t('errors.google.aliasesAuth'));
    const token = await this.accessToken(acc);
    let res;
    try {
      res = await this.fetch('https://gmail.googleapis.com/gmail/v1/users/me/settings/sendAs', {
        headers: { authorization: `Bearer ${token}` },
        redirect: 'error'
      });
    } catch (_) {
      throw new Error(t('errors.google.unreachable'));
    }
    if (!res.ok) throw new Error(t('errors.google.aliases', { status: res.status }));
    const body = await res.json();
    const usable = (body.sendAs || []).filter((s) => s.isPrimary || s.verificationStatus === 'accepted');
    acc.aliases = usable
      .filter((s) => s.sendAsEmail.toLowerCase() !== acc.email.toLowerCase())
      .map((s) => ({ address: s.sendAsEmail.toLowerCase(), name: s.displayName || '' }));
    const gmailDefault = usable.find((s) => s.isDefault);
    // Only adopt Gmail's default when the user has not picked one in this app.
    if (!acc.defaultFrom && gmailDefault) acc.defaultFrom = gmailDefault.sendAsEmail.toLowerCase();
    if (acc.defaultFrom && !this.identity(acc, acc.defaultFrom)) acc.defaultFrom = null;
    acc.aliasesFetchedAt = Date.now();
    this.persistAccounts();
    this.emit('updated');
    return this.publicAccount(acc);
  }

  defaultAccount() {
    return this.accounts.find((a) => a.id === this.settings.defaultAccountId) || this.accounts[0] || null;
  }

  publicAccount(acc) {
    const cache = this.caches.get(acc.id) || {};
    return {
      id: acc.id,
      type: acc.type,
      provider: acc.provider,
      email: acc.email,
      name: acc.name,
      label: acc.label || acc.email,
      color: acc.color,
      // null for both: the account uses the general signature.
      signature: acc.signature,
      signatureHtml: acc.signatureHtml || null,
      // The booking link under this account's signature: null follows the general setting, like the signature.
      bookingFooter: typeof acc.bookingFooter === 'boolean' ? acc.bookingFooter : null,
      bookingFooterText: typeof acc.bookingFooter === 'boolean' && typeof acc.bookingFooterText === 'string' ? acc.bookingFooterText : null,
      // Whether the calendar server sends invitations, as set in Server settings; null: as its DAV header says.
      caldavInvites: acc.caldavInvites === 'provider' || acc.caldavInvites === 'none' ? acc.caldavInvites : null,
      auth: acc.auth || (acc.type === 'demo' ? 'none' : 'password'),
      identities: this.identities(acc),
      defaultFrom: this.identity(acc).address,
      imap: acc.imap ? { ...acc.imap } : null,
      smtp: acc.smtp ? { ...acc.smtp } : null,
      caldav: acc.caldav ? { url: acc.caldav.url } : null,
      isDefault: this.defaultAccount() === acc,
      lastSync: cache.lastSync || null,
      error: cache.error || null,
      syncing: this.syncs.has(acc.id),
      archive: (this.archiveFolder(acc.id) || {}).path || null,
      folders: (cache.folders || [])
        .filter((f) => f.role !== 'flagged' && f.role !== 'important' && (f.role !== 'all' || this.archiveFolder(acc.id) === f))
        // Gmail's All Mail is where archived mail goes, so it is shown as the archive.
        .map((f) => (f.role === 'all' ? { ...f, role: 'archive' } : f))
        .map((f) => ({ path: f.path, name: util.displayFolderName(f), role: f.role || null }))
    };
  }

  // The accounts with their secrets in plain text, for the one export a paired phone may fetch (agents/phonehost.js).
  // This is the only method that hands out a password or a Google refresh token; nothing else may call it. A secret
  // that does not decrypt here (stored under another Windows user or on another computer) is left out and marked, so
  // the phone can ask for it instead. Demo accounts have no secret.
  exportAccounts() {
    return this.accounts.map((acc) => {
      const auth = acc.auth || (acc.type === 'demo' ? 'none' : 'password');
      const out = {
        id: acc.id,
        type: acc.type,
        provider: acc.provider,
        auth,
        email: acc.email,
        name: acc.name,
        label: acc.label || acc.email,
        color: acc.color,
        signature: acc.signature == null ? null : acc.signature,
        signatureHtml: acc.signatureHtml || null,
        imap: acc.imap ? { ...acc.imap } : null,
        smtp: acc.smtp ? { ...acc.smtp } : null,
        caldav: acc.caldav ? { ...acc.caldav } : null,
        aliases: Array.isArray(acc.aliases) ? acc.aliases.map((a) => ({ ...a })) : [],
        defaultFrom: acc.defaultFrom || null,
        scopes: Array.isArray(acc.scopes) ? [...acc.scopes] : [],
        createdAt: acc.createdAt || null
      };
      if (acc.type === 'demo' || !acc.secret) return out;
      try {
        const secret = this.secrets.decrypt(acc.secret);
        if (auth === 'oauth2') out.refreshToken = secret;
        else out.password = secret;
      } catch (_) {
        out.secretError = 'decrypt-failed';
      }
      return out;
    });
  }

  state() {
    return {
      accounts: this.accounts.map((a) => this.publicAccount(a)),
      settings: this.settings,
      providers: publicProviders(),
      googleAvailable: Boolean(this.google && this.google.available()),
      demoEmail: DEMO_EMAIL
    };
  }

  async addAccount(input) {
    const email = String(input.email || '').trim();
    if (input.type !== 'demo' && !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) {
      throw new Error(t('errors.account.invalidEmail'));
    }
    const finalEmail = input.type === 'demo' ? DEMO_EMAIL : email;
    const defaults = serverDefaults(input.provider || 'other', finalEmail);
    const acc = {
      id: crypto.randomUUID(),
      type: input.type === 'demo' ? 'demo' : 'imap',
      provider: input.type === 'demo' ? 'demo' : input.provider || 'other',
      email: finalEmail,
      name: input.name || (input.type === 'demo' ? t('setup.demo.name') : finalEmail.split('@')[0]),
      label: finalEmail,
      color: COLORS[this.accounts.length % COLORS.length],
      signature: null,
      imap: input.type === 'demo' ? null : { ...defaults.imap, user: finalEmail, ...(input.imap || {}) },
      smtp: input.type === 'demo' ? null : { ...defaults.smtp, user: finalEmail, ...(input.smtp || {}) },
      secret: input.type === 'demo' ? null : this.secrets.encrypt(String(input.password || '')),
      createdAt: Date.now()
    };
    if (acc.type === 'imap' && !input.password) throw new Error(t('errors.account.password'));
    // The calendar server from the setup form, optional. An empty field adds the account without a calendar.
    const caldavUrl = acc.type === 'imap' && input.caldav ? String(input.caldav.url || '').trim() : '';
    return this.finishAdd(acc, caldavUrl ? { caldav: { url: caldavUrl } } : {});
  }

  // caldav: a calendar server entered while adding the account. It is checked with the same sign-in once the mail
  // servers answered, and the account is only saved when it answers too, as in Server settings.
  async finishAdd(acc, { caldav = null } = {}) {
    if (this.accounts.some((a) => a.email.toLowerCase() === acc.email.toLowerCase())) {
      throw new Error(t('errors.account.duplicate'));
    }
    const session = this.session(acc);
    try {
      await session.verify();
      if (caldav) acc.caldav = await this.setupCalDav(acc, caldav);
    } catch (err) {
      this.sessions.delete(acc.id);
      this.tokens.delete(acc.id);
      await session.close();
      throw err;
    }
    this.accounts.push(acc);
    this.caches.set(acc.id, { folders: [], boxes: {}, lastSync: null });
    if (!this.settings.defaultAccountId) {
      this.settings.defaultAccountId = acc.id;
      this.persistSettings();
    }
    this.persistAccounts();
    this.emit('updated');
    this.syncAccount(acc.id).catch(() => {});
    return this.publicAccount(acc);
  }

  async updateAccount(id, patch) {
    const acc = this.account(id);
    // Cleaned before anything changes, so a signature that is too large leaves the account as it was.
    // null resets the account to the general signature.
    const reset = patch.signature === null || patch.signatureHtml === null;
    const signature = patch.signature !== undefined || patch.signatureHtml !== undefined ? (reset ? { signature: null, signatureHtml: '' } : signatureFields(patch)) : null;
    if (patch.name !== undefined) acc.name = String(patch.name);
    if (signature) {
      acc.signature = signature.signature;
      acc.signatureHtml = signature.signatureHtml || null;
    }
    // The booking link under this account's signature. bookingFooter null goes back to the general setting, text and
    // all; a text without {url}, or over 300 characters, means the general text.
    if (patch.bookingFooter !== undefined) {
      if (typeof patch.bookingFooter === 'boolean') acc.bookingFooter = patch.bookingFooter;
      else {
        delete acc.bookingFooter;
        delete acc.bookingFooterText;
      }
    }
    if (patch.bookingFooterText !== undefined && typeof acc.bookingFooter === 'boolean') {
      const v = patch.bookingFooterText;
      if (typeof v === 'string' && v.trim() && v.trim().length <= 300 && v.includes('{url}')) acc.bookingFooterText = v.trim();
      else delete acc.bookingFooterText;
    }
    if (patch.color) acc.color = patch.color;
    if (patch.aliases) {
      // Validate the whole list before touching the account, so a typo cannot wipe existing aliases.
      const seen = new Set([acc.email.toLowerCase()]);
      const next = [];
      for (const a of patch.aliases) {
        const address = String(a.address || '').trim().toLowerCase();
        if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(address)) throw new Error(t('common.errors.invalidEmailValue', { address: a.address }));
        if (seen.has(address)) continue;
        seen.add(address);
        next.push({ address, name: a.name || '' });
      }
      acc.aliases = next;
      if (acc.defaultFrom && !this.identity(acc, acc.defaultFrom)) acc.defaultFrom = null;
    }
    if (patch.defaultFrom !== undefined) {
      if (patch.defaultFrom && !this.identity(acc, patch.defaultFrom)) throw new Error(t('errors.account.identity'));
      acc.defaultFrom = patch.defaultFrom ? patch.defaultFrom.toLowerCase() : null;
    }
    if (patch.makeDefault) {
      this.settings.defaultAccountId = id;
      this.persistSettings();
    }
    const serverChange = patch.imap || patch.smtp || patch.password;
    if ((serverChange || patch.caldav !== undefined) && acc.type === 'imap') {
      const next = {
        ...acc,
        imap: { ...acc.imap, ...(patch.imap || {}) },
        smtp: { ...acc.smtp, ...(patch.smtp || {}) },
        secret: patch.password && acc.auth !== 'oauth2' ? this.secrets.encrypt(patch.password) : acc.secret
      };
      if (serverChange) {
        const probe = new ImapAccount(next, this.credentials(next));
        await probe.verify();
      }
      // The calendar server is checked with the same username and password, and only saved when it answers.
      const calendar = patch.caldav !== undefined && acc.auth !== 'oauth2' ? await this.checkCalDav(acc, next, patch) : undefined;
      Object.assign(acc, next);
      if (calendar !== undefined) acc.caldav = calendar.caldav;
      if (serverChange) {
        const old = this.sessions.get(id);
        this.sessions.delete(id);
        if (old) old.close();
      }
      if (calendar && calendar.changed) this.emit('calendar-changed', acc);
    }
    // Whether the calendar server sends invitations ('provider', 'none'; null: as its DAV header says). It decides
    // whether guests can be added there, so the calendars are read again with it.
    if (patch.caldavInvites !== undefined && acc.type === 'imap') {
      const invites = patch.caldavInvites === 'provider' || patch.caldavInvites === 'none' ? patch.caldavInvites : null;
      if ((acc.caldavInvites || null) !== invites) {
        if (invites) acc.caldavInvites = invites;
        else delete acc.caldavInvites;
        if (acc.caldav) this.emit('calendar-changed', acc);
      }
    }
    this.persistAccounts();
    this.emit('updated');
    return this.publicAccount(acc);
  }

  // The CalDAV server from Server settings: { caldav, changed }, where caldav is null for none. The same address
  // with the same sign-in isn't checked again.
  async checkCalDav(acc, next, patch) {
    const text = String((patch.caldav && patch.caldav.url) || '').trim();
    if (!text) return { caldav: null, changed: Boolean(acc.caldav) };
    const url = serverUrl(text);
    const user = next.imap.user || acc.email;
    if (acc.caldav && acc.caldav.url === url && !patch.password && user === (acc.imap.user || acc.email)) return { caldav: acc.caldav, changed: false };
    const found = await discoverCalDav({ url, user, password: this.credentials(next), fetchImpl: this.fetch });
    return { caldav: { url: found.url, home: found.home }, changed: true };
  }

  // The calendar server from the setup form, checked as Server settings check it. The account doesn't exist yet, so
  // a refused sign-in can't send the user to its Server settings.
  async setupCalDav(acc, caldav) {
    try {
      return (await this.checkCalDav(acc, acc, { caldav })).caldav;
    } catch (err) {
      if (err && err.code === 'password') throw Object.assign(new Error(t('errors.caldav.setupSignIn')), { calendar: true, code: 'password' });
      throw err;
    }
  }

  // Where the account's calendars live, found again after the server moved them. url: the server it was found on; a
  // search that started before the user entered another server must not leave the old server's home behind.
  setCalDavHome(id, home, url) {
    const acc = this.accounts.find((a) => a.id === id);
    if (!acc || !acc.caldav || acc.caldav.url !== url || acc.caldav.home === home) return;
    acc.caldav = { ...acc.caldav, home };
    this.persistAccounts();
  }

  async removeAccount(id) {
    const acc = this.account(id);
    // Off the list before its connection closes, so a prefetch that goes on meanwhile doesn't open a new one.
    this.accounts = this.accounts.filter((a) => a !== acc);
    const s = this.sessions.get(id);
    this.sessions.delete(id);
    if (s) await s.close();
    this.caches.delete(id);
    for (const key of this.found.keys()) if (key.startsWith(`${id}\n`)) this.found.delete(key);
    this.tokens.delete(id);
    clearTimeout(this.saveTimers.get(id));
    this.saveTimers.delete(id);
    fs.rmSync(this.file('cache', `${id}.json`), { force: true });
    fs.rmSync(this.file('bodies', id), { recursive: true, force: true });
    if (acc.type === 'demo') fs.rmSync(this.file('demo-server.json'), { force: true });
    if (this.settings.defaultAccountId === id) {
      this.settings.defaultAccountId = this.accounts[0] ? this.accounts[0].id : null;
      this.persistSettings();
    }
    this.persistAccounts();
    this.emit('account-removed', acc);
    this.emit('updated');
  }

  // quiet: save without 'updated', for a change no list or count depends on (zoom), so the windows don't redraw.
  updateSettings(patch, { quiet = false } = {}) {
    // Calendar settings change only through calendarSettings(), which merges them per calendar.
    if (patch && Object.prototype.hasOwnProperty.call(patch, 'calendar')) {
      const { calendar, ...rest } = patch;
      patch = rest;
    }
    if (patch.signature !== undefined || patch.signatureHtml !== undefined) patch = { ...patch, ...signatureFields(patch) };
    this.settings = { ...this.settings, ...patch };
    normalizeSettings(this.settings);
    setLanguage(this.settings.language);
    this.persistSettings();
    if (!quiet) this.emit('updated');
    return this.settings;
  }

  // A change to settings.calendar. visible, availability and colors merge per calendar, and a calendar set to null
  // goes back to its default; other fields replace. What doesn't fit keeps the value the user had: a working day that
  // would end before it starts keeps both ends, so a page that saves each field on its own doesn't lose the other.
  // The result is normalized and saved, and 'calendar-settings' tells the calendar services before 'updated' tells the
  // windows.
  calendarSettings(patch) {
    const p = patch && typeof patch === 'object' && !Array.isArray(patch) ? patch : {};
    const current = this.settings.calendar;
    const next = { ...current };
    for (const [key, value] of Object.entries(p)) {
      if (CALENDAR_MAPS.includes(key)) {
        const merged = { ...current[key] };
        if (value && typeof value === 'object' && !Array.isArray(value)) {
          for (const [k, x] of Object.entries(value)) {
            if (x === null) delete merged[k];
            else if (CALENDAR_VALUES[key](x)) merged[k] = x;
          }
        }
        next[key] = merged;
      } else if (Object.prototype.hasOwnProperty.call(current, key)) next[key] = value;
    }
    this.settings = { ...this.settings, calendar: normalizeCalendar(next, current) };
    this.persistSettings();
    this.emit('calendar-settings', this.settings.calendar);
    this.emit('updated');
    return this.settings.calendar;
  }

  // ---------- sync ----------

  // Gmail has no archive folder; archiving there means moving to All Mail.
  archiveFolder(accountId) {
    return this.folderByRole(accountId, 'archive') || this.folderByRole(accountId, 'all');
  }

  folderByRole(accountId, role) {
    const cache = this.caches.get(accountId);
    return cache && cache.folders.find((f) => f.role === role);
  }

  folderInfo(accountId, folderPath) {
    const cache = this.caches.get(accountId);
    return cache && cache.folders.find((f) => f.path === folderPath);
  }

  // Syncs every account; rejects with the collected errors if any account failed.
  async syncAll() {
    const errors = [];
    await Promise.all(
      this.accounts.map((a) => this.syncAccount(a.id).catch((err) => errors.push(`${a.email}: ${friendlyError(err, a)}`)))
    );
    if (errors.length) throw new Error(errors.join('\n'));
  }

  // The sync the user asked for, of one account or of all. A failure reads the way the sidebar shows it for the
  // account (friendlyError), with the account so a refused password at Gmail or Yahoo points to its app password.
  async syncNow(accountId = null) {
    if (!accountId) return this.syncAll();
    try {
      await this.syncAccount(accountId);
    } catch (err) {
      throw new Error(friendlyError(err, this.accounts.find((a) => a.id === accountId)));
    }
  }

  pendingKey(accountId, folder, uid) {
    return `${accountId}\n${folder}\n${uid}`;
  }

  // Holds an optimistic change to one email until its IMAP command settles, so a sync that read server state just
  // before the command cannot undo it in the cache. Changes to the same email line up in the order they were made: a
  // sync sees them merged with the latest on top, and each one lets go of its own only, so an earlier change that
  // finishes never drops a later one (such as Undo of a Mark all as read that is still on its way).
  // The changes to one email share a base: the fields as the server last took them, taken from current (the email
  // before this change) the first time a change sets a field, and moved up by each change that reaches the server.
  // before: settles when the earlier changes to this email have. confirm(): the server took this change.
  // rollback(): what a failure puts back, the base of each field no later change has set since.
  holdPending(accountId, folder, uid, patch, current = null) {
    const key = this.pendingKey(accountId, folder, uid);
    const earlier = this.pending.get(key) || [];
    const base = earlier.length ? earlier[0].base : {};
    if (current) for (const k of Object.keys(patch)) if (!(k in base)) base[k] = Boolean(current[k]);
    let settle;
    const hold = { patch, base, settled: new Promise((resolve) => (settle = resolve)) };
    this.pending.set(key, [...earlier, hold]);
    return {
      before: Promise.all(earlier.map((h) => h.settled)),
      confirm: () => {
        for (const k of Object.keys(patch)) if (k in base) base[k] = patch[k];
      },
      rollback: () => {
        const all = this.pending.get(key) || [];
        const later = all.slice(all.indexOf(hold) + 1);
        const back = {};
        for (const k of Object.keys(patch)) if (k in base && !later.some((h) => k in h.patch)) back[k] = base[k];
        return back;
      },
      release: () => {
        settle();
        const rest = (this.pending.get(key) || []).filter((h) => h !== hold);
        if (rest.length) this.pending.set(key, rest);
        else {
          // Nothing more on its way for this email: what its changes did to the count stays until a sync counts again.
          this.pending.delete(key);
          this.recounts.delete(key);
        }
      },
    };
  }

  // Runs op once the earlier changes to this email have reached the server, so a move or delete never overtakes a
  // flag write still on its way to the old folder and uid. recount: the email's record of the inbox's count of older
  // unread mail, as removeFromCache() gives it; it stays in this.recounts until the email's changes have settled.
  async withPending(acc, folder, uid, patch, op, recount = null) {
    const hold = this.holdPending(acc.id, folder, uid, patch);
    if (recount) this.recounts.set(recount.key, recount);
    try {
      await hold.before;
      return await op();
    } finally {
      hold.release();
    }
  }

  applyPending(accountId, folder, messages) {
    if (!this.pending.size) return messages;
    const out = [];
    for (const m of messages) {
      const holds = this.pending.get(this.pendingKey(accountId, folder, m.uid));
      const p = holds && Object.assign({}, ...holds.map((h) => h.patch));
      if (!p) out.push(m);
      else if (!p.removed) out.push({ ...m, ...p });
    }
    return out;
  }

  syncAccount(id, extraFolders = []) {
    if (this.syncs.has(id)) return this.syncs.get(id);
    const started = Date.now();
    this.syncStarts.set(id, started);
    const report = { folders: 0, failed: 0, fresh: 0 };
    const job = this.runSync(id, extraFolders, report)
      .then(
        (fresh) => {
          this.logSync(id, started, report, null);
          return fresh;
        },
        (err) => {
          this.logSync(id, started, report, err);
          throw err;
        }
      )
      .finally(() => {
        this.syncs.delete(id);
        this.emit('updated');
      });
    this.syncs.set(id, job);
    this.emit('updated');
    return job;
  }

  // Syncs, in the background, each account whose last sync started at least age ms ago or not at all since Rukoo
  // started, and that isn't syncing now; for the main window coming forward (main.js syncOnReturn()). The ids it
  // started. A sync goes through the inbox first, so that is where new mail shows first.
  syncStale(age) {
    const now = Date.now();
    const started = [];
    for (const a of this.accounts) {
      if (this.syncs.has(a.id) || now - (this.syncStarts.get(a.id) || 0) < age) continue;
      this.syncAccount(a.id).catch(() => {});
      started.push(a.id);
    }
    return started;
  }

  // One line per account per sync for the log in the profile (diaglog.js): when it started, how long it took, how many
  // folders it went through, how many new emails it found, and what went wrong as a code. The account by the start of
  // its id, and nothing that names it or its mail.
  logSync(id, started, report, err) {
    if (!this.log) return;
    const acc = this.accounts.find((a) => a.id === id);
    const kind = !acc ? 'removed' : acc.type === 'demo' ? 'demo' : `${acc.provider || 'imap'}/${acc.auth === 'oauth2' ? 'oauth' : 'password'}`;
    const parts = [`account=${String(id).slice(0, 8)}`, kind, `start=${new Date(started).toISOString()}`, `ms=${Date.now() - started}`, `folders=${report.folders}`, `new=${report.fresh}`];
    if (report.failed) parts.push(`failedFolders=${report.failed}`);
    if (err) parts.push(`error=${errorCode(err)}`);
    try {
      this.log(err ? 'warn' : 'info', 'sync', parts.join(' '));
    } catch (_) {
      // The log never stops a sync.
    }
  }

  // report: filled in for the log (logSync()).
  async runSync(id, extraFolders, report = {}) {
    const acc = this.account(id);
    const cache = this.caches.get(id);
    const fresh = [];
    // Pick up Gmail aliases once for Google accounts; failures here never block mail sync.
    if (acc.auth === 'oauth2' && !acc.aliasesFetchedAt) await this.fetchGmailAliases(id).catch(() => {});
    // An account removed during that wait must not get a new session; see removeAccount().
    if (!this.accounts.includes(acc)) return fresh;
    let failed = null;
    try {
      // The session as it is after that wait: signing in again replaces it, and the old one takes no more commands.
      cache.folders = await this.session(acc).listFolders({ background: true });
      const targets = cache.folders.filter((f) => SYNC_ROLES.includes(f.role)).map((f) => f.path);
      const archive = this.archiveFolder(id);
      if (archive && !targets.includes(archive.path)) targets.push(archive.path);
      // Keep user folders that were opened before in sync too.
      for (const p of [...extraFolders, ...Object.keys(cache.boxes)]) {
        if (!targets.includes(p) && cache.folders.some((f) => f.path === p)) targets.push(p);
      }
      report.folders = targets.length;
      for (const p of targets) {
        try {
          fresh.push(...(await this.syncFolderInto(acc, p, { background: true })));
        } catch (err) {
          // A folder that fails leaves the others to sync. Not once the engine closed or the account went: the next
          // folder would open a new session for it.
          if (this.closed || !this.accounts.includes(acc)) throw err;
          failed = failed || err;
          report.failed = (report.failed || 0) + 1;
        }
      }
      report.fresh = fresh.length;
      for (const p of Object.keys(cache.boxes)) {
        if (!cache.folders.some((f) => f.path === p)) delete cache.boxes[p];
      }
      // The last sync stays the last one that got through every folder.
      if (!failed) cache.lastSync = Date.now();
      cache.error = failed ? friendlyError(failed, acc) : null;
    } catch (err) {
      cache.error = friendlyError(err, acc);
      this.changed(id);
      throw err;
    }
    this.changed(id);
    // New mail in the folders that did sync is announced now: the next sync no longer sees it as new.
    if (fresh.length) this.emit('new-mail', fresh);
    this.prefetch(id);
    if (failed) throw failed;
    return fresh;
  }

  // How many of a folder's newest messages the cache keeps: the folder's limit, plus what Load older added (box.extra).
  windowOf(accountId, folderPath) {
    const cache = this.caches.get(accountId);
    const info = (cache && cache.folders.find((f) => f.path === folderPath)) || {};
    const box = cache && cache.boxes[folderPath];
    return (info.role === 'inbox' ? this.limits.inbox : this.limits.default) + ((box && box.extra) || 0);
  }

  // background: a sync nobody waits for, which lets the user's own commands go first on the connection.
  async syncFolderInto(acc, folderPath, { background = false } = {}) {
    const cache = this.caches.get(acc.id);
    const info = cache.folders.find((f) => f.path === folderPath) || { path: folderPath };
    const prev = cache.boxes[folderPath];
    const known = new Map();
    if (prev) for (const m of prev.messages) known.set(m.uid, m);
    // The older mail a server search found there, as the command runs, whose flags the sync asks for too.
    let asked = [];
    const res = await this.session(acc).syncFolder(folderPath, {
      // Read when the command runs: Load older can widen the window while this sync waits for the connection.
      limit: () => this.windowOf(acc.id, folderPath),
      // With the older mail whose change of the unread count is on its way, a move included; see carryRecounts().
      keep: () => {
        const entry = this.foundIn(acc.id, folderPath);
        asked = entry ? [...entry.hits.keys()] : [];
        const changing = [...this.recounts.values()].filter((r) => r.accountId === acc.id && r.folder === folderPath).map((r) => r.uid);
        return [...new Set([...asked, ...changing])];
      },
      known,
      uidValidity: prev ? prev.uidValidity : null,
      background,
      // What inbox categories sort by (categories/).
      signals: info.role === 'inbox',
      gmailTabs: info.role === 'inbox' && acc.provider === 'google',
      olderUnseen: info.role === 'inbox'
    });
    const sameValidity = prev && prev.uidValidity === res.uidValidity;
    const maxKnown = sameValidity ? Math.max(0, ...prev.messages.map((m) => m.uid)) : Infinity;
    // What Load older added, as it is now: it may have changed while this sync ran.
    const extra = (cache.boxes[folderPath] || {}).extra || 0;
    cache.boxes[folderPath] = {
      uidValidity: res.uidValidity,
      exists: res.exists,
      // The inbox's unread mail older than messages, as the server counted it; null when it couldn't. See olderUnread().
      ...(info.role === 'inbox' ? { olderUnseen: Number.isInteger(res.olderUnseen) ? res.olderUnseen : null } : {}),
      ...(extra ? { extra } : {}),
      syncedAt: Date.now(),
      messages: this.applyPending(acc.id, folderPath, res.messages)
    };
    this.pruneFound(acc.id, folderPath);
    this.refreshFound(acc.id, folderPath, asked, res.kept);
    this.carryRecounts(acc.id, folderPath, res.kept);
    if (!sameValidity && prev) {
      fs.rmSync(this.file('bodies', acc.id), { recursive: true, force: true });
      for (const [key, r] of this.undoable) if (r.accountId === acc.id && r.destination === folderPath) this.undoable.delete(key);
    }
    // Only report mail that arrived after an earlier sync, never the initial download.
    if (info.role !== 'inbox' || !prev) return [];
    return res.messages
      .filter((m) => m.uid > maxKnown && m.unread && !this.isSpam(m.from))
      .map((m) => this.publicMessage(acc, folderPath, m));
  }

  async openFolder(accountId, folderPath) {
    const cache = this.caches.get(accountId);
    const box = cache && cache.boxes[folderPath];
    if (box && Date.now() - box.syncedAt < 60000) return;
    const acc = this.account(accountId);
    await this.syncFolderAndNotify(acc, folderPath);
  }

  // For syncs outside runSync (after a move, opening a folder): still report new mail.
  async syncFolderAndNotify(acc, folderPath) {
    const fresh = await this.syncFolderInto(acc, folderPath);
    this.changed(acc.id);
    if (fresh.length) this.emit('new-mail', fresh);
  }

  // ---------- listing ----------

  isSpam(from) {
    return this.settings.spam.includes(lowerAddr(from));
  }

  isVip(from) {
    return this.settings.vips.includes(lowerAddr(from));
  }

  publicMessage(acc, folderPath, m) {
    const info = this.folderInfo(acc.id, folderPath);
    return {
      id: encodeId(acc.id, folderPath, m.uid),
      accountId: acc.id,
      accountColor: acc.color,
      folder: folderPath,
      role: (info && info.role) || null,
      uid: m.uid,
      // One email can be in several folders under several ids (Inbox and Sent of a mail to yourself, two Gmail
      // labels); by this the chat panel adds it to a message once.
      messageId: m.messageId || null,
      subject: m.subject,
      from: m.from,
      to: m.to,
      cc: m.cc,
      date: m.date,
      preview: m.preview,
      unread: m.unread,
      starred: m.starred,
      answered: m.answered,
      hasAttachments: m.hasAttachments,
      vip: this.isVip(m.from),
      // The inbox category (categories/), set by main once categories are on; null elsewhere.
      category: this.categoryOf && info && info.role === 'inbox' ? this.categoryOf(acc, m) : null
    };
  }

  scopeAccounts(scope) {
    return !scope || scope === 'all' ? this.accounts : this.accounts.filter((a) => a.id === scope);
  }

  // query: also the older mail the server search found for it (searchServer()), marked older.
  collect(scope, predicate, { query = null } = {}) {
    const out = [];
    for (const acc of this.scopeAccounts(scope)) {
      const cache = this.caches.get(acc.id);
      if (!cache) continue;
      const hits = query ? (p) => this.olderHits(acc.id, p, query) : null;
      for (const [folderPath, box] of Object.entries(cache.boxes)) {
        const info = cache.folders.find((f) => f.path === folderPath) || { path: folderPath };
        // Gmail's All Mail also holds every message of the other folders. Normally its copies of those
        // are hidden; the archive view only drops what is still in a system folder, so archived mail
        // with a label stays visible there.
        const elsewhere = info.role === 'all' ? this.messageIdsOutside(cache, folderPath, predicate.archive ? SYSTEM_ROLES : null, hits) : null;
        const shown = (m) => !(elsewhere && m.messageId && elsewhere.has(m.messageId)) && predicate(info, m);
        for (const m of box.messages) if (shown(m)) out.push(this.publicMessage(acc, folderPath, m));
        if (hits) for (const m of hits(folderPath)) if (shown(m)) out.push({ ...this.publicMessage(acc, folderPath, m), older: true });
      }
    }
    return out;
  }

  // hits: for a search, the older mail the server found in a folder, which counts as being there too.
  messageIdsOutside(cache, folderPath, roles = null, hits = null) {
    const ids = new Set();
    for (const [p, box] of Object.entries(cache.boxes)) {
      if (p === folderPath) continue;
      if (roles && !roles.includes((cache.folders.find((f) => f.path === p) || {}).role)) continue;
      for (const m of box.messages) if (m.messageId) ids.add(m.messageId);
      if (hits) for (const m of hits(p)) if (m.messageId) ids.add(m.messageId);
    }
    return ids;
  }

  viewPredicate(view, folder) {
    const inbox = (info, m) => info.role === 'inbox' && !this.isSpam(m.from);
    switch (view) {
      case 'inbox':
        return inbox;
      case 'unread':
        return (info, m) => inbox(info, m) && m.unread;
      case 'vip':
        return (info, m) => inbox(info, m) && this.isVip(m.from);
      case 'starred':
        return (info, m) => m.starred && info.role !== 'trash' && info.role !== 'junk';
      case 'folder':
        return (info) => info.path === folder;
      case 'everything':
        return () => true;
      case 'archive':
        return Object.assign((info) => info.role === 'archive' || info.role === 'all', { archive: true });
      default:
        if (VIEW_ROLES[view]) return (info) => info.role === VIEW_ROLES[view];
        return () => false;
    }
  }

  // A search (query) matches the subject, preview, sender and recipients of the cached mail, and lists the older mail
  // the server search found for it as well; the server matched that, which can be on text the cache doesn't have.
  listMessages({ scope = 'all', view = 'inbox', folder = null, query = '', sort } = {}) {
    const q = String(query || '').trim().toLowerCase();
    let list =
      view === 'saved'
        ? this.saved.filter((s) => scope === 'all' || s.accountId === scope).map((s) => ({ ...s, unread: false }))
        : this.collect(scope, this.viewPredicate(view, folder), { query: q || null });
    if (q) {
      list = list.filter(
        (m) =>
          m.older ||
          [m.subject, m.preview, m.from && m.from.name, m.from && m.from.address, ...[...(m.to || []), ...(m.cc || [])].map((t) => `${t.name} ${t.address}`)]
            .join(' ')
            .toLowerCase()
            .includes(q)
      );
    }
    const order = sort || this.settings.sort;
    // Mail with the same time goes in the order it arrived in its folder, by uid.
    const byDate = (a, b) => b.date - a.date || (b.uid || 0) - (a.uid || 0);
    if (order === 'date-asc') list.sort((a, b) => byDate(b, a));
    else if (order === 'unread') list.sort((a, b) => Number(b.unread) - Number(a.unread) || byDate(a, b));
    else if (order === 'sender') {
      list.sort((a, b) => (a.from.name || a.from.address).localeCompare(b.from.name || b.from.address, getLocale()) || byDate(a, b));
    } else list.sort(byDate);
    return list;
  }

  // Unread inbox mail older than the newest messages the cache keeps (LIMITS), which the server counted at the last
  // sync; folderPath: of that folder only. Mail from senders on the spam list is in it too: it can't be told apart
  // without its headers. Reading a message in the cache counts at once; one older than that counts from the next sync.
  olderUnread(scope, folderPath = null) {
    let n = 0;
    for (const acc of this.scopeAccounts(scope)) {
      const cache = this.caches.get(acc.id);
      if (!cache) continue;
      for (const f of cache.folders) {
        if (f.role !== 'inbox' || (folderPath !== null && f.path !== folderPath)) continue;
        const box = cache.boxes[f.path];
        if (box && Number.isInteger(box.olderUnseen)) n += box.olderUnseen;
      }
    }
    return n;
  }

  // Unread mail in Inbox: without the inbox categories Inbox doesn't show (set by main, categories/), with the older
  // unread mail, which has no category.
  inboxUnread(scope, hidden = this.hiddenCategories ? this.hiddenCategories() : new Set()) {
    return this.collect(scope, this.viewPredicate('unread')).filter((m) => !hidden.has(m.category)).length + this.olderUnread(scope);
  }

  // Unread mail in the inboxes of all accounts: the taskbar badge's "unread" count and the tray's tooltip.
  unreadCount() {
    return this.inboxUnread('all');
  }

  // How many of the emails in ids (a Set of message ids) still count as new: in an inbox, unread, and in a category
  // Inbox shows. The taskbar badge's count of new mail. An id that is gone from the cache, because the email moved or
  // was deleted, leaves ids.
  newCount(ids) {
    const hidden = this.hiddenCategories ? this.hiddenCategories() : new Set();
    let n = 0;
    for (const id of ids) {
      let found;
      try {
        found = this.locate(id);
      } catch (_) {
        ids.delete(id);
        continue;
      }
      const { acc, msg, folder } = found;
      if (!msg.unread || this.isSpam(msg.from) || (this.folderInfo(acc.id, folder) || {}).role !== 'inbox') continue;
      if (hidden.size && this.categoryOf && hidden.has(this.categoryOf(acc, msg))) continue;
      n++;
    }
    return n;
  }

  counts(scope = 'all') {
    const res = { accounts: {}, views: {} };
    const hidden = this.hiddenCategories ? this.hiddenCategories() : new Set();
    for (const acc of this.accounts) {
      res.accounts[acc.id] = this.inboxUnread(acc.id, hidden);
    }
    res.all = Object.values(res.accounts).reduce((a, b) => a + b, 0);
    // The list only holds the newest inboxLimit messages of each inbox, so its header names the older unread mail apart.
    res.olderUnread = this.olderUnread(scope);
    res.inboxLimit = this.limits.inbox;
    const count = (view, onlyUnread) =>
      this.collect(scope, this.viewPredicate(view)).filter((m) => !onlyUnread || m.unread).length;
    // Only the inbox and Unread add the older mail: VIP picks its mail by sender, and other folders count their cache.
    // Unread and VIP still count the categories Inbox doesn't show.
    res.views = {
      inbox: this.inboxUnread(scope, hidden),
      unread: count('unread', true) + res.olderUnread,
      vip: count('vip', true),
      starred: count('starred'),
      saved: this.saved.filter((s) => scope === 'all' || s.accountId === scope).length,
      drafts: count('drafts'),
      sent: count('sent'),
      trash: count('trash'),
      junk: count('junk', true),
      archive: count('archive', true)
    };
    res.folders = {};
    if (scope !== 'all') {
      const cache = this.caches.get(scope);
      for (const [p, box] of Object.entries((cache && cache.boxes) || {})) {
        res.folders[p] = box.messages.filter((m) => m.unread).length + this.olderUnread(scope, p);
      }
    }
    return res;
  }

  // ---------- older mail ----------

  // The synced folders a view lists in an account, as viewPredicate() picks them; 'everything' is a search of all
  // folders. The cache's order, which starts with the inbox.
  viewFolders(accountId, view, folder = null) {
    const cache = this.caches.get(accountId);
    if (!cache) return [];
    const role = (p) => (cache.folders.find((f) => f.path === p) || {}).role;
    const paths = Object.keys(cache.boxes);
    switch (view) {
      case 'inbox':
      case 'unread':
      case 'vip':
        return paths.filter((p) => role(p) === 'inbox');
      case 'folder':
        return paths.filter((p) => p === folder);
      case 'starred':
        return paths.filter((p) => role(p) !== 'trash' && role(p) !== 'junk');
      case 'everything':
        return paths;
      case 'archive':
        return paths.filter((p) => role(p) === 'archive' || role(p) === 'all');
      default:
        return VIEW_ROLES[view] ? paths.filter((p) => role(p) === VIEW_ROLES[view]) : [];
    }
  }

  // How many messages of a folder are on the server below the cache's window, as the last sync counted them.
  olderOnServer(accountId, folderPath) {
    const box = (this.caches.get(accountId) || { boxes: {} }).boxes[folderPath];
    return box ? Math.max(0, (box.exists || 0) - this.windowOf(accountId, folderPath)) : 0;
  }

  // The folders of a view that Load older can widen, with how many messages each has on the server beyond the cache.
  olderFolders(accountId, view, folder = null) {
    if (!OLDER_VIEWS.includes(view)) return [];
    const cache = this.caches.get(accountId);
    return this.viewFolders(accountId, view, folder)
      .filter((p) => (cache.boxes[p].extra || 0) < this.limits.extra)
      .map((p) => ({ path: p, more: this.olderOnServer(accountId, p) }))
      .filter((f) => f.more > 0);
  }

  // How many older messages Load older can still bring into a view (the list's scope, view and folder).
  olderCount({ scope = 'all', view = 'inbox', folder = null } = {}) {
    let n = 0;
    for (const acc of this.scopeAccounts(scope)) for (const f of this.olderFolders(acc.id, view, folder)) n += f.more;
    return n;
  }

  // For the list on screen: more, what Load older can bring in (olderCount()); server, how many messages of the
  // folders a search there covers are on the server below the cache, so a search of the server can find any.
  olderState(opts = {}) {
    const { scope = 'all', view = 'inbox', folder = null } = opts;
    let server = 0;
    if (view !== 'saved') {
      for (const acc of this.scopeAccounts(scope)) {
        for (const p of this.viewFolders(acc.id, view, folder)) server += this.olderOnServer(acc.id, p);
      }
    }
    return { more: this.olderCount(opts), server };
  }

  // Load older at the end of the list: widens the window of each folder the view lists by the next batch of older
  // messages and syncs it. The wider window stays, also over restarts, up to LIMITS.extra beyond the folder's limit;
  // new mail moves it along, like the window it widens. A folder that fails keeps its window; the error names its
  // account. A second call for the same view while one runs waits for that one.
  loadOlder({ scope = 'all', view = 'inbox', folder = null } = {}) {
    const key = JSON.stringify([scope, view, folder]);
    if (this.loading.has(key)) return this.loading.get(key);
    const job = (async () => {
      const errors = [];
      await Promise.all(
        this.scopeAccounts(scope).map(async (acc) => {
          for (const { path: p } of this.olderFolders(acc.id, view, folder)) {
            const box = this.caches.get(acc.id).boxes[p];
            const before = box.extra || 0;
            const extra = Math.min(this.limits.extra, before + this.limits.batch);
            box.extra = extra;
            try {
              await this.syncFolderAndNotify(acc, p);
            } catch (err) {
              const now = (this.caches.get(acc.id) || { boxes: {} }).boxes[p];
              if (now && now.extra === extra) now.extra = before;
              if (this.closed || !this.accounts.includes(acc)) return;
              errors.push(`${acc.email}: ${friendlyError(err, acc)}`);
              // The account's next folder would fail the same way.
              return;
            }
          }
        })
      );
      if (errors.length) throw new Error(errors.join('\n'));
    })().finally(() => this.loading.delete(key));
    this.loading.set(key, job);
    return job;
  }

  foundKey(accountId, folderPath) {
    return `${accountId}\n${folderPath}`;
  }

  // The older mail found in a folder, while it belongs to the uids the folder has now; null for none.
  foundIn(accountId, folderPath) {
    const entry = this.found.get(this.foundKey(accountId, folderPath));
    const box = (this.caches.get(accountId) || { boxes: {} }).boxes[folderPath];
    return entry && box && entry.uidValidity === box.uidValidity ? entry : null;
  }

  // The older messages the server search found in a folder for query (lower case, trimmed).
  olderHits(accountId, folderPath, query) {
    const entry = this.foundIn(accountId, folderPath);
    if (!entry) return [];
    const out = [];
    for (const h of entry.hits.values()) if (h.query === query) out.push(h.msg);
    return out;
  }

  // An older message the server search found, by its uid; null when there is none.
  olderHit(accountId, folderPath, uid) {
    const entry = this.foundIn(accountId, folderPath);
    const hit = entry && entry.hits.get(Number(uid));
    return hit ? hit.msg : null;
  }

  // After a sync of a folder: older mail found there that is now in the window, or gone from the server, leaves the
  // found mail. All of it goes when the folder's uids were given out again.
  pruneFound(accountId, folderPath) {
    const key = this.foundKey(accountId, folderPath);
    const entry = this.found.get(key);
    if (!entry) return;
    const box = this.caches.get(accountId).boxes[folderPath];
    if (!box || box.uidValidity !== entry.uidValidity || !box.exists) {
      this.found.delete(key);
      return;
    }
    // Every message from the oldest the window has upward is in the window.
    if (!box.messages.length) return;
    const first = Math.min(...box.messages.map((m) => m.uid));
    for (const uid of [...entry.hits.keys()]) if (uid >= first) entry.hits.delete(uid);
  }

  // After a sync of a folder, for the older mail found there that the sync asked about (asked, their uids): what
  // another device read, starred or deleted shows here too. kept has the flags the server has now, without the
  // messages it no longer has; an optimistic change still on its way to the server goes over them. Nothing changes
  // when the server didn't answer (kept not a list). The same object stays, which an open message or an undo may hold.
  // Older inbox mail needs no recount: the same sync counted the older unread mail on the server again, before any
  // change still on its way, which carryRecounts() adds.
  refreshFound(accountId, folderPath, asked, kept) {
    const entry = this.foundIn(accountId, folderPath);
    if (!entry || !asked.length || !Array.isArray(kept)) return;
    const now = new Map(this.applyPending(accountId, folderPath, kept).map((r) => [r.uid, r]));
    for (const uid of asked) {
      const hit = entry.hits.get(uid);
      if (!hit) continue;
      const row = now.get(uid);
      if (!row) entry.hits.delete(uid);
      else for (const k of ['unread', 'starred', 'answered']) if (k in row) hit.msg[k] = row[k];
    }
  }

  // Searches the server for mail older than the cache holds, in the folders a search covers: the list's scope, view
  // and folder as listMessages takes them, where 'everything' searches all synced folders. listMessages lists what it
  // finds for the same query, until a sync finds it in the window or another search of that folder replaces it.
  // Folders the cache holds entirely aren't searched, nor are saved emails. One call searches up to SEARCH.folders
  // folders per account, so a list of results comes back in time; remaining says how many are left. resume: search the
  // next ones of the same search (scope, view, folder and query) instead of starting again from the first. Returns
  // more, how many matches beyond the newest SEARCH.hits per folder didn't come back, and errors, one line per account
  // whose server couldn't be searched; that account's other folders aren't searched.
  async searchServer({ scope = 'all', view = 'inbox', folder = null, query = '', resume = false } = {}) {
    const text = String(query || '').trim().slice(0, 500);
    const key = text.toLowerCase();
    const out = { more: 0, errors: [], remaining: 0 };
    if (key.length < this.search.min || view === 'saved') return out;
    // The views that list mail by a flag ask the server for it too.
    const criteria = view === 'unread' ? { seen: false } : view === 'starred' ? { flagged: true } : {};
    const id = JSON.stringify([scope, view, folder, key]);
    let run = this.searching;
    if (!resume || !run || run.id !== id) {
      // The folders to search, by account, in the order viewFolders() gives them: the inbox first.
      run = { id, queue: new Map() };
      for (const acc of this.scopeAccounts(scope)) {
        if (this.caches.get(acc.id)) run.queue.set(acc.id, this.viewFolders(acc.id, view, folder).filter((p) => this.olderOnServer(acc.id, p) > 0));
      }
      this.searching = run;
    }
    await Promise.all(
      [...run.queue].map(async ([accountId, paths]) => {
        const batch = paths.splice(0, this.search.folders);
        const acc = this.accounts.find((a) => a.id === accountId);
        const cache = acc && this.caches.get(acc.id);
        if (!cache) {
          paths.length = 0;
          return;
        }
        for (const p of batch) {
          // A newer search started: this one stops, and what it finds would replace that one's results.
          if (this.searching !== run) return;
          const box = cache.boxes[p];
          // Gone since the search started, or all of it in the cache now.
          if (!box || this.olderOnServer(acc.id, p) === 0) continue;
          const prior = this.foundIn(acc.id, p);
          const known = new Map(prior ? [...prior.hits].map(([uid, h]) => [uid, h.msg]) : []);
          const info = cache.folders.find((f) => f.path === p) || {};
          let res;
          try {
            res = await this.session(acc).searchFolder(p, text, {
              below: box.messages.length ? Math.min(...box.messages.map((m) => m.uid)) : null,
              limit: this.search.hits,
              known,
              uidValidity: box.uidValidity,
              signals: info.role === 'inbox',
              criteria,
              gmail: info.role !== 'trash' && info.role !== 'junk'
            });
          } catch (err) {
            // The account's next folder would fail the same way.
            paths.length = 0;
            if (this.searching !== run || this.closed || !this.accounts.includes(acc)) return;
            out.errors.push(`${acc.email}: ${friendlyError(err, acc)}`);
            return;
          }
          if (this.searching !== run) return;
          this.keepHits(acc, p, key, res);
          out.more += Math.max(0, res.total - res.messages.length);
        }
      })
    );
    for (const paths of run.queue.values()) out.remaining += paths.length;
    return out;
  }

  // Keeps what a server search of a folder found for query (lower case), unless the folder changed meanwhile: its
  // uids given out again, or the account removed. Mail the window has now stays out. A message found before keeps its
  // object, which an open message or an undo may hold, with the server's flags.
  keepHits(acc, folderPath, query, res) {
    const box = this.accounts.includes(acc) && (this.caches.get(acc.id) || { boxes: {} }).boxes[folderPath];
    if (!box || box.uidValidity !== res.uidValidity) return;
    const key = this.foundKey(acc.id, folderPath);
    let entry = this.found.get(key);
    if (!entry || entry.uidValidity !== res.uidValidity) {
      entry = { uidValidity: res.uidValidity, hits: new Map() };
      this.found.set(key, entry);
    }
    // What matched this query before and doesn't now no longer shows for it.
    for (const h of entry.hits.values()) if (h.query === query) h.query = null;
    const first = box.messages.length ? Math.min(...box.messages.map((m) => m.uid)) : Infinity;
    for (const m of this.applyPending(acc.id, folderPath, res.messages)) {
      if (m.uid >= first) continue;
      const prior = entry.hits.get(m.uid);
      const hit = prior || { msg: m, query };
      if (prior) {
        Object.assign(prior.msg, m);
        prior.query = query;
        entry.hits.delete(m.uid);
      }
      entry.hits.set(m.uid, hit);
    }
    // The oldest kept go first; what was just found is at the end.
    for (const uid of [...entry.hits.keys()]) {
      if (entry.hits.size <= this.search.kept) break;
      entry.hits.delete(uid);
    }
  }

  // ---------- messages ----------

  // older: the message is older mail the server search found, outside the cache's window.
  locate(id) {
    const { accountId, folder, uid } = decodeId(id);
    const acc = this.account(accountId);
    const cache = this.caches.get(accountId);
    const box = cache.boxes[folder];
    const cached = box && box.messages.find((m) => m.uid === uid);
    const msg = cached || this.olderHit(accountId, folder, uid);
    if (!msg) throw new Error(t('errors.message.missing'));
    return { acc, cache, box, msg, folder, uid, older: !cached };
  }

  async rawSource(id) {
    if (String(id).startsWith('saved:')) {
      const item = this.saved.find((s) => s.id === id);
      if (!item) throw new Error(t('errors.message.savedMissing'));
      return fs.readFileSync(this.file('saved', item.file));
    }
    const { acc, box, folder, uid } = this.locate(id);
    const file = this.bodyFile(acc, box, folder, uid);
    if (fs.existsSync(file)) return fs.readFileSync(file);
    return this.download(acc, box, folder, uid, file, false);
  }

  // Where a message's raw source is kept once downloaded.
  bodyFile(acc, box, folder, uid) {
    return this.file('bodies', acc.id, `${sha(`${folder}|${box.uidValidity}|${uid}`)}.eml`);
  }

  // One download per message at a time. A message the user opens while it waits as background work is fetched
  // again in the user's turn, rather than behind the rest of the prefetch. What waits for the background download
  // then has the message as soon as the user's download brings it.
  download(acc, box, folder, uid, file, background) {
    const running = this.downloads.get(file);
    if (running && (background || !running.background)) return running.raw;
    const entry = { background };
    entry.raw = new Promise((resolve, reject) => (entry.settle = { resolve, reject }));
    if (running) entry.raw.then(running.settle.resolve, () => {});
    this.session(acc)
      .fetchSource(folder, uid, { background })
      .then((raw) => {
        // Not kept for an account removed in the meantime, or a folder whose uids were reassigned: their bodies
        // were just deleted. A body that can't be saved is still shown.
        const now = this.caches.get(acc.id);
        if (this.accounts.includes(acc) && now && now.boxes[folder] && now.boxes[folder].uidValidity === box.uidValidity) {
          try {
            fs.mkdirSync(path.dirname(file), { recursive: true });
            const tmp = `${file}.${crypto.randomUUID()}.tmp`;
            fs.writeFileSync(tmp, raw);
            fs.renameSync(tmp, file);
          } catch (_) {
            // Opened again later, it is downloaded again.
          }
        }
        return raw;
      })
      .finally(() => {
        if (this.downloads.get(file) === entry) this.downloads.delete(file);
      })
      .then(entry.settle.resolve, entry.settle.reject);
    this.downloads.set(file, entry);
    return entry.raw;
  }

  // Resolves once the whole message is on disk, so the next getMessage reads it from there: at once when it is, or
  // when its download lands. That download is background work, or the user's with now (an attachment they opened).
  // false when it came in but couldn't be kept, on a full disk for example: getMessage without fast then reads it whole
  // from the server again, and with fast still gives the text alone.
  async downloaded(id, { now = false } = {}) {
    if (String(id).startsWith('saved:')) return true;
    const { acc, box, folder, uid } = this.locate(id);
    const file = this.bodyFile(acc, box, folder, uid);
    if (!fs.existsSync(file)) await (now ? this.rawSource(id) : this.download(acc, box, folder, uid, file, true));
    return fs.existsSync(file);
  }

  // A large message the user opens before it is downloaded: its text, invitation and inline pictures, with the
  // attachments left empty (ImapAccount.fetchReadable), so the reader shows it without waiting for them. The whole
  // message follows as background work; downloaded() says when it is in. null to read the whole message now: one that
  // is saved, small, on disk or on its way in the user's turn, or whose account or server can't split it.
  async readableSource(id) {
    if (String(id).startsWith('saved:')) return null;
    const { acc, box, msg, folder, uid } = this.locate(id);
    const file = this.bodyFile(acc, box, folder, uid);
    const session = this.session(acc);
    const running = this.downloads.get(file);
    if ((msg.size || 0) <= PREFETCH.maxBytes || typeof session.fetchReadable !== 'function' || (running && !running.background) || fs.existsSync(file)) return null;
    let source = null;
    try {
      source = await session.fetchReadable(folder, uid);
    } catch (_) {
      // A server that doesn't give the parts: the whole message, as before.
    }
    if (!source) return null;
    this.download(acc, box, folder, uid, file, true).catch(() => {});
    return source;
  }

  // Downloads the newest inbox messages ahead of time, so opening one reads it from disk. The fetches run as
  // background work, behind anything the user does on the connection, and mark nothing read (imapflow fetches with
  // PEEK). Bodies of messages that left the synced folders go first, so the cache stays as large as the lists.
  async prefetch(accountId) {
    if (this.closed || this.prefetching.has(accountId)) return;
    this.prefetching.add(accountId);
    try {
      const acc = this.account(accountId);
      const cache = this.caches.get(accountId);
      this.pruneBodies(acc, cache);
      const inbox = cache.folders.find((f) => f.role === 'inbox');
      const box = inbox && cache.boxes[inbox.path];
      if (!box) return;
      const newest = [...box.messages].sort((a, b) => b.date - a.date).slice(0, PREFETCH.count);
      for (const m of newest) {
        if (this.closed || !this.accounts.includes(acc)) return;
        const file = this.bodyFile(acc, box, inbox.path, m.uid);
        if (m.size > PREFETCH.maxBytes || fs.existsSync(file)) continue;
        await this.download(acc, box, inbox.path, m.uid, file, true);
      }
    } catch (_) {
      // Offline, signed out or the message is gone: the next sync tries again, and opening a message still works.
    } finally {
      this.prefetching.delete(accountId);
    }
  }

  // Removes downloaded bodies of messages that are in none of the account's synced folders any more.
  pruneBodies(acc, cache) {
    const dir = this.file('bodies', acc.id);
    if (!fs.existsSync(dir)) return;
    const keep = new Set();
    for (const [folder, box] of Object.entries(cache.boxes)) {
      for (const m of box.messages) keep.add(path.basename(this.bodyFile(acc, box, folder, m.uid)));
      const found = this.foundIn(acc.id, folder);
      if (found) for (const uid of found.hits.keys()) keep.add(path.basename(this.bodyFile(acc, box, folder, uid)));
    }
    for (const name of fs.readdirSync(dir)) {
      if (name.endsWith('.eml') && !keep.has(name)) fs.rmSync(path.join(dir, name), { force: true });
    }
  }

  // fast: for the reader, which can show a large message before its attachments are in (readableSource). The result
  // then has partial set, attachment sizes from the server, and nothing in place of the inline pictures that didn't
  // come with the text. Everything else gets the whole message.
  async getMessage(id, { fast = false } = {}) {
    const skeleton = fast ? await this.readableSource(id) : null;
    const parsed = await simpleParser(skeleton || (await this.rawSource(id)));
    // The size of an attachment the skeleton left empty.
    const omitted = (a) => {
      const v = skeleton && a.headers && typeof a.headers.get === 'function' ? a.headers.get(util.OMITTED.toLowerCase()) : null;
      return v != null && /^\d+$/.test(String(v)) ? Number(v) : null;
    };
    const base = String(id).startsWith('saved:')
      ? { ...this.saved.find((s) => s.id === id) }
      : (() => {
          const { acc, folder, msg } = this.locate(id);
          return this.publicMessage(acc, folder, msg);
        })();
    let html = parsed.html ? util.sanitizeHtml(parsed.html) : util.textToHtml(parsed.text || '');
    // An inline picture the skeleton left empty is an empty data: address, which shows as a broken image. A blank one
    // keeps its place until the whole message is in.
    if (skeleton) html = html.replace(/data:image\/[\w.+-]+;base64,(?=["')\s>]|$)/gi, BLANK_IMAGE);
    // An invitation's calendar part in the body (calendar: true) shows as its card in the reader, not as an attachment;
    // agents still see it as one, as before. An .ics attachment stays a plain attachment.
    const part = invite.calendarPart(parsed.attachments);
    const body = new Set(part ? part.hidden : []);
    return {
      ...(skeleton ? { partial: true } : {}),
      ...base,
      from: util.addressFromParsed(parsed.from)[0] || base.from,
      to: util.addressFromParsed(parsed.to),
      cc: util.addressFromParsed(parsed.cc),
      bcc: util.addressFromParsed(parsed.bcc),
      replyTo: util.addressFromParsed(parsed.replyTo),
      subject: parsed.subject || base.subject || '',
      date: parsed.date ? parsed.date.getTime() : base.date,
      messageId: parsed.messageId || null,
      inReplyTo: parsed.inReplyTo || null,
      references: [].concat(parsed.references || []),
      html,
      isHtml: Boolean(parsed.html),
      unsubscribe: util.unsubscribeInfo(parsed),
      text: parsed.text || util.htmlToPlain(parsed.html || ''),
      invite: part ? this.readInvite(parsed, part, base.accountId) : null,
      attachments: (parsed.attachments || [])
        .map((a, index) => ({
          index,
          filename: a.filename || t('native.attachments.defaultNumbered', { number: index + 1 }),
          contentType: a.contentType,
          size: omitted(a) ?? a.size,
          related: Boolean(a.related || (a.contentDisposition === 'inline' && a.cid)),
          ...(body.has(index) ? { calendar: true } : {})
        }))
        .filter((a) => !a.related)
    };
  }

  // The addresses an invitation can be for: the account's own and its aliases, or every account's when it isn't
  // known (a saved email, an .ics file).
  ownAddresses(accountId = null) {
    const accs = this.accounts.filter((a) => !accountId || a.id === accountId);
    return (accs.length ? accs : this.accounts).flatMap((a) => this.identities(a).map((i) => i.address.toLowerCase()));
  }

  readInvite(parsed, part, accountId) {
    const a = parsed.attachments[part.index];
    return invite.readInvite(a && a.content ? a.content.toString('utf8') : '', { me: this.ownAddresses(accountId), method: part.method });
  }

  // An email's invitation for answering it: its calendar text, its METHOD and the account it came to. null without one.
  async inviteSource(id) {
    const parsed = await simpleParser(await this.rawSource(id));
    const part = invite.calendarPart(parsed.attachments);
    const a = part && parsed.attachments[part.index];
    if (!a || !a.content) return null;
    const accountId = String(id).startsWith('saved:') ? null : this.locate(id).acc.id;
    return { text: a.content.toString('utf8'), method: part.method, accountId };
  }

  async attachment(id, index) {
    const parsed = await simpleParser(await this.rawSource(id));
    const a = (parsed.attachments || [])[index];
    if (!a) throw new Error(t('errors.attachment.missing'));
    // The declared charset, for text attachments that are not UTF-8 (windows-1252 CSVs, iso-8859-1 notes).
    const type = a.headers && typeof a.headers.get === 'function' ? a.headers.get('content-type') : null;
    const charset = (type && type.params && type.params.charset) || null;
    return { filename: a.filename || t('native.attachments.defaultNumbered', { number: index + 1 }), content: a.content, contentType: a.contentType, charset };
  }

  // One email's flags. An email no longer in the cache, or a server that refuses, throws.
  async setFlags(id, flags) {
    this.locate(id);
    const { error } = await this.setFlagsMany([id], flags);
    if (error) throw new Error(error);
  }

  // Flags for several emails at once, such as everything the list shows. The cache changes for all of them and the
  // list redraws once; then the server hears of them one by one, while a sync landing in between keeps the change.
  // A write waits for an earlier change to the same email still on its way, so the server ends with the latest.
  // Emails no longer in the cache are skipped. A failure puts the flag back to what the server last took, unless a
  // later change has set it since, so a refused Undo of a refused Mark all as read leaves the email as it was.
  // Resolves with the ids the server took (done), the ids it refused (failed) and the first refusal (error).
  async setFlagsMany(ids, flags) {
    const patch = {};
    for (const k of ['unread', 'starred', 'answered']) if (flags[k] !== undefined) patch[k] = Boolean(flags[k]);
    const found = [];
    for (const id of new Set(ids)) {
      try {
        found.push({ id, ...this.locate(id) });
      } catch (_) {
        // Moved or deleted since the list showed it.
      }
    }
    const accounts = new Set(found.map((f) => f.acc.id));
    for (const f of found) {
      f.hold = this.holdPending(f.acc.id, f.folder, f.uid, patch, f.msg);
      // Older inbox mail the server search found is in the server's count of older unread mail (olderUnread()), which
      // only the next sync redoes, so the count follows the message here. See recountOf().
      f.recount = f.older && patch.unread !== undefined ? this.recountOf(f.acc.id, f.folder, f.uid, f.msg) : null;
      Object.assign(f.msg, patch);
      if (f.recount) {
        this.recounts.set(f.recount.key, f.recount);
        this.recount(f.recount);
      }
    }
    for (const id of accounts) this.changed(id);
    const done = [];
    const failed = [];
    let error = null;
    for (const f of found) {
      try {
        await f.hold.before;
        await this.session(f.acc).setFlags(f.folder, f.uid, patch);
        f.hold.confirm();
        done.push(f.id);
      } catch (err) {
        failed.push(f.id);
        error = error || friendlyError(err);
        // Revert on whatever object the cache holds now; a sync may have replaced it, and counted again.
        const back = f.hold.rollback();
        try {
          Object.assign(this.locate(f.id).msg, back);
        } catch (_) {
          // The message left the cache in the meantime; a move still on its way puts this object back if it fails.
          Object.assign(f.msg, back);
        }
        if (f.recount) this.recount(f.recount);
      } finally {
        f.hold.release();
      }
    }
    if (failed.length) for (const id of accounts) this.changed(id);
    return { done, failed, error };
  }

  // Changes an inbox's count of older unread mail by n, when the server gave one.
  recountOlder(box, n) {
    if (n && box && Number.isInteger(box.olderUnseen)) box.olderUnseen = Math.max(0, box.olderUnseen + n);
  }

  // The record of what the changes to one older message the server search found did to the folder's count of older
  // unread mail, while any of them is on its way to the server. One record serves all of the email's changes, flags
  // and a move alike, and lasts until the last of them settles (holdPending()), so whichever of them fail or get
  // through, in whatever order, the count ends matching the message. box: whose count, null once it no longer holds
  // the message (carryRecounts()); basis: whether that count holds the message as unread; applied: what has gone onto
  // it since. msg: the message (the search results keep the object); out: a move or delete took it out of the folder.
  // A new record is held once this.recounts has it, which setFlagsMany() and withPending() do.
  recountOf(accountId, folder, uid, msg) {
    const key = this.pendingKey(accountId, folder, uid);
    const held = this.recounts.get(key);
    if (held) return held;
    const box = this.caches.get(accountId).boxes[folder];
    return { key, accountId, folder, uid, msg, out: false, box, basis: Boolean(msg.unread), applied: 0 };
  }

  // Moves the count to the message as the cache has it now: counted while it is unread and in the folder.
  recount(r) {
    const want = (!r.out && r.msg.unread ? 1 : 0) - (r.basis ? 1 : 0);
    this.recountOlder(r.box, want - r.applied);
    r.applied = want;
  }

  // After a sync replaced a folder's box: its count holds each older message as the server had it, which the flags it
  // brought for the messages with changes still on their way say (kept). Their records start again from there, on the
  // new box. A message without flags is gone from the server, in the window now, or the folder's uids were given out
  // again: it no longer changes the count, box null.
  carryRecounts(accountId, folderPath, kept) {
    const box = this.caches.get(accountId).boxes[folderPath];
    const rows = new Map((Array.isArray(kept) ? kept : []).map((k) => [k.uid, k]));
    for (const r of this.recounts.values()) {
      if (r.accountId !== accountId || r.folder !== folderPath) continue;
      const row = rows.get(r.uid);
      r.box = row ? box : null;
      r.basis = Boolean(row && row.unread);
      r.applied = 0;
      this.recount(r);
    }
  }

  // Takes a message out of the cache, as a move or delete does before the server answers, older mail the server
  // search found included. Returns what puts it back when the server turns the command down, with recount, the
  // message's record of the inbox's count of older unread mail (recountOf()), for withPending().
  removeFromCache(acc, folder, uid) {
    const cache = this.caches.get(acc.id);
    const box = cache.boxes[folder];
    const msg = box && box.messages.find((m) => m.uid === uid);
    if (box) box.messages = box.messages.filter((m) => m.uid !== uid);
    const entry = this.foundIn(acc.id, folder);
    const hit = entry && entry.hits.get(uid);
    // Unread older inbox mail leaves the server's count with it.
    const recount = hit ? this.recountOf(acc.id, folder, uid, hit.msg) : null;
    if (hit) {
      entry.hits.delete(uid);
      recount.out = true;
      this.recount(recount);
    }
    const putBack = () => {
      const now = (this.caches.get(acc.id) || { boxes: {} }).boxes[folder];
      if (!now) return;
      if (msg && !now.messages.some((m) => m.uid === uid)) now.messages.push(msg);
      if (hit && this.foundIn(acc.id, folder) === entry) {
        entry.hits.set(uid, hit);
        recount.out = false;
        this.recount(recount);
      }
    };
    putBack.recount = recount;
    return putBack;
  }

  async move(id, destination) {
    const { acc, msg, folder, uid } = this.locate(id);
    if (destination === folder) return;
    const putBack = this.removeFromCache(acc, folder, uid);
    this.changed(acc.id);
    let moved;
    try {
      moved = await this.withPending(acc, folder, uid, { removed: true }, () => this.session(acc).move(folder, uid, destination), putBack.recount);
    } catch (err) {
      putBack();
      this.changed(acc.id);
      throw new Error(friendlyError(err));
    }
    // Servers without UIDPLUS do not report the new uid; such a move cannot be undone.
    if (moved && moved.uid && moved.uidValidity) {
      this.rememberMove(id, { accountId: acc.id, folder, destination, uid: moved.uid, uidValidity: moved.uidValidity, msg });
    }
    this.noteMove(acc.id, msg.messageId, destination, folder);
    if (this.caches.get(acc.id).boxes[destination]) {
      this.syncFolderAndNotify(acc, destination).catch(() => {});
    }
  }

  rememberMove(id, record) {
    const now = Date.now();
    for (const [key, r] of this.undoable) if (now - r.at > UNDO_MS) this.undoable.delete(key);
    this.undoable.set(id, { ...record, at: now });
  }

  canUndo(id) {
    return this.undoable.has(id);
  }

  // Where a message went, by its Message-ID, kept with the account's cache. Rukoo only syncs folders it has
  // opened, so this is how a reference to the message (an agent's source card) finds it in any other folder.
  // One Message-ID can have several copies (a mail to yourself in Inbox and Sent), so each record is one copy:
  // where it is now and the folders it came through (past, the last few). A move without its source folder
  // replaces whatever was known for that Message-ID.
  noteMove(accountId, messageId, folder, source = null) {
    const key = messageKey(messageId);
    const cache = this.caches.get(accountId);
    if (!key || !cache) return;
    const moves = Array.isArray(cache.moves) ? cache.moves : [];
    const prev = moves.filter((m) => m.id === key && (source == null || m.folder === source)).pop();
    const past = [...((prev && prev.past) || []), ...(source ? [source] : [])].filter((f) => f !== folder).slice(-MOVE_PAST);
    const rest = moves.filter((m) => !(m.id === key && (source == null || m === prev)));
    rest.push({ id: key, folder, past });
    cache.moves = rest.slice(-MOVES_KEPT);
    this.persistCache(accountId);
  }

  // The current id of a message Rukoo moved, found by its Message-ID in the folder it was moved to. That folder
  // is synced first when it is not in the cache yet. from: the folder the reference had it in, so a move of
  // another copy does not count; without one, the latest move of any copy. null when Rukoo did not move it or
  // it is no longer there.
  async findMoved(accountId, messageId, from = null) {
    const key = messageKey(messageId);
    const cache = this.caches.get(accountId);
    const mine = key && cache && Array.isArray(cache.moves) ? cache.moves.filter((m) => m.id === key && (from == null || (m.past || []).includes(from))) : [];
    const last = mine[mine.length - 1];
    if (!last || !cache.folders.some((f) => f.path === last.folder)) return null;
    let uid = this.uidByMessageId(accountId, last.folder, messageId);
    if (!uid) {
      await this.openFolder(accountId, last.folder);
      uid = this.uidByMessageId(accountId, last.folder, messageId);
    }
    return uid ? encodeId(accountId, last.folder, uid) : null;
  }

  // Moves a message back to where it was before move(), remove() or archive().
  // Returns the id it gets in its old folder.
  async undoMove(id) {
    const rec = this.undoable.get(id);
    this.undoable.delete(id);
    if (!rec || Date.now() - rec.at > UNDO_MS) throw new Error(t('errors.undo.expired'));
    const acc = this.account(rec.accountId);
    this.removeFromCache(acc, rec.destination, rec.uid);
    this.changed(acc.id);
    let uid;
    try {
      const back = await this.withPending(acc, rec.destination, rec.uid, { removed: true }, () =>
        this.session(acc).move(rec.destination, rec.uid, rec.folder, { expectUidValidity: rec.uidValidity })
      );
      uid = back && back.uid;
    } catch (err) {
      if (this.caches.get(acc.id).boxes[rec.destination]) this.syncFolderAndNotify(acc, rec.destination).catch(() => {});
      if (err && err.code === 'UIDVALIDITY') throw new Error(t('errors.undo.rebuilt'));
      throw new Error(friendlyError(err));
    }
    // Put it back right away; with the new uid in the cache, the follow-up sync does not report it as new mail.
    const box = this.caches.get(acc.id).boxes[rec.folder];
    if (box && uid && !box.messages.some((m) => m.uid === uid)) box.messages.push({ ...rec.msg, uid });
    this.noteMove(acc.id, rec.msg.messageId, rec.folder, rec.destination);
    this.changed(acc.id);
    if (box) this.syncFolderAndNotify(acc, rec.folder).catch(() => {});
    return uid ? encodeId(acc.id, rec.folder, uid) : null;
  }

  // The account a message (or a saved copy) belongs to.
  messageAccount(id) {
    if (String(id).startsWith('saved:')) {
      const s = this.saved.find((x) => x.id === id);
      return s ? this.account(s.accountId) : null;
    }
    return this.account(decodeId(id).accountId);
  }

  async createFolder(accountId, name) {
    const clean = String(name || '').trim();
    if (!clean) throw new Error(t('errors.folder.nameRequired'));
    if (clean.length > 100 || /[\\/%*]/.test(clean)) throw new Error(t('errors.folder.invalidName'));
    const acc = this.account(accountId);
    const cache = this.caches.get(acc.id);
    if (cache.folders.some((f) => (f.name || '').toLowerCase() === clean.toLowerCase())) throw new Error(t('errors.folder.duplicate'));
    const created = await this.session(acc).createFolder(clean);
    cache.folders = await this.session(acc).listFolders();
    this.changed(acc.id);
    return created;
  }

  async archive(id) {
    const { acc } = this.locate(id);
    const target = this.archiveFolder(acc.id);
    if (!target) throw new Error(t('errors.folder.noArchive'));
    await this.move(id, target.path);
  }

  // Moves to Prullenbak, or deletes for good when the message is already there.
  // Without a Prullenbak it returns 'confirm' and changes nothing until called with force.
  async remove(id, { force = false } = {}) {
    const { acc, folder, uid } = this.locate(id);
    const trash = this.folderByRole(acc.id, 'trash');
    if (trash && trash.path !== folder) return this.move(id, trash.path).then(() => 'trash');
    if (!trash && !force) return 'confirm';
    const putBack = this.removeFromCache(acc, folder, uid);
    this.changed(acc.id);
    try {
      await this.withPending(acc, folder, uid, { removed: true }, () => this.session(acc).deleteForever(folder, uid), putBack.recount);
    } catch (err) {
      putBack();
      this.changed(acc.id);
      throw new Error(friendlyError(err));
    }
    return 'deleted';
  }

  async emptyFolder(accountId, folderPath) {
    const ids = this.listMessages({ scope: accountId, view: 'folder', folder: folderPath }).map((m) => m.id);
    for (const id of ids) await this.remove(id, { force: true }).catch(() => {});
    return ids.length;
  }

  toggleVip(address) {
    const a = lowerAddr(address);
    const vips = new Set(this.settings.vips);
    if (vips.has(a)) vips.delete(a);
    else vips.add(a);
    this.updateSettings({ vips: [...vips] });
    return vips.has(a);
  }

  async markSpam(id) {
    const { acc, msg } = this.locate(id);
    const a = lowerAddr(msg.from);
    if (!this.settings.spam.includes(a)) this.updateSettings({ spam: [...this.settings.spam, a] });
    const junk = this.folderByRole(acc.id, 'junk');
    if (junk) await this.move(id, junk.path);
  }

  async saveToDevice(id) {
    const raw = await this.rawSource(id);
    const { acc, folder, msg } = this.locate(id);
    const pub = this.publicMessage(acc, folder, msg);
    const savedId = `saved:${crypto.randomUUID()}`;
    const file = `${savedId.slice(6)}.eml`;
    fs.mkdirSync(this.file('saved'), { recursive: true });
    fs.writeFileSync(this.file('saved', file), raw);
    // The Message-ID lets a reference to the copy find the email in the mailbox once the copy is deleted.
    this.saved.push({ ...pub, messageId: msg.messageId || null, id: savedId, file, role: 'saved', folder: null, starred: false, unread: false });
    writeJson(this.file('saved.json'), this.saved);
    this.emit('updated');
    return savedId;
  }

  deleteSaved(id) {
    const item = this.saved.find((s) => s.id === id);
    if (!item) return;
    fs.rmSync(this.file('saved', item.file), { force: true });
    this.saved = this.saved.filter((s) => s !== item);
    writeJson(this.file('saved.json'), this.saved);
    this.emit('updated');
  }

  // ---------- compose ----------

  async buildMail(acc, payload) {
    const from = this.identity(acc, payload.from || null);
    if (!from) throw new Error(t('errors.send.identity', { address: payload.from }));
    const inline = util.inlineDataImages(payload.html || '');
    return {
      from: { name: from.name || acc.name || '', address: from.address },
      to: addressList(payload.to),
      cc: addressList(payload.cc),
      bcc: addressList(payload.bcc),
      subject: payload.subject || '',
      html: inline.html || undefined,
      text: payload.text || (payload.html ? util.htmlToPlain(payload.html) : ''),
      inReplyTo: payload.inReplyTo || undefined,
      references: payload.references && payload.references.length ? payload.references : undefined,
      date: new Date(),
      attachments: [
        ...(payload.attachments || []).map((a) =>
          a.path
            ? { filename: a.filename || path.basename(a.path), path: a.path }
            : { filename: a.filename, content: Buffer.from(a.content, 'base64'), contentType: a.contentType }
        ),
        ...(await this.forwardAttachments(payload)),
        ...inline.attachments
      ]
    };
  }

  // Attachments of the original message that the user kept when forwarding.
  async forwardAttachments(payload) {
    if (!payload.forwardId || !(payload.forwardIndexes || []).length) return [];
    const out = [];
    for (const index of payload.forwardIndexes) {
      const a = await this.attachment(payload.forwardId, index);
      out.push({ filename: a.filename, content: a.content, contentType: a.contentType });
    }
    return out;
  }

  contacts() {
    const map = new Map();
    const add = (a, weight) => {
      const key = lowerAddr(a);
      if (!key || !key.includes('@') || /no-?reply|do-?not-?reply/.test(key)) return;
      const prev = map.get(key) || { name: '', address: a.address, score: 0 };
      if (!prev.name && a.name) prev.name = a.name;
      prev.score += weight;
      map.set(key, prev);
    };
    const own = new Set(this.accounts.flatMap((a) => this.identities(a).map((i) => i.address.toLowerCase())));
    for (const cache of this.caches.values()) {
      for (const [folderPath, box] of Object.entries(cache.boxes)) {
        const info = cache.folders.find((f) => f.path === folderPath) || {};
        for (const m of box.messages) {
          // People you write to rank above people who write to you.
          if (info.role === 'sent') (m.to || []).concat(m.cc || []).forEach((a) => add(a, 3));
          else add(m.from, 1);
        }
      }
    }
    return [...map.values()]
      .filter((c) => !own.has(c.address.toLowerCase()))
      .sort((a, b) => b.score - a.score)
      .map(({ name, address }) => ({ name, address }));
  }

  // The people in an email thread, for @-mentions in the composer: everyone on the account's synced messages whose
  // Message-ID is one of ids (a reply's References and In-Reply-To), or that answer one of them. People from the
  // newest message come first; your own addresses and no-reply senders are left out.
  threadContacts(accountId, ids) {
    const cache = this.caches.get(accountId);
    const want = new Set((Array.isArray(ids) ? ids : []).map(messageKey).filter(Boolean));
    if (!cache || !want.size) return [];
    const inThread = (v) => Boolean(v) && want.has(messageKey(v));
    const hits = [];
    for (const box of Object.values(cache.boxes)) {
      for (const m of box.messages) if (inThread(m.messageId) || inThread(m.inReplyTo)) hits.push(m);
    }
    hits.sort((a, b) => (b.date || 0) - (a.date || 0));
    const own = new Set(this.ownAddresses());
    const people = new Map();
    for (const m of hits) {
      for (const a of [m.from, ...(m.replyTo || []), ...(m.to || []), ...(m.cc || [])]) {
        const key = lowerAddr(a);
        if (!key.includes('@') || own.has(key) || /no-?reply|do-?not-?reply/.test(key)) continue;
        const prev = people.get(key);
        if (!prev) people.set(key, { name: (a && a.name) || '', address: a.address });
        else if (!prev.name && a.name) prev.name = a.name;
      }
    }
    return [...people.values()];
  }

  // Synced mail related to m: linked by Message-ID, In-Reply-To or References, or with the same subject minus Re:
  // and Fwd: and someone in common other than the user. m is a full message or one built from the cache; its
  // References count, which the cache doesn't keep. Newest first, at most limit. skipRoles: folder roles to leave
  // out; skipSpam: leave out mail from addresses marked as spam, before the limit. Gmail's All Mail repeats the other
  // folders, so its copy counts only when no other folder has the email.
  // The agents' get_screen (agents/tools.js threadOf) and the reader's related emails (related()) use this.
  relatedMessages(m, { limit = 10, skipRoles = [], skipSpam = false } = {}) {
    const ids = util.threadIds(m);
    const self = util.normId(m.messageId);
    const subject = util.baseSubject(m.subject);
    const own = new Set(this.ownAddresses());
    const people = (x) => [x.from, ...(x.to || []), ...(x.cc || [])].map((a) => lowerAddr(a && a.address)).filter(Boolean);
    const theirs = new Set(people(m).filter((a) => !own.has(a)));
    const seen = new Set();
    const hits = [];
    for (const acc of this.accounts) {
      const cache = this.caches.get(acc.id);
      if (!cache) continue;
      const role = (p) => (cache.folders.find((f) => f.path === p) || {}).role;
      const boxes = Object.entries(cache.boxes)
        .filter(([p]) => !skipRoles.includes(role(p)))
        .sort(([a], [b]) => Number(role(a) === 'all') - Number(role(b) === 'all'));
      for (const [folder, box] of boxes) {
        for (const r of box.messages) {
          const id = encodeId(acc.id, folder, r.uid);
          const mid = util.normId(r.messageId);
          if (id === m.id || (mid && mid === self) || (skipSpam && this.isSpam(r.from))) continue;
          const link = util.linked(ids, r);
          const similar = !link && subject && util.baseSubject(r.subject) === subject && people(r).some((a) => theirs.has(a));
          if (!link && !similar) continue;
          const key = mid || id;
          if (seen.has(key)) continue;
          seen.add(key);
          hits.push({ acc, folder, r });
        }
      }
    }
    hits.sort((a, b) => (b.r.date || 0) - (a.r.date || 0));
    return hits.slice(0, limit).map(({ acc, folder, r }) => this.publicMessage(acc, folder, r));
  }

  // The related emails under an email in the reader. references: the email's References, from getMessage. Trash and
  // spam stay out, like they do from the inbox; so does mail from addresses marked as spam.
  related(id, references = []) {
    if (String(id).startsWith('saved:')) return [];
    const { msg } = this.locate(id);
    const m = { id, messageId: msg.messageId, inReplyTo: msg.inReplyTo, references, subject: msg.subject, from: msg.from, to: msg.to, cc: msg.cc };
    return this.relatedMessages(m, { limit: RELATED_LIMIT, skipRoles: ['trash', 'junk'], skipSpam: true });
  }

  // What send() refuses before it builds the mail: an unknown account or sender address, no recipients, an address
  // that isn't one. Main asks first when Send waits for its Undo notice (outbox.js), so the open editor still shows
  // the mistake. Returns the account.
  checkSend(payload) {
    const acc = this.account(payload.accountId);
    if (!this.identity(acc, payload.from || null)) throw new Error(t('errors.send.identity', { address: payload.from }));
    const people = [...addressList(payload.to), ...addressList(payload.cc), ...addressList(payload.bcc)];
    if (!people.length) throw new Error(t('errors.send.noRecipients'));
    for (const a of people) {
      if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(a.address)) throw new Error(t('common.errors.invalidEmailValue', { address: a.address }));
    }
    return acc;
  }

  // icalEvent: an iCalendar part for the message ({ method, content }), only from the main process: the answer to an
  // invitation (see answerInvite in main.js). It goes as the text/calendar alternative and as invite.ics.
  async send(payload, { icalEvent = null } = {}) {
    const acc = this.checkSend(payload);
    const mail = await this.buildMail(acc, payload);
    if (icalEvent) mail.icalEvent = { method: icalEvent.method, content: icalEvent.content, filename: 'invite.ics' };
    const raw = await this.session(acc).send(mail);
    // Sending takes a while. New account settings meanwhile close the session it started on, so the Sent copy goes
    // through the session the account has now; an account removed meanwhile keeps no copy.
    if (this.accounts.includes(acc)) {
      const session = this.session(acc);
      const sent = this.folderByRole(acc.id, 'sent');
      if (sent && !session.savesSentCopy()) await session.append(sent.path, raw, ['\\Seen']).catch(() => {});
    }
    if (payload.draftId) await this.discardDraft(payload.draftId).catch(() => {});
    if (payload.replyToId) {
      // The original may have been moved in the meantime; that is fine.
      await this.setFlags(payload.replyToId, { answered: true }).catch(() => {});
    }
    this.syncAccount(acc.id).catch(() => {});
    return true;
  }

  async saveDraft(payload) {
    const acc = this.account(payload.accountId);
    const drafts = this.folderByRole(acc.id, 'drafts');
    if (!drafts) throw new Error(t('errors.folder.noDrafts'));
    const mail = await this.buildMail(acc, payload);
    // A Message-ID of our own finds the stored draft again on servers that do not report its uid.
    mail.messageId = `<${crypto.randomUUID()}@${mail.from.address.split('@')[1] || 'rukoo.invalid'}>`;
    // Keep Bcc in the stored draft so it survives reopening.
    const raw = await ImapAccount.compose({ ...mail, keepBcc: true });
    const session = this.session(acc);
    const uid = await session.append(drafts.path, raw, ['\\Seen', '\\Draft']);
    if (payload.draftId) await this.discardDraft(payload.draftId).catch(() => {});
    await this.syncFolderInto(acc, drafts.path);
    this.changed(acc.id);
    const stored = uid || this.uidByMessageId(acc.id, drafts.path, mail.messageId);
    return stored ? encodeId(acc.id, drafts.path, stored) : null;
  }

  uidByMessageId(accountId, folderPath, messageId) {
    const box = this.caches.get(accountId).boxes[folderPath];
    const want = messageKey(messageId);
    const hit = want && box && box.messages.find((m) => m.messageId && messageKey(m.messageId) === want);
    return hit ? hit.uid : null;
  }

  async discardDraft(id) {
    const { acc, folder, uid } = this.locate(id);
    this.removeFromCache(acc, folder, uid);
    this.changed(acc.id);
    await this.session(acc).deleteForever(folder, uid);
  }

  async close() {
    this.closed = true;
    this.flush();
    await Promise.all([...this.sessions.values()].map((s) => s.close().catch(() => {})));
    this.sessions.clear();
  }
}

module.exports = { Engine, encodeId, decodeId, DEFAULT_SETTINGS };
