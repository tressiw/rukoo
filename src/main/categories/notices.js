'use strict';

// Desktop notifications for new inbox mail, by category. A category muted on this computer sends none. An email Jev
// is about to sort, because the rules aren't sure about it, waits for Jev's answer, at most HOLD, and then goes by the
// category it got. With nothing muted, all new mail notifies at once, as without categories.
//
// A batch of new mail notifies as one, also when part of it waits: the first AT_ONCE one by one, then one that counts
// the batch. What is decided at arrival shows at once; the rest uses what is left of the batch's AT_ONCE, and the
// count comes once, at the end. So an email Jev already answered waits for the slower ones in its batch, at most HOLD.
const { t } = require('../../i18n');

// How long a notification waits for Jev; after that, the category the email has then decides.
const HOLD = 20000;
// Notifications one by one for one batch of new mail; a longer batch adds one that counts it.
const AT_ONCE = 3;

// An email's own notification, which opens it.
const single = (m) => ({ id: m.id, title: m.from.name || m.from.address, body: [m.subject, m.preview].filter(Boolean).join('\n').slice(0, 180) });
// The one that counts a batch.
const counted = (count) => ({ id: null, title: 'Rukoo Mail', body: t('native.notifications.newCount', { count }) });

// What to show for new mail: the first few, each opening its email (id), then one that counts them all (id null).
function mailNotices(messages) {
  const list = messages.slice(0, AT_ONCE).map(single);
  if (messages.length > list.length) list.push(counted(messages.length));
  return list;
}

class MailNotices {
  // categories: the Categories (index.js). show(notices, messages): puts notices like mailNotices() on screen, for the
  // emails in messages, the ones of the batch that notify now.
  constructor({ categories, show, hold = HOLD }) {
    this.categories = categories;
    this.show = show;
    this.hold = hold;
    // Batches with emails waiting for Jev: { held, waiting, shown, count, timer }. held: those emails; waiting: the
    // ones still waiting, as { batch, m, key }; shown: notifications one by one so far; count: emails notified so far.
    this.batches = new Set();
    // The waiting emails by key, so Jev's answer for one looks at that one only.
    this.byKey = new Map();
    this.answered = (key) => this.recheck([...(this.byKey.get(key) || [])]);
    // Muted categories, Jev or categories turned on or off, a category removed: every waiting email.
    this.reconsider = () => this.recheck([...this.byKey.values()].flatMap((set) => [...set]));
    categories.on('sorted', this.answered);
    categories.on('reconsider', this.reconsider);
  }

  // A batch of new inbox mail (the engine's 'new-mail'). What Jev is about to sort waits; the rest is decided now.
  add(messages) {
    const ctx = this.categories.noticeContext();
    if (!ctx.muted.size) return this.notify(messages);
    const now = [];
    const batch = { held: [], waiting: new Set(), shown: 0, count: 0, timer: null };
    for (const m of messages) {
      const { what, key } = this.categories.notice(m, { ctx });
      if (what === 'show') now.push(m);
      if (what !== 'wait') continue;
      batch.held.push(m);
      const entry = { batch, m, key };
      batch.waiting.add(entry);
      if (!this.byKey.has(key)) this.byKey.set(key, new Set());
      this.byKey.get(key).add(entry);
    }
    if (!batch.held.length) return this.notify(now);
    const list = now.slice(0, AT_ONCE).map(single);
    if (list.length) this.show(list, now);
    Object.assign(batch, { shown: list.length, count: now.length });
    batch.timer = setTimeout(() => this.release(batch), this.hold);
    if (batch.timer.unref) batch.timer.unref();
    this.batches.add(batch);
  }

  // Waiting emails Jev may no longer have to answer for; a batch goes once none of it waits.
  recheck(entries) {
    if (!entries.length) return;
    const ctx = this.categories.noticeContext();
    const done = new Set();
    for (const entry of entries) {
      if (this.categories.notice(entry.m, { ctx }).what === 'wait') continue;
      this.drop(entry);
      if (!entry.batch.waiting.size) done.add(entry.batch);
    }
    for (const batch of done) this.release(batch, ctx);
  }

  drop(entry) {
    entry.batch.waiting.delete(entry);
    const same = this.byKey.get(entry.key);
    if (same && same.delete(entry) && !same.size) this.byKey.delete(entry.key);
  }

  // The rest of a batch, by the category each email has now: what is left of AT_ONCE, and the count for the batch.
  release(batch, ctx = this.categories.noticeContext()) {
    if (!this.batches.delete(batch)) return;
    clearTimeout(batch.timer);
    for (const entry of [...batch.waiting]) this.drop(entry);
    const shows = batch.held.filter((m) => this.categories.notice(m, { jev: false, ctx }).what === 'show');
    const list = shows.slice(0, AT_ONCE - batch.shown).map(single);
    const count = batch.count + shows.length;
    if (count > batch.shown + list.length) list.push(counted(count));
    if (list.length) this.show(list, shows);
  }

  notify(messages) {
    if (messages.length) this.show(mailNotices(messages), messages);
  }

  close() {
    for (const batch of this.batches) clearTimeout(batch.timer);
    this.batches.clear();
    this.byKey.clear();
    this.categories.off('sorted', this.answered);
    this.categories.off('reconsider', this.reconsider);
  }
}

module.exports = { MailNotices, mailNotices, HOLD };
