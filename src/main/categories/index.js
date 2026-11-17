'use strict';

// Inbox categories: the inbox split into views, like Gmail's tabs, for every kind of account. Nothing is labelled,
// moved or flagged on the mail server. Which category an email is in lives in the Rukoo account (account/), so every
// device shows the same: the categories themselves, the category each email got, and the corrections a person makes.
//
// An email's category is the first of: a correction for that email, a rule for its sender, the category it got when
// it was first sorted, and else what the rules say now. The first device to sort an email stores the result; the
// others use it. Sorting uses the rules (rules.js), and Jev (jev.js) where this device has a TypeSafe key.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { EventEmitter } = require('events');
const { t } = require('../../i18n');
const { PRESETS, PRESET_COLORS, presetFor } = require('./rules');
const jevApi = require('./jev');
const { encodeId } = require('../engine');

const SETTING = 'categories';
const MESSAGE_ID_MAX = 256;
const MAX_CATEGORIES = 12;
const SORT_AFTER = 1500;
const JEV_AT_ONCE = 3;
// Below this, Jev's choice is a guess, and the email goes to the category for when Rukoo isn't sure.
const JEV_CONFIDENT = 0.5;
// Jev's last decisions, for the sidebar's "what Jev is doing".
const RECENT = 12;
// While Jev works through a long list, the inbox catches up this often, so emails move as they are sorted.
const LIVE_EVERY = 2000;
const COLORS = ['#ff5d73', '#4a7dff', '#2fd6c0', '#f5c542', '#c56cf0', '#ff8a3d', '#5ec2ff', '#7a808e'];

function readJson(file, fallback) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (_) {
    return fallback;
  }
}

const lower = (address) => String(address || '').trim().toLowerCase();

class Categories extends EventEmitter {
  constructor({ engine, account, secrets, dataDir, jev = jevApi }) {
    super();
    this.engine = engine;
    this.account = account;
    this.secrets = secrets;
    this.dataDir = dataDir;
    this.jev = jev;
    this.local = {};
    this.timer = null;
    // The sort after TypeSafe was busy, apart from soon()'s timer, which every mail update moves.
    this.retryTimer = null;
    this.sorting = null;
    this.jevBusyUntil = 0;
    // Goes up with every change to Jev's settings; what a request made under older settings answers is dropped.
    this.jevGeneration = 0;
    // Emails "Sort the inbox again" still has to sort, by key, because Jev was busy or stopped before it got to them.
    // Their older category stays until then.
    this.redo = new Set();
    // The emails the running sort has for Jev, by key, until each one's answer is in: their notifications wait for it.
    this.asking = new Set();
    // What the sorting is doing, for the sidebar (activity()). user: someone asked for this sort, so its end is
    // news: turning categories on, a new Jev key, "Sort the inbox again".
    this.progress = { running: false, user: false, again: false, total: 0, done: 0, asked: 0, answered: 0, sorted: 0, waiting: 0, startedAt: 0, finishedAt: 0 };
    this.recent = [];
    this.progressTimer = null;
    this.liveTimer = null;
    this.soonUser = false;
    // A sort is storing categories (sort()), and the end of a burst of records still to say (recordsChanged()).
    this.running = false;
    this.recordsTimer = null;
    this.recordsLater = false;
  }

  file() {
    return path.join(this.dataDir, 'categories.json');
  }

  init() {
    this.local = readJson(this.file(), {});
    this.redo = new Set(Array.isArray(this.local.redo) ? this.local.redo.filter((k) => typeof k === 'string') : []);
    this.engine.categoryOf = (acc, m) => this.of(acc, m);
    this.engine.hiddenCategories = () => this.hiddenFromInbox();
    this.account.on('records', ({ collections }) => {
      if (collections.some((c) => ['category', 'classification', 'override', 'sender', 'setting'].includes(c))) this.recordsChanged();
      // Categories turned on or off, or one removed, here or on another device: notifications waiting for Jev are
      // decided again (notices.js). Not for each email's category, which comes with every answer.
      if (collections.some((c) => c === 'category' || c === 'setting')) this.emit('reconsider');
    });
    this.engine.on('updated', () => this.soon());
    return this;
  }

  // What stays on this computer: Jev's settings, the categories whose notifications are off (muted), and the emails a
  // re-sort still has to do (redo), so a restart doesn't forget them.
  saveLocal() {
    // The account's records go to disk first: a re-sort list without an email whose new category is still only in
    // memory would forget that email for good if the app quit right then. When they can't be written, the emails on
    // the list on disk stay on it.
    const written = this.account.flush();
    this.local.redo = written ? [...this.redo] : [...new Set([...(this.local.redo || []), ...this.redo])];
    fs.mkdirSync(this.dataDir, { recursive: true });
    fs.writeFileSync(this.file(), JSON.stringify(this.local));
  }

  changed() {
    // Whatever a burst of records still had to say, this says it.
    this.recordsLater = false;
    this.emit('changed');
    this.engine.emit('updated');
  }

  // Records come in bursts: a sort by the rules stores one per inbox email in a row, a sync brings a page from another
  // device. Each change redraws the window's sidebar and list, so a burst says so once when it starts and once when it
  // is over, on the next turn of the event loop. While a sort runs, live() paces it instead, and the sort says so when
  // it ends.
  recordsChanged() {
    if (this.running) return this.live();
    if (this.recordsTimer) {
      this.recordsLater = true;
      return;
    }
    this.changed();
    this.recordsTimer = setImmediate(() => {
      this.recordsTimer = null;
      if (this.recordsLater) this.changed();
    });
  }

  // ---- what the categories are ----

  settings() {
    const v = this.account.get('setting', SETTING) || {};
    return { enabled: v.enabled === true, unsure: typeof v.unsure === 'string' ? v.unsure : 'to-read', seeded: v.seeded === true };
  }

  putSettings(patch) {
    this.account.put('setting', SETTING, { ...this.settings(), ...patch });
  }

  // The categories in order, with the names and descriptions people see. A preset nobody renamed takes its name and
  // description from the translations, so it follows the language.
  list() {
    return this.account
      .list('category')
      .map(({ k, v }) => {
        const preset = PRESETS.includes(k) ? k : null;
        return {
          id: k,
          preset,
          name: (v && v.name) || (preset ? t(`categories.presets.${preset}.name`) : k),
          description: (v && v.description) || (preset ? t(`categories.presets.${preset}.description`) : ''),
          custom: { name: Boolean(v && v.name), description: Boolean(v && v.description) },
          color: (v && v.color) || PRESET_COLORS[preset] || COLORS[0],
          order: Number(v && v.order) || 0,
          // Whether Inbox itself shows this category's mail; a record without the field is shown.
          inbox: !(v && v.inbox === false)
        };
      })
      .sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
  }

  // The categories Inbox leaves out, while categories are on. Their own tabs still show their mail.
  hiddenFromInbox() {
    if (!this.settings().enabled) return new Set();
    return new Set(this.list().filter((c) => !c.inbox).map((c) => c.id));
  }

  // Prototype for the guide: the categories as they are, or the presets they would start with, without writing them.
  preview() {
    if (this.settings().seeded) return this.list();
    return PRESETS.map((id, order) => ({
      id,
      preset: id,
      name: t(`categories.presets.${id}.name`),
      description: t(`categories.presets.${id}.description`),
      custom: { name: false, description: false },
      color: PRESET_COLORS[id],
      order,
      inbox: true
    }));
  }

  // The five presets, the first time categories are turned on.
  seed() {
    if (this.settings().seeded) return;
    PRESETS.forEach((id, order) => {
      if (this.account.get('category', id) === undefined) this.account.put('category', id, { name: null, description: null, color: PRESET_COLORS[id], order });
    });
    this.putSettings({ seeded: true });
  }

  state() {
    const s = this.settings();
    const list = this.list();
    return {
      enabled: s.enabled,
      unsure: list.some((c) => c.id === s.unsure) ? s.unsure : (list[0] && list[0].id) || null,
      categories: list,
      jev: { on: Boolean(this.local.jev && this.local.jev.on), hasKey: Boolean(this.local.jev && this.local.jev.key), error: (this.local.jev && this.local.jev.error) || null, model: jevApi.MODEL },
      // Senders that always go to one category, for Settings.
      senders: this.account
        .list('sender')
        .filter(({ v }) => v && list.some((c) => c.id === v.category))
        .map(({ k, v }) => ({ address: k, category: v.category }))
        .sort((a, b) => a.address.localeCompare(b.address)),
      // Categories whose new mail sends no notification on this computer.
      muted: list.filter((c) => this.mutedIds().includes(c.id)).map((c) => c.id),
      signedIn: this.account.signedIn,
      limit: MAX_CATEGORIES
    };
  }

  setEnabled(on) {
    if (on) this.seed();
    this.putSettings({ enabled: Boolean(on) });
    this.changed();
    if (on) this.soon(0, { user: true });
    return this.state();
  }

  setUnsure(id) {
    if (!this.list().some((c) => c.id === id)) throw new Error('invalid');
    this.putSettings({ unsure: id });
    this.changed();
    return this.state();
  }

  // A new category (no id) or changes to one. name or description null goes back to a preset's own. inbox false
  // keeps its mail out of Inbox itself; shown, the record leaves the field out, as records before it did. The state,
  // with the id of the category saved (saved).
  saveCategory({ id = null, name, description, color, inbox } = {}) {
    const list = this.list();
    const was = id ? this.account.get('category', id) : undefined;
    if (id && was === undefined) throw new Error('invalid');
    if (!id && list.length >= MAX_CATEGORIES) throw new Error(t('categories.errors.tooMany', { count: MAX_CATEGORIES }));
    const key = id || `c-${crypto.randomBytes(5).toString('hex')}`;
    const clean = (v, max) => (typeof v === 'string' && v.trim() ? v.trim().slice(0, max) : null);
    const value = {
      ...(was || {}),
      name: name === undefined ? (was && was.name) || null : clean(name, 40),
      description: description === undefined ? (was && was.description) || null : clean(description, 400),
      color: typeof color === 'string' && /^#[0-9a-f]{6}$/i.test(color) ? color : (was && was.color) || COLORS[list.length % COLORS.length],
      order: was ? Number(was.order) || 0 : Math.max(-1, ...list.map((c) => c.order)) + 1
    };
    if (inbox === false) value.inbox = false;
    else if (inbox === true) delete value.inbox;
    if (!PRESETS.includes(key) && !value.name) throw new Error(t('categories.errors.needName'));
    this.account.put('category', key, value);
    this.changed();
    return { ...this.state(), saved: key };
  }

  removeCategory(id) {
    const list = this.list();
    if (!list.some((c) => c.id === id)) throw new Error('invalid');
    if (list.length <= 1) throw new Error(t('categories.errors.last'));
    this.account.remove('category', id);
    if (this.settings().unsure === id) this.putSettings({ unsure: list.find((c) => c.id !== id).id });
    if (this.mutedIds().includes(id)) {
      this.local.muted = this.mutedIds().filter((k) => k !== id);
      this.saveLocal();
    }
    this.changed();
    return this.state();
  }

  reorder(ids) {
    const list = this.list();
    if (ids.length !== list.length || !list.every((c) => ids.includes(c.id))) throw new Error('invalid');
    ids.forEach((id, order) => {
      const v = this.account.get('category', id);
      if (Number(v.order) !== order) this.account.put('category', id, { ...v, order });
    });
    this.changed();
    return this.state();
  }

  // ---- which category an email is in ----

  // The key an email's records are stored under: the account's address and the email's Message-ID.
  // A Message-ID is the sender's to choose, also its length: a very long one is kept as its hash, so no email can
  // make a record too large to sync.
  key(acc, m) {
    const sha = (text) => crypto.createHash('sha256').update(text).digest('hex');
    const id = !m.messageId
      ? `h:${sha(`${lower(m.from && m.from.address)}\n${m.date}\n${m.subject || ''}`)}`
      : m.messageId.length > MESSAGE_ID_MAX
        ? `m:${sha(m.messageId)}`
        : m.messageId;
    return `${lower(acc.email)}\n${id}`;
  }

  own(acc) {
    return [acc.email, ...((acc.aliases || []).map((a) => a.address))].filter(Boolean);
  }

  // Addresses the person has written to, from the Sent folders; worked out again at most every half minute.
  writtenTo() {
    if (this.written && Date.now() - this.written.at < 30000) return this.written.set;
    const set = new Set();
    for (const cache of this.engine.caches.values()) {
      for (const [folderPath, box] of Object.entries(cache.boxes)) {
        if ((cache.folders.find((f) => f.path === folderPath) || {}).role !== 'sent') continue;
        for (const m of box.messages) for (const a of [...(m.to || []), ...(m.cc || [])]) if (a && a.address) set.add(lower(a.address));
      }
    }
    this.written = { set, at: Date.now() };
    return set;
  }

  // The category the rules give an email, among the ones that exist: { category, strength }. order: the ids in order.
  byRules(acc, m, ids, unsure, order = this.list().map((c) => c.id)) {
    const known = this.writtenTo().has(lower(m.from && m.from.address));
    const { preset, strength } = presetFor(m, { own: this.own(acc), vip: this.engine.isVip(m.from), known }, order);
    return ids.has(preset) ? { category: preset, strength } : { category: unsure, strength: 'weak' };
  }

  // The categories as placing an email needs them, worked out once for many: { ids, order, unsure }. Null while
  // categories are off.
  context() {
    const s = this.settings();
    if (!s.enabled) return null;
    const order = this.list().map((c) => c.id);
    if (!order.length) return null;
    const ids = new Set(order);
    return { ids, order, unsure: ids.has(s.unsure) ? s.unsure : order[0] };
  }

  // An inbox email's category id, or null when categories are off. Called for every message the list shows.
  of(acc, m, ctx = this.context()) {
    if (!ctx) return null;
    const { ids, order, unsure } = ctx;
    const key = this.key(acc, m);
    for (const [collection, k] of [['override', key], ['sender', lower(m.from && m.from.address)], ['classification', key]]) {
      const v = this.account.get(collection, k);
      if (v && ids.has(v.category)) return v.category;
    }
    return this.byRules(acc, m, ids, unsure, order).category;
  }

  // Puts emails in a category, by id; with sender, also every later email from their senders.
  move(ids, category, { sender = false } = {}) {
    if (!this.list().some((c) => c.id === category)) throw new Error('invalid');
    const senders = new Set();
    for (const id of ids) {
      const { acc, msg } = this.engine.locate(id);
      this.account.put('override', this.key(acc, msg), { category, at: Date.now() });
      if (sender && msg.from && msg.from.address) senders.add(lower(msg.from.address));
    }
    for (const address of senders) this.account.put('sender', address, { category, at: Date.now() });
    this.changed();
    return true;
  }

  // Forgets a sender's rule.
  forgetSender(address) {
    this.account.remove('sender', lower(address));
    this.changed();
    return true;
  }

  // ---- sorting ----

  // A sort in a moment. Every mail update moves it, but a sort someone asked for stays one.
  soon(delay = SORT_AFTER, { user = false } = {}) {
    clearTimeout(this.timer);
    this.soonUser = this.soonUser || user;
    this.timer = setTimeout(() => {
      const asked = this.soonUser;
      this.soonUser = false;
      this.sort({ user: asked }).catch(() => {});
    }, delay);
    if (this.timer.unref) this.timer.unref();
  }

  // Inbox emails nobody has sorted yet, newest first: [{ id, acc, m, key }].
  unsorted({ again = false } = {}) {
    const out = [];
    for (const acc of this.engine.accounts) {
      const cache = this.engine.caches.get(acc.id);
      if (!cache) continue;
      for (const f of cache.folders.filter((x) => x.role === 'inbox')) {
        const box = cache.boxes[f.path];
        if (!box) continue;
        for (const m of box.messages) {
          const key = this.key(acc, m);
          if (!this.toSort(key, again)) continue;
          out.push({ id: encodeId(acc.id, f.path, m.uid), acc, m, key });
        }
      }
    }
    return out.sort((a, b) => b.m.date - a.m.date);
  }

  // Whether a sort takes up an email, by its key: not one a person moved, and not sorted yet, unless it is sorted
  // again (again, or still on the re-sort list). unsorted() and jevSoon() both go by this.
  toSort(key, again = false) {
    if (this.account.get('override', key) !== undefined) return false;
    return again || this.redo.has(key) || this.account.get('classification', key) === undefined;
  }

  // Whether this computer sorts with Jev: on, with a key. Mail Jev should sort waits for it, also while TypeSafe
  // refuses the key or is busy; the rules show a category for it meanwhile (of()), without storing one.
  jevWanted() {
    const j = this.local.jev;
    return Boolean(j && j.on && j.key);
  }

  // The key to call Jev with now: none after TypeSafe refused it (until a new one) or while TypeSafe is busy.
  jevKey() {
    const j = this.local.jev;
    if (!this.jevWanted() || j.error === 'key' || Date.now() < this.jevBusyUntil) return null;
    try {
      return this.secrets.decrypt(j.key);
    } catch (_) {
      return null;
    }
  }

  // Sorts what isn't sorted yet. again: sorts every inbox email again, apart from the ones people moved themselves.
  // A sort asked for while one runs follows it, sorting again if any of those asks did, so nothing asked for is lost:
  // "Sort the inbox again" during an ordinary sort, or the sort a new Jev key starts. The promise covers both.
  sort({ again = false, user = false } = {}) {
    if (!this.settings().enabled) return Promise.resolve(0);
    if (this.sorting) {
      this.followUp = { again: Boolean(this.followUp && this.followUp.again) || again, user: Boolean(this.followUp && this.followUp.user) || user };
      return this.sorting;
    }
    // This sort does what the one soon() has waiting would: that one isn't needed anymore, and its asker is this one's.
    clearTimeout(this.timer);
    this.timer = null;
    user = user || this.soonUser;
    this.soonUser = false;
    this.sorting = (async () => {
      let total = 0;
      for (let ask = { again, user }; ask; ask = this.followUp) {
        this.followUp = null;
        if (!this.settings().enabled) break;
        // Set before run() stores anything, which it can do before its first await (recordsChanged()).
        this.running = true;
        try {
          total += await this.run(ask.again, ask.user);
        } finally {
          this.running = false;
        }
      }
      return total;
    })().finally(() => {
      this.sorting = null;
    });
    return this.sorting;
  }

  async run(again, user = false) {
    const list = this.list();
    if (!list.length) return 0;
    const ids = new Set(list.map((c) => c.id));
    const s = this.settings();
    const unsure = ids.has(s.unsure) ? s.unsure : list[0].id;
    const wanted = this.jevWanted();
    const key = this.jevKey();
    const generation = this.jevGeneration;
    // Sorting again replaces what an email had; otherwise the first stored category stays (first write wins).
    const replace = (k) => again || this.redo.has(k);
    const store = (k, value) => {
      const done = replace(k) ? this.account.put('classification', k, value) : this.account.put('classification', k, value, { ifAbsent: true });
      this.redo.delete(k);
      return done;
    };
    const later = (k) => {
      if (replace(k)) this.redo.add(k);
    };
    const forJev = [];
    let sorted = 0;
    let byRules = 0;
    let waiting = 0;
    for (const item of this.unsorted({ again })) {
      // Cached before Rukoo kept the mail-kind headers: the next sync brings them, and until then the rules would
      // guess with less than they will have. The rules still show a category for it meanwhile.
      if (!item.m.signals) continue;
      const rule = this.byRules(item.acc, item.m, ids, unsure);
      if (rule.strength === 'strong' || !wanted) {
        byRules++;
        if (store(item.key, { category: rule.category, source: 'rules', at: Date.now() }) !== false) sorted++;
      } else if (key) forJev.push(item);
      else {
        waiting++;
        later(item.key);
      }
    }
    const p = this.progress;
    // A sort nobody asked for with nothing to do isn't news; one that waits for Jev only updates how many wait.
    if (user || byRules + forJev.length) {
      Object.assign(p, { running: true, user, again, total: byRules + forJev.length, done: byRules, asked: forJev.length, answered: 0, sorted, waiting, startedAt: Date.now(), finishedAt: 0 });
      this.progressChanged(true);
    } else if (p.waiting !== waiting) {
      p.waiting = waiting;
      this.progressChanged(true);
    }
    // A re-sort's emails are on file as still to do before Jev is asked, so one the app quits during goes on after a
    // restart. Each leaves the list once its answer is stored.
    const pending = forJev.filter((item) => replace(item.key));
    if (pending.length) {
      for (const item of pending) this.redo.add(item.key);
      this.saveLocal();
    }
    let stop = null;
    // Turning Jev or categories off, or a new key, stops what is still queued: nothing more goes to TypeSafe.
    let cancelled = false;
    const next = async () => {
      while (forJev.length && !stop && !cancelled) {
        if (!this.settings().enabled || this.jevGeneration !== generation || this.jevKey() !== key) {
          cancelled = true;
          break;
        }
        const { id, acc, m, key: k } = forJev.shift();
        try {
          const own = this.own(acc).map(lower);
          const toMe = (m.to || []).some((a) => own.includes(lower(a.address)));
          const res = await this.jev.classify(key, m, list, { toMe });
          // Settings changed while the answer was on its way (Jev or categories off, a new key): it isn't stored,
          // and the email waits for the sort that follows.
          if (!this.settings().enabled || this.jevGeneration !== generation) {
            later(k);
            cancelled = true;
            break;
          }
          const sure = res.confidence >= JEV_CONFIDENT;
          const category = sure ? res.category : unsure;
          if (store(k, { category, source: 'jev', confidence: res.confidence, model: res.model, at: Date.now() }) !== false) {
            sorted++;
            this.live();
          }
          p.answered++;
          p.done++;
          p.sorted = sorted;
          // An email sorted again shows once, with its newest decision.
          const decision = { id, subject: m.subject || '', from: (m.from && (m.from.name || m.from.address)) || '', category, chose: res.category, confidence: res.confidence, sure, at: Date.now() };
          this.recent = [decision, ...this.recent.filter((r) => r.id !== id)].slice(0, RECENT);
          this.progressChanged();
        } catch (err) {
          later(k);
          stop = err;
        } finally {
          // The email's notification may wait for this answer (notices.js).
          this.asking.delete(k);
          this.emit('sorted', k);
        }
      }
    };
    const asked = forJev.length;
    for (const item of forJev) this.asking.add(item.key);
    await Promise.all(Array.from({ length: JEV_AT_ONCE }, next));
    // What a stop or a change of settings left in the queue; the change says so itself ('reconsider').
    this.asking.clear();
    for (const item of forJev) later(item.key);
    // Records the sort's pace still held back, its own or another device's, show with the end of the sort.
    const held = Boolean(this.liveTimer);
    clearTimeout(this.liveTimer);
    this.liveTimer = null;
    if (p.running) Object.assign(p, { running: false, sorted, waiting: this.jevWanted() ? waiting + (p.asked - p.answered) : 0, finishedAt: Date.now() });
    if (this.jevGeneration === generation) {
      if (stop) this.jevFailed(stop);
      // Jev answered again after being busy or out of reach. A refused key only clears with a new key (setJev).
      else if (asked && this.local.jev && (this.local.jev.error === 'busy' || this.local.jev.error === 'network')) this.setJevError(null);
    }
    // Mail waits for Jev while TypeSafe is busy: make sure a sort follows the wait.
    if (wanted && !key && Date.now() < this.jevBusyUntil && !this.retryTimer) this.retryAfter(this.jevBusyUntil - Date.now() + 1000);
    if (this.redo.size || (this.local.redo && this.local.redo.length)) this.saveLocal();
    if (sorted || held) this.changed();
    this.progressChanged(true);
    return sorted;
  }

  // While Jev sorts, what it stored shows every few seconds instead of only at the end.
  live() {
    if (this.liveTimer) return;
    this.liveTimer = setTimeout(() => {
      this.liveTimer = null;
      this.changed();
    }, LIVE_EVERY);
    if (this.liveTimer.unref) this.liveTimer.unref();
  }

  // What the sorting is doing, for the sidebar: the progress of the last sort, how many emails wait for Jev and
  // why, and Jev's last decisions.
  activity() {
    const j = this.local.jev || {};
    const jev = this.jevWanted();
    return {
      ...this.progress,
      jev,
      error: jev ? j.error || null : null,
      retryAt: jev && this.jevBusyUntil > Date.now() ? this.jevBusyUntil : 0,
      recent: this.recent.slice()
    };
  }

  // Tells the window, at most a few times a second while Jev answers; now: straight away.
  progressChanged(now = false) {
    if (now) {
      clearTimeout(this.progressTimer);
      this.progressTimer = null;
      this.emit('progress', this.activity());
      return;
    }
    if (this.progressTimer) return;
    this.progressTimer = setTimeout(() => {
      this.progressTimer = null;
      this.emit('progress', this.activity());
    }, 150);
    if (this.progressTimer.unref) this.progressTimer.unref();
  }

  // A wrong key waits for a new one; TypeSafe being busy or out of reach waits a few minutes. The rules keep showing
  // a category for every email in the meantime.
  jevFailed(err) {
    if (err && err.code === 'key') this.setJevError('key');
    else {
      const wait = 5 * 60 * 1000;
      this.jevBusyUntil = Date.now() + wait;
      this.setJevError(err && err.code === 'busy' ? 'busy' : 'network');
      this.retryAfter(wait + 1000);
    }
  }

  retryAfter(ms) {
    clearTimeout(this.retryTimer);
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      this.sort().catch(() => {});
    }, ms);
    if (this.retryTimer.unref) this.retryTimer.unref();
  }

  setJevError(code) {
    this.local.jev = { ...(this.local.jev || {}), error: code };
    this.saveLocal();
    this.emit('changed');
    this.emit('reconsider');
    this.progressChanged(true);
  }

  // Jev on this computer: on, off, or a new key. The key stays here, encrypted like mail passwords.
  setJev({ on, key } = {}) {
    const j = { ...(this.local.jev || {}) };
    if (typeof key === 'string' && key.trim()) j.key = this.secrets.encrypt(key.trim());
    if (key === null) delete j.key;
    if (typeof on === 'boolean') j.on = on && Boolean(j.key);
    j.error = null;
    this.local.jev = j;
    this.jevBusyUntil = 0;
    this.jevGeneration += 1;
    clearTimeout(this.retryTimer);
    this.retryTimer = null;
    this.saveLocal();
    this.emit('changed');
    this.emit('reconsider');
    // Jev off, or a new key: what was waiting for Jev is the next sort's to count.
    if (!j.on) this.progress.waiting = 0;
    this.progressChanged(true);
    if (j.on) this.soon(0, { user: true });
    return this.state();
  }

  sortAgain() {
    return this.sort({ again: true, user: true });
  }

  // ---- notifications ----

  // Categories whose new mail sends no desktop notification. On this computer only, like the Notifications setting,
  // so a phone and a computer can differ.
  mutedIds() {
    return Array.isArray(this.local.muted) ? this.local.muted.filter((k) => typeof k === 'string') : [];
  }

  setNotify(id, on) {
    if (!this.list().some((c) => c.id === id)) throw new Error('invalid');
    const ids = this.mutedIds().filter((k) => k !== id);
    this.local.muted = on ? ids : [...ids, id];
    this.saveLocal();
    this.emit('changed');
    this.emit('reconsider');
    return this.state();
  }

  // What deciding notifications needs, worked out once for many emails (notices.js): context(), the muted categories
  // that exist (none while categories are off), whether Jev can be asked, and whether a re-sort is to follow.
  noticeContext() {
    const ctx = this.context();
    const ids = this.mutedIds();
    const muted = new Set(ctx ? ctx.order.filter((id) => ids.includes(id)) : []);
    return { ...ctx, muted, jev: Boolean(muted.size && this.jevKey()), again: Boolean(this.followUp && this.followUp.again) };
  }

  // What a new inbox email's notification does (notices.js), with the email's key: 'wait' while Jev's answer for it is
  // still to come, 'mute' when its category is muted, else 'show'. m: the engine's 'new-mail' message. jev false:
  // the category it has now decides.
  notice(m, { jev = true, ctx = this.noticeContext() } = {}) {
    if (!ctx.muted.size) return { what: 'show', key: null };
    let found = null;
    try {
      found = this.engine.locate(m.id);
    } catch (_) {}
    // Gone from the inbox meanwhile: the category it came with.
    if (!found || !found.msg) return { what: ctx.muted.has(m.category) ? 'mute' : 'show', key: null };
    const key = this.key(found.acc, found.msg);
    if (jev && this.jevSoon(found.acc, found.msg, key, ctx)) return { what: 'wait', key };
    return { what: ctx.muted.has(this.of(found.acc, found.msg, ctx)) ? 'mute' : 'show', key };
  }

  // Whether Jev's answer for an email is still to come: the running sort has it for Jev, or the next sort takes it up
  // (toSort(), as unsorted() does) and sends it to Jev because the rules aren't sure. A correction or a sender rule
  // places the email whatever Jev answers.
  jevSoon(acc, m, key, ctx) {
    if (!ctx.jev) return false;
    if (this.account.get('override', key) !== undefined) return false;
    const sender = this.account.get('sender', lower(m.from && m.from.address));
    if (sender && ctx.ids.has(sender.category)) return false;
    if (this.asking.has(key)) return true;
    if (!m.signals || !this.toSort(key, ctx.again)) return false;
    return this.byRules(acc, m, ctx.ids, ctx.unsure, ctx.order).strength !== 'strong';
  }

  close() {
    clearTimeout(this.timer);
    clearTimeout(this.retryTimer);
    clearTimeout(this.progressTimer);
    clearTimeout(this.liveTimer);
    clearImmediate(this.recordsTimer);
    this.recordsTimer = null;
  }
}

module.exports = { Categories, MAX_CATEGORIES, JEV_CONFIDENT };
