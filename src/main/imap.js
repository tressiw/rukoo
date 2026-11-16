'use strict';

const { t } = require('../i18n');

const { ImapFlow } = require('imapflow');
const nodemailer = require('nodemailer');
const MailComposer = require('nodemailer/lib/mail-composer');
const util = require('./mailutil');
const { SIGNAL_HEADERS, mailSignals } = require('./categories/signals');
const { appPasswordProvider } = require('./providers');

const PREVIEW_BYTES = 4096;
// What a cached message needs from the server besides its flags. These never change for a uid, so a sync fetches
// them only for messages the cache doesn't have yet; see syncFolder().
const STORED = ['messageId', 'inReplyTo', 'subject', 'from', 'to', 'cc', 'replyTo', 'date', 'preview', 'hasAttachments', 'size'];

// A connection and the commands waiting for it: what the user waits for goes first, then background work. idle
// settles when its queue has run dry.
const line = () => ({ client: null, busy: false, idle: Promise.resolve(), queue: { now: [], later: [] } });

// Hosts whose SMTP service already files a copy in the Sent folder.
const AUTO_SENT_HOSTS = /(gmail\.com|googlemail\.com|office365\.com|outlook\.com)$/i;

const errorText = (err) => String((err && (err.responseText || err.response || err.message)) || err);

// Whether the server turned the username and password down, as opposed to not being reached. imapflow marks a failed
// LOGIN or AUTHENTICATE, nodemailer a failed SMTP AUTH with EAUTH. Gmail says "Application-specific password
// required" to a normal Google password, in IMAP and in SMTP.
function authFailed(err) {
  if (!err || err.oauth) return false;
  if (err.authenticationFailed || err.code === 'EAUTH') return true;
  return /AUTHENTICATIONFAILED|Invalid credentials|authentication failed|535|Username and Password not accepted|Application-specific password required/i.test(errorText(err));
}

// account: the account the error is for, when known. A refused sign-in at a provider that only takes an app
// password then says so instead of the general message.
function friendlyError(err, account = null) {
  const msg = errorText(err);
  if (err && err.oauth) return msg;
  if (authFailed(err)) {
    const provider = appPasswordProvider(account);
    if (provider === 'google') return t('errors.connection.googleAppPassword');
    if (provider === 'yahoo') return t('errors.connection.yahooAppPassword');
    return t('errors.connection.auth');
  }
  if (/ENOTFOUND|EAI_AGAIN/i.test(msg)) return t('errors.connection.host');
  if (/ECONNREFUSED/i.test(msg)) return t('errors.connection.refused');
  if (/ETIMEDOUT|timeout/i.test(msg)) return t('errors.connection.timeout');
  if (/certificate|self.signed|CERT_/i.test(msg)) return t('errors.connection.certificate');
  return msg;
}

// How many messages in the open folder are unread below uid first, the oldest message of the window a sync fetches;
// null when the server doesn't say. Only below the window: the engine counts the window's unread mail in its cache,
// where reading a message counts at once. With ESEARCH the server sends only the count; otherwise imapflow counts the
// uids it gets.
async function unseenBelow(client, first) {
  if (!first) return null;
  if (first === 1) return 0;
  try {
    const found = await client.search({ seen: false, uid: `1:${first - 1}` }, { uid: true, returnOptions: ['COUNT'] });
    if (Array.isArray(found)) return found.length;
    return found && Number.isInteger(found.count) ? found.count : null;
  } catch (_) {
    return null;
  }
}

// The flags of the messages with these uids in the open folder, as { uid, unread, starred, answered }; a uid the
// server no longer has is left out. null when the server doesn't answer, so the caller keeps what it had.
async function flagsOf(client, uids) {
  try {
    const out = [];
    for await (const m of client.fetch(uids.join(','), { uid: true, flags: true }, { uid: true })) {
      const flags = m.flags || new Set();
      out.push({ uid: m.uid, unread: !flags.has('\\Seen'), starred: flags.has('\\Flagged'), answered: flags.has('\\Answered') });
    }
    return out;
  } catch (_) {
    return null;
  }
}

// The uids in the open folder that match query, within criteria (a uid range, flags). Gmail searches the way its own
// search box does (X-GM-RAW), the message text included; not with gmail false, for Spam and Trash, which that search
// leaves out unless asked. Other servers, and Gmail when its search fails, compare the subject and the addresses.
// imapflow answers false for a search the server refused.
async function searchUids(client, criteria, query, { gmail = true } = {}) {
  if (gmail && client.capabilities && client.capabilities.has('X-GM-EXT-1')) {
    try {
      const found = await client.search({ ...criteria, gmraw: query }, { uid: true });
      if (Array.isArray(found)) return found;
    } catch (_) {
      // The plain search below.
    }
  }
  const found = await client.search({ ...criteria, or: [{ subject: query }, { from: query }, { to: query }, { cc: query }] }, { uid: true });
  if (!Array.isArray(found)) throw new Error(t('errors.search.refused'));
  return found;
}

// The cache's form of the messages in rows (uid and flags each, as a FETCH gave them), in the order of rows. What known
// holds for a uid is reused; envelope, structure and a preview come from the server only for the rest: on a large
// Gmail folder that is the slow part, and repeating it every sync kept the connection busy for minutes. signals: also
// what kind of mail each one is (categories/signals.js). tabs: the Gmail tab of each uid, or null when not asked. A
// message gone between the fetches (moved or deleted meanwhile) is left out.
async function describe(client, rows, { known = new Map(), signals = false, tabs = null } = {}) {
  const cached = (uid) => {
    const prior = known.get(uid);
    return Boolean(prior) && STORED.every((k) => k in prior);
  };
  const fresh = rows.filter((m) => !cached(m.uid)).map((m) => m.uid);
  const details = new Map();
  if (fresh.length) {
    for await (const m of client.fetch(fresh.join(','), { uid: true, envelope: true, bodyStructure: true, internalDate: true, size: true }, { uid: true })) {
      details.set(m.uid, m);
    }
    // imapflow skips a FETCH response it can't parse, and the command still succeeds. It parses 25 levels of
    // parentheses, so a message with some 22 nested multiparts is left out, and would be asked for on every sync
    // and never listed. Asked again without the structure, it lists by its envelope, without a preview or
    // attachment mark, and is cached.
    const missing = fresh.filter((uid) => !details.has(uid));
    if (missing.length) {
      for await (const m of client.fetch(missing.join(','), { uid: true, envelope: true, internalDate: true, size: true }, { uid: true })) {
        details.set(m.uid, m);
      }
    }
  }

  // Fetch a few KB of the readable part for messages we have not seen before.
  const previews = new Map();
  const byPart = new Map();
  for (const m of details.values()) {
    const node = util.findPreviewPart(m.bodyStructure);
    if (!node) continue;
    const key = util.partKey(node);
    if (!byPart.has(key)) byPart.set(key, []);
    byPart.get(key).push({ uid: m.uid, node });
  }
  for (const [key, items] of byPart) {
    const nodes = new Map(items.map((i) => [i.uid, i.node]));
    const uids = items.map((i) => i.uid).join(',');
    for await (const m of client.fetch(uids, { uid: true, bodyParts: [{ key, maxLength: PREVIEW_BYTES }] }, { uid: true })) {
      const node = nodes.get(m.uid);
      const buf = m.bodyParts && m.bodyParts.values().next().value;
      if (!buf || !node) continue;
      const decodedAlready = m.binaryParts && m.binaryParts.size > 0;
      const effective = decodedAlready ? { ...node, encoding: '7bit' } : node;
      try {
        previews.set(m.uid, util.previewFromPart(buf, effective));
      } catch (_) {
        previews.set(m.uid, '');
      }
    }
  }

  // A few header lines, for new messages and for ones cached before Rukoo kept them.
  const kinds = new Map();
  if (signals) {
    const need = rows.filter((r) => details.has(r.uid) || !(known.get(r.uid) || {}).signals).map((r) => r.uid);
    if (need.length) {
      for await (const m of client.fetch(need.join(','), { uid: true, headers: SIGNAL_HEADERS }, { uid: true })) {
        kinds.set(m.uid, mailSignals(m.headers));
      }
    }
  }
  const extra = (uid, prior) => {
    const out = {};
    const kind = kinds.get(uid) || (prior && prior.signals);
    if (kind) out.signals = kind;
    if (tabs) out.gmailTab = tabs.get(uid) || 'primary';
    else if (prior && prior.gmailTab) out.gmailTab = prior.gmailTab;
    return out;
  };

  const messages = [];
  for (const row of rows) {
    const flags = row.flags || new Set();
    const state = { unread: !flags.has('\\Seen'), starred: flags.has('\\Flagged'), answered: flags.has('\\Answered') };
    const m = details.get(row.uid);
    const prior = known.get(row.uid);
    if (!m) {
      // Gone between the fetches: the next sync no longer lists it.
      if (!cached(row.uid)) continue;
      messages.push({ uid: row.uid, ...Object.fromEntries(STORED.map((k) => [k, prior[k]])), ...state, ...extra(row.uid, prior) });
      continue;
    }
    const env = m.envelope || {};
    messages.push({
      uid: m.uid,
      messageId: env.messageId || null,
      inReplyTo: env.inReplyTo || null,
      subject: env.subject || '',
      from: util.addressFromEnvelope(env.from)[0] || { name: '', address: '' },
      to: util.addressFromEnvelope(env.to),
      cc: util.addressFromEnvelope(env.cc),
      replyTo: util.addressFromEnvelope(env.replyTo),
      date: new Date(env.date || m.internalDate || Date.now()).getTime(),
      preview: previews.has(m.uid) ? previews.get(m.uid) : prior ? prior.preview || '' : '',
      ...state,
      hasAttachments: util.hasAttachments(m.bodyStructure),
      size: m.size || 0,
      ...extra(m.uid, prior)
    });
  }
  return messages;
}

class ImapAccount {
  // credentials: a password string, or { accessToken: async () => token } for OAuth (XOAUTH2).
  constructor(account, credentials) {
    this.account = account;
    this.credentials = credentials;
    // Two connections. The main one does all the work: syncs, prefetching, flags, moves. The reader only fetches the
    // message the user opens (read()), so opening one never waits for a sync or prefetch that is running.
    this.main = line();
    this.reader = line();
    // Set when the server turned the reader down: reads then go over the main connection, ahead of its waiting work.
    this.oneConnection = false;
    // Set by close(): the account is gone or uses a new session, so nothing may connect through this one again.
    this.closed = false;
  }

  async auth(user) {
    const c = this.credentials;
    if (c && typeof c === 'object' && c.accessToken) return { user, accessToken: await c.accessToken() };
    return { user, pass: c };
  }

  async imapOptions() {
    const { imap } = this.account;
    return {
      host: imap.host,
      port: Number(imap.port),
      secure: Boolean(imap.secure),
      auth: await this.auth(imap.user || this.account.email),
      logger: false,
      emitLogs: false,
      connectionTimeout: 20000,
      greetingTimeout: 15000,
      socketTimeout: 5 * 60 * 1000,
      tls: { rejectUnauthorized: !imap.allowInvalidCert }
    };
  }

  async transport() {
    const { smtp } = this.account;
    const auth = await this.auth(smtp.user || this.account.email);
    return nodemailer.createTransport({
      host: smtp.host,
      port: Number(smtp.port),
      secure: Boolean(smtp.secure),
      requireTLS: !smtp.secure,
      auth: auth.accessToken ? { type: 'OAuth2', user: auth.user, accessToken: auth.accessToken } : auth,
      connectionTimeout: 20000,
      tls: { rejectUnauthorized: !smtp.allowInvalidCert }
    });
  }

  async connect(conn = this.main) {
    if (conn.client && conn.client.usable) return conn.client;
    const client = new ImapFlow(await this.imapOptions());
    const drop = () => {
      if (conn.client === client) conn.client = null;
    };
    client.on('error', drop);
    client.on('close', drop);
    try {
      await client.connect();
    } catch (err) {
      // A rejected login can leave the TLS socket open.
      client.close();
      throw err;
    }
    // Closed while it connected: the session is gone, so the connection goes too.
    if (this.closed) {
      client.logout().catch(() => client.close());
      throw new Error(t('errors.connection.closed'));
    }
    conn.client = client;
    return client;
  }

  // The commands on a connection run one at a time. Background work waits until nothing else is queued: a flag
  // change goes ahead of the next folder of a sync, but not ahead of the command already running.
  run(fn, { background = false } = {}) {
    return this.queue(this.main, fn, background);
  }

  // Fetching the message the user opens: over the reader connection, which nothing else uses. It only reads, so it
  // can run next to a sync without the two seeing different states of a change.
  read(fn) {
    return this.oneConnection ? this.run(fn) : this.queue(this.reader, fn, false);
  }

  queue(conn, fn, background) {
    return new Promise((resolve, reject) => {
      conn.queue[background ? 'later' : 'now'].push({ fn, resolve, reject });
      this.drain(conn);
    });
  }

  async drain(conn) {
    if (conn.busy) return;
    conn.busy = true;
    let done;
    conn.idle = new Promise((resolve) => (done = resolve));
    try {
      for (let task; (task = conn.queue.now.shift() || conn.queue.later.shift()); ) {
        if (this.closed) {
          task.reject(new Error(t('errors.connection.closed')));
          continue;
        }
        let client;
        try {
          client = await this.connect(conn);
        } catch (err) {
          client = await this.oneAtATime(conn, task, err);
          if (!client) continue;
        }
        // Nothing runs for a session closed while it connected: an account removed, or signed in again.
        if (this.closed) {
          task.reject(new Error(t('errors.connection.closed')));
          continue;
        }
        try {
          task.resolve(await task.fn(client));
        } catch (err) {
          if (conn.client && !conn.client.usable) conn.client = null;
          task.reject(err);
        }
      }
    } finally {
      conn.busy = false;
      done();
    }
  }

  // A server that takes one connection at a time turns down whichever of the two connects second. From then on
  // everything goes over the main connection, reads ahead of waiting background work, as before there was a reader.
  // Returns the client to run task on, or null once task is turned down or handed to the main connection.
  async oneAtATime(conn, task, err) {
    const other = conn === this.reader ? this.main : this.reader;
    if (!other.client || !other.client.usable) {
      task.reject(err);
      return null;
    }
    this.oneConnection = true;
    if (conn === this.reader) {
      for (const waiting of [task, ...conn.queue.now.splice(0)]) this.run(waiting.fn).then(waiting.resolve, waiting.reject);
      return null;
    }
    // The main connection is the one that has to work: the reader finishes the message it is fetching, hands over
    // what waits for it and logs out, and the main connection tries again.
    for (const waiting of this.reader.queue.now.splice(0)) this.run(waiting.fn).then(waiting.resolve, waiting.reject);
    await this.reader.idle;
    const reader = this.reader.client;
    this.reader.client = null;
    if (reader) {
      try {
        await reader.logout();
      } catch (_) {
        reader.close();
      }
    }
    // Closed meanwhile: the command is for a session that is gone.
    if (this.closed) {
      task.reject(new Error(t('errors.connection.closed')));
      return null;
    }
    try {
      return await this.connect(conn);
    } catch (again) {
      task.reject(again);
      return null;
    }
  }

  async verify() {
    const client = new ImapFlow(await this.imapOptions());
    client.on('error', () => {});
    try {
      await client.connect();
      await client.logout();
    } catch (err) {
      client.close();
      throw this.verifyError('IMAP', err);
    }
    try {
      const transport = await this.transport();
      await transport.verify();
      transport.close();
    } catch (err) {
      throw this.verifyError('SMTP', err);
    }
  }

  // The reason after the server that gave it, with code 'auth' when that server turned the sign-in down, so the
  // setup form can tell a wrong password from a server address to correct. A provider that only takes an app
  // password gets its message without the server: the same app password signs in to both. appPasswordProvider names
  // that provider, also for an account added under "Other", so the form links to the page its message points to.
  verifyError(server, err) {
    const auth = authFailed(err);
    const provider = auth ? appPasswordProvider(this.account) : null;
    const out = new Error(provider ? friendlyError(err, this.account) : `${server}: ${friendlyError(err, this.account)}`);
    if (auth) out.code = 'auth';
    if (provider) out.appPasswordProvider = provider;
    return out;
  }

  listFolders({ background = false } = {}) {
    return this.run(async (client) => {
      const list = await client.list();
      return list
        .filter((f) => !(f.flags && f.flags.has && f.flags.has('\\Noselect')))
        .map((f) => {
          const folder = {
            path: f.path,
            name: f.name,
            delimiter: f.delimiter,
            specialUse: f.specialUse || null,
            flags: f.flags ? Array.from(f.flags) : []
          };
          folder.role = util.folderRole(folder);
          delete folder.flags;
          return folder;
        });
    }, { background });
  }

  // Creates a top-level folder in the personal namespace and returns its path.
  createFolder(name) {
    return this.run(async (client) => {
      const prefix = (client.namespace && client.namespace.prefix) || '';
      const res = await client.mailboxCreate(prefix + name);
      return (res && res.path) || prefix + name;
    });
  }

  // limit: how many of the newest messages the cache keeps, or a function that says so when the command runs: Load
  // older can widen the window while the sync waits for the connection. signals: also what kind of mail each message
  // is (categories/signals.js); gmailTabs: also the Gmail tab each message is in. Both are for the inbox and its
  // categories. olderUnseen: also how many messages older than the window are unread, which the inbox's unread count
  // adds to the window's own. keep: uids below the window the cache holds as well (the older mail a server search
  // found), or a function that gives them when the command runs; kept then has the flags of those the server still
  // has, as { uid, unread, starred, answered }. Not after a UIDVALIDITY change, when they are other messages.
  syncFolder(path, { limit = 200, known: knownIn = new Map(), uidValidity = null, background = false, signals = false, gmailTabs = false, olderUnseen = false, keep = null } = {}) {
    return this.run(async (client) => {
      const lock = await client.getMailboxLock(path, { readOnly: true });
      try {
        const box = client.mailbox;
        // After a UIDVALIDITY change old UIDs point at different messages; reuse nothing.
        const same = Boolean(uidValidity) && String(box.uidValidity) === String(uidValidity);
        const known = same ? knownIn : new Map();
        const result = { uidValidity: String(box.uidValidity), exists: box.exists, messages: [] };
        if (!box.exists) return result;
        const size = typeof limit === 'function' ? limit() : limit;
        const start = Math.max(1, box.exists - size + 1);
        // The flags of every message in the window: all that can change about a message the cache already has.
        const rows = [];
        for await (const m of client.fetch(`${start}:*`, { uid: true, flags: true })) rows.push(m);
        const first = rows.length ? Math.min(...rows.map((r) => r.uid)) : null;
        // Gmail doesn't show its tabs as folders, but its own search finds them (X-GM-RAW). Asked again on every sync,
        // since a message can move to another tab. What none of them holds is in Primary.
        let tabs = null;
        if (gmailTabs && rows.length && client.capabilities && client.capabilities.has('X-GM-EXT-1')) {
          tabs = new Map();
          try {
            for (const tab of ['promotions', 'social', 'updates', 'forums']) {
              const uids = await client.search({ uid: `${first}:*`, gmraw: `category:${tab}` }, { uid: true });
              for (const uid of uids || []) tabs.set(uid, tab);
            }
          } catch (_) {
            tabs = null;
          }
        }
        if (olderUnseen) result.olderUnseen = start === 1 ? 0 : await unseenBelow(client, first);
        result.messages = await describe(client, rows, { known, signals, tabs });
        const older = same && keep ? (typeof keep === 'function' ? keep() : keep).filter((uid) => !first || uid < first) : [];
        if (older.length) result.kept = await flagsOf(client, older);
        return result;
      } finally {
        lock.release();
      }
    }, { background });
  }

  // Older mail that matches query, for a search of more than the cache holds: in the folder at path, below the uid
  // below (the oldest message the cache has; null for the whole folder). The newest limit matches come back in the
  // cache's form, reusing what known has as syncFolder does, with total saying how many matched in all. criteria: more
  // to match, such as { seen: false }. signals as for syncFolder. gmail: false keeps Gmail to the plain search, for its
  // Spam and Trash. The user waits for it, so it goes ahead of a sync.
  searchFolder(path, query, { below = null, limit = 50, known: knownIn = new Map(), uidValidity = null, signals = false, criteria = {}, gmail = true } = {}) {
    return this.run(async (client) => {
      const lock = await client.getMailboxLock(path, { readOnly: true });
      try {
        const box = client.mailbox;
        const known = uidValidity && String(box.uidValidity) === String(uidValidity) ? knownIn : new Map();
        const result = { uidValidity: String(box.uidValidity), total: 0, messages: [] };
        if (!box.exists || below === 1) return result;
        const uids = await searchUids(client, { ...criteria, uid: below ? `1:${below - 1}` : '1:*' }, query, { gmail });
        result.total = uids.length;
        const newest = [...uids].sort((a, b) => b - a).slice(0, limit);
        if (!newest.length) return result;
        const rows = [];
        for await (const m of client.fetch(newest.join(','), { uid: true, flags: true }, { uid: true })) rows.push(m);
        result.messages = await describe(client, rows, { known, signals });
        return result;
      } finally {
        lock.release();
      }
    });
  }

  // The message the user opens comes over the reader, with the folder opened read-only: that connection only reads.
  // A prefetch runs on the main connection and opens the folder read-write, like setFlags: imapflow reopens a folder
  // whenever the mode changes. Either way the fetch leaves the flags alone: imapflow fetches with PEEK.
  fetchSource(path, uid, { background = false } = {}) {
    const fetch = async (client) => {
      const lock = await client.getMailboxLock(path, background ? {} : { readOnly: true });
      try {
        const msg = await client.fetchOne(String(uid), { uid: true, source: true }, { uid: true });
        if (!msg || !msg.source) throw new Error(t('errors.message.serverMissing'));
        return msg.source;
      } finally {
        lock.release();
      }
    };
    return background ? this.run(fetch, { background }) : this.read(fetch);
  }

  // A large message the user opens, without its attachments: its structure and header first, then the parts the
  // reader shows and the MIME header of every part (mailutil.readableParts), rebuilt into a source mailparser reads
  // like the whole message (mailutil.skeletonSource). Over the reader, like fetchSource. null when the message
  // doesn't split.
  fetchReadable(path, uid) {
    return this.read(async (client) => {
      const lock = await client.getMailboxLock(path, { readOnly: true });
      try {
        const head = await client.fetchOne(String(uid), { uid: true, bodyStructure: true, headers: true, size: true }, { uid: true });
        if (!head || !head.bodyStructure || !head.headers) throw new Error(t('errors.message.serverMissing'));
        const keys = util.readableParts(head.bodyStructure, head.size);
        if (!keys) return null;
        const msg = await client.fetchOne(String(uid), { uid: true, bodyParts: keys }, { uid: true });
        // Every part asked for, or the whole message: a text part the server left out would read as an empty one.
        if (!msg || !msg.bodyParts || !keys.every((key) => Buffer.isBuffer(msg.bodyParts.get(key.toLowerCase())))) {
          throw new Error(t('errors.message.serverMissing'));
        }
        return util.skeletonSource(head.headers, head.bodyStructure, msg.bodyParts);
      } finally {
        lock.release();
      }
    });
  }

  setFlags(path, uid, { unread, starred, answered }) {
    return this.run(async (client) => {
      const lock = await client.getMailboxLock(path);
      try {
        const opts = { uid: true };
        if (unread === true) await client.messageFlagsRemove(String(uid), ['\\Seen'], opts);
        if (unread === false) await client.messageFlagsAdd(String(uid), ['\\Seen'], opts);
        if (starred === true) await client.messageFlagsAdd(String(uid), ['\\Flagged'], opts);
        if (starred === false) await client.messageFlagsRemove(String(uid), ['\\Flagged'], opts);
        if (answered === true) await client.messageFlagsAdd(String(uid), ['\\Answered'], opts);
      } finally {
        lock.release();
      }
    });
  }

  // Returns the message's uid in the destination and that mailbox's UIDVALIDITY, as far as the
  // server reports them (UIDPLUS). With expectUidValidity, refuses to touch a mailbox whose uids
  // were reassigned since: the uid would point at another message.
  move(path, uid, destination, { expectUidValidity = null } = {}) {
    return this.run(async (client) => {
      const lock = await client.getMailboxLock(path);
      try {
        if (expectUidValidity && String(client.mailbox.uidValidity) !== String(expectUidValidity)) {
          throw Object.assign(new Error(t('errors.folder.rebuilt')), { code: 'UIDVALIDITY' });
        }
        const res = await client.messageMove(String(uid), destination, { uid: true });
        return {
          uid: res && res.uidMap ? res.uidMap.get(uid) || null : null,
          uidValidity: res && res.uidValidity ? String(res.uidValidity) : null
        };
      } finally {
        lock.release();
      }
    });
  }

  deleteForever(path, uid) {
    return this.run(async (client) => {
      const lock = await client.getMailboxLock(path);
      try {
        await client.messageDelete(String(uid), { uid: true });
      } finally {
        lock.release();
      }
    });
  }

  append(path, raw, flags = []) {
    return this.run(async (client) => {
      const res = await client.append(path, raw, flags);
      return res && res.uid ? res.uid : null;
    });
  }

  static async compose({ keepBcc = false, ...mail }) {
    const node = new MailComposer({ ...mail, attachDataUrls: true }).compile();
    // Drafts keep their Bcc header; sent mail must not reveal it.
    node.keepBcc = keepBcc;
    return node.build();
  }

  // Sends the message and returns the raw RFC 822 source.
  async send(mail) {
    const raw = await ImapAccount.compose(mail);
    const transporter = await this.transport();
    try {
      const recipients = [...(mail.to || []), ...(mail.cc || []), ...(mail.bcc || [])].map((a) => a.address || a);
      await transporter.sendMail({
        envelope: { from: mail.from.address, to: recipients },
        raw
      });
    } catch (err) {
      throw new Error(friendlyError(err));
    } finally {
      transporter.close();
    }
    return raw;
  }

  savesSentCopy() {
    return AUTO_SENT_HOSTS.test(this.account.smtp.host || '');
  }

  async close() {
    this.closed = true;
    const clients = [this.main.client, this.reader.client].filter(Boolean);
    this.main.client = null;
    this.reader.client = null;
    await Promise.all(
      clients.map(async (client) => {
        try {
          await client.logout();
        } catch (_) {
          client.close();
        }
      })
    );
  }
}

module.exports = { ImapAccount, friendlyError };
