'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { EventEmitter } = require('events');
const { mailSignals } = require('../src/main/categories/signals');
const { presetFor, looksLikePerson } = require('../src/main/categories/rules');
const jev = require('../src/main/categories/jev');
const { Categories } = require('../src/main/categories');
const { MailNotices, mailNotices, HOLD } = require('../src/main/categories/notices');
const { AccountService } = require('../src/main/account');

const secrets = { encrypt: (s) => `plain:${Buffer.from(s).toString('base64')}`, decrypt: (s) => Buffer.from(String(s).slice(6), 'base64').toString() };

function tmpDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rukoo-categories-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test('mail headers become a few signals', () => {
  const raw = Buffer.from(
    'List-Unsubscribe: <https://example.com/u>,\r\n <mailto:u@example.com>\r\nPrecedence: bulk\r\nX-Mailer: MailChimp Mailer\r\nAuto-Submitted: no\r\n'
  );
  assert.deepEqual(mailSignals(raw), { list: true, bulk: true, campaign: true });
  assert.deepEqual(mailSignals({ 'Auto-Submitted': 'auto-generated', 'Feedback-ID': '1:2:3' }), { bulk: true, auto: true });
  assert.deepEqual(mailSignals(undefined), {});
  assert.deepEqual(mailSignals('Subject: hello\r\n'), {});
});

test('the rules place mail by what it says about itself', () => {
  const own = ['me@work.example'];
  const msg = (from, extra = {}) => ({ from, to: [{ address: 'me@work.example' }], subject: 'Hello', ...extra });
  const pick = (m, opts = {}) => presetFor(m, { own, ...opts });

  assert.deepEqual(pick(msg({ name: 'Boss', address: 'boss@corp.example' }), { vip: true }), { preset: 'needs-you', strength: 'strong' });
  assert.equal(pick(msg({ name: 'Shop', address: 'hi@shop.example' }, { signals: { campaign: true } })).preset, 'promotions');
  assert.equal(pick(msg({ name: 'Shop', address: 'hi@shop.example' }, { gmailTab: 'promotions' })).preset, 'promotions');
  assert.equal(pick(msg({ name: 'ANWB', address: 'newsletter@anwb.nl' })).preset, 'promotions');
  assert.equal(pick(msg({ name: 'Energy', address: 'hi@energy.example' }, { subject: 'Your invoice for October' })).preset, 'money');
  assert.equal(pick(msg({ name: 'Energie', address: 'hi@energie.example' }, { subject: 'Je factuur van oktober' })).preset, 'money');
  assert.deepEqual(pick(msg({ name: 'App', address: 'no-reply@app.example' })), { preset: 'updates', strength: 'strong' });
  assert.equal(pick(msg({ name: 'App', address: 'hi@app.example' }, { signals: { auto: true } })).preset, 'updates');
  assert.equal(pick(msg({ name: 'Forum', address: 'hi@forum.example' }, { gmailTab: 'forums' })).preset, 'updates');
  // A newsletter, unless the sender is someone you write to.
  assert.equal(pick(msg({ name: 'Club', address: 'board@club.example' }, { signals: { list: true } })).preset, 'promotions');
  assert.equal(pick(msg({ name: 'Club', address: 'board@club.example' }, { signals: { list: true } }), { known: true }).preset, 'needs-you');
  assert.equal(pick(msg({ name: 'Bank', address: 'service@bank.example' }, { signals: { bulk: true } })).preset, 'updates');
  // People: to you directly needs you, copied is to read.
  assert.equal(pick(msg({ name: 'Sanne de Vries', address: 'sanne@studio.example' })).preset, 'needs-you');
  assert.equal(pick({ ...msg({ name: 'Sanne de Vries', address: 'sanne@studio.example' }), to: [{ address: 'team@studio.example' }], cc: [{ address: 'me@work.example' }] }).preset, 'to-read');
  assert.equal(pick(msg({ name: 'Colleague', address: 'x@work.example' })).preset, 'needs-you');
  assert.equal(pick(msg({ name: 'Friend', address: 'friend@gmail.com' })).preset, 'needs-you');
  assert.equal(pick(msg({ name: 'Vandebron', address: 'support@vandebron.nl' })).preset, 'updates');
  assert.equal(pick(msg({ name: 'Vandebron', address: 'support@vandebron.nl' }), { known: true }).preset, 'needs-you');
});

test('when the rules find several categories, the one higher in the person\'s order wins', () => {
  const m = { from: { name: 'Shop', address: 'hi@shop.example' }, to: [], subject: 'Your invoice', signals: { campaign: true } };
  assert.equal(presetFor(m, {}).preset, 'money', 'Money is above Promotions by default');
  assert.equal(presetFor(m, {}, ['promotions', 'money']).preset, 'promotions');
  const alert = { from: { name: 'Bank', address: 'noreply@bank.example' }, to: [], subject: 'Payment received' };
  assert.equal(presetFor(alert, {}).preset, 'money');
  assert.equal(presetFor(alert, {}, ['needs-you', 'updates', 'money']).preset, 'updates');
});

test('mail cached before its headers were fetched waits to be sorted', async (t) => {
  const msgs = inbox();
  delete msgs[0].signals;
  const { cats, acc, account } = setup(t, msgs);
  cats.setEnabled(true);
  await cats.sort();
  assert.equal(account.get('classification', cats.key(acc, msgs[0])), undefined);
  assert.equal(cats.of(acc, msgs[0]), 'needs-you', 'the rules show a category meanwhile');
  msgs[0].signals = { campaign: true };
  await cats.sort();
  assert.equal(account.get('classification', cats.key(acc, msgs[0])).category, 'promotions');
});

test('a person is a name of two words that matches the address', () => {
  assert.equal(looksLikePerson({ name: 'Sanne de Vries', address: 'sanne.devries@x.example' }), true);
  assert.equal(looksLikePerson({ name: 'Joris Bakker', address: 'jbakker@x.example' }), true);
  assert.equal(looksLikePerson({ name: 'José Müller', address: 'jose@x.example' }), true);
  assert.equal(looksLikePerson({ name: 'Matthew Berman', address: 'newsletter@x.example' }), false);
  assert.equal(looksLikePerson({ name: 'Vandebron', address: 'vandebron@x.example' }), false);
});

test('Jev gets one choice question with the categories as options', async () => {
  const sent = [];
  const request = (url, opts, cb) => {
    const req = new EventEmitter();
    req.write = (body) => sent.push({ url, opts, body: JSON.parse(body) });
    req.end = () => {
      const res = new EventEmitter();
      res.statusCode = 200;
      cb(res);
      const question = sent[0].body.questions.category;
      const choice = Object.keys(question.criteria)[2];
      res.emit('data', Buffer.from(JSON.stringify({ model: 'jev-1.13.0', answers: { category: { type: 'choice', choice, confidence: 0.82, probabilities: {} } } })));
      res.emit('end');
    };
    return req;
  };
  const categories = [
    { id: 'needs-you', name: 'Needs you', description: 'Asks you to act' },
    { id: 'c-1', name: 'Needs you', description: 'A duplicate name' },
    { id: 'money', name: 'Geldzaken', description: 'Facturen' }
  ];
  const m = { from: { name: 'A', address: 'a@x.example' }, subject: 'Factuur', preview: 'x'.repeat(5000), signals: { list: true }, gmailTab: 'updates' };
  const res = await jev.classify('KEY', m, categories, { toMe: true, request });
  assert.deepEqual(res, { category: 'money', confidence: 0.82, model: 'jev-1.13.0' });
  const { body, opts, url } = sent[0];
  assert.equal(url, 'https://api.typesafe.ai/v1/systemone');
  assert.equal(opts.headers.authorization, 'Bearer KEY');
  assert.equal(body.model, jev.MODEL);
  assert.deepEqual(Object.keys(body.questions.category.criteria), ['needs_you', 'needs_you_2', 'geldzaken']);
  assert.equal(body.state.text.length, jev.TEXT_CHARS);
  assert.equal(body.state.addressed_to_me, true);
  assert.equal(body.state.mailing_list, true);
  assert.equal(body.state.gmail_tab, 'updates');
});

test("Jev's model name is kept only when it looks like one, since every classification stores it", async () => {
  const answering = (model) => (url, opts, cb) => {
    const req = new EventEmitter();
    req.write = () => {};
    req.end = () => {
      const res = new EventEmitter();
      res.statusCode = 200;
      cb(res);
      res.emit('data', Buffer.from(JSON.stringify({ ...(model === undefined ? {} : { model }), answers: { category: { choice: 'a', confidence: 0.9 } } })));
      res.emit('end');
    };
    return req;
  };
  const model = async (m) => (await jev.classify('K', {}, [{ id: 'a', name: 'A' }], { request: answering(m) })).model;
  assert.equal(await model('jev-1.14.0'), 'jev-1.14.0');
  assert.equal(await model('jev-latest'), 'jev-latest');
  assert.equal(await model('j'.repeat(40)), 'j'.repeat(40));
  // Most of a megabyte, one character too long, other characters, empty, not a string, missing.
  for (const odd of ['j'.repeat(900000), 'j'.repeat(41), 'Jev-1.13.0', 'jev 1.13', 'jev_1.13', 'jev/1.13', 'jev-1.13\n', '', 42, ['jev'], null, undefined]) {
    assert.equal(await model(odd), jev.MODEL, `model ${JSON.stringify(odd)?.slice(0, 40)}`);
  }
});

test('a Jev answer cut off halfway is a network error, not a wait forever', async (t) => {
  const http = require('http');
  const server = http.createServer((req, res) => {
    req.resume();
    res.writeHead(200, { 'content-type': 'application/json', 'content-length': '500' });
    res.write('{"answers":');
    setTimeout(() => res.socket.destroy(), 20);
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  t.after(() => server.close());
  const { port } = server.address();
  const request = (url, opts, cb) => http.request(`http://127.0.0.1:${port}/`, opts, cb);
  await assert.rejects(jev.classify('K', {}, [{ id: 'a', name: 'A' }], { request }), { code: 'network' });
});

test('a Jev answer larger than 1 MiB is not read into memory', async (t) => {
  const http = require('http');
  const server = http.createServer((req, res) => {
    req.resume();
    res.writeHead(200, { 'content-type': 'application/json' });
    res.on('error', () => {});
    // A valid answer, padded to 2 MiB.
    res.end(`{"answers":{"category":{"choice":"a","confidence":0.9}}}${' '.repeat(2 * jev.MAX_ANSWER)}`);
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  t.after(() => {
    server.closeAllConnections();
    server.close();
  });
  const { port } = server.address();
  const request = (url, opts, cb) => http.request(`http://127.0.0.1:${port}/`, opts, cb);
  await assert.rejects(jev.classify('K', {}, [{ id: 'a', name: 'A' }], { request }), { code: 'server' });
});

test('Jev errors say what went wrong', async () => {
  const failing = (status) => (url, opts, cb) => {
    const req = new EventEmitter();
    req.write = () => {};
    req.end = () => {
      const res = new EventEmitter();
      res.statusCode = status;
      cb(res);
      res.emit('data', Buffer.from('{"error":"x"}'));
      res.emit('end');
    };
    return req;
  };
  const cats = [{ id: 'a', name: 'A' }];
  await assert.rejects(jev.classify('K', {}, cats, { request: failing(401) }), { code: 'key' });
  await assert.rejects(jev.classify('K', {}, cats, { request: failing(429) }), { code: 'busy' });
  await assert.rejects(jev.classify('K', {}, cats, { request: failing(500) }), { code: 'server' });
});

// An engine with one account and an inbox, enough for the categories.
function fakeEngine(messages, sent = []) {
  const engine = new EventEmitter();
  const acc = { id: 'acc1', email: 'me@work.example', aliases: [] };
  engine.accounts = [acc];
  engine.caches = new Map([
    [
      acc.id,
      {
        folders: [
          { path: 'INBOX', role: 'inbox' },
          { path: 'Sent', role: 'sent' }
        ],
        boxes: { INBOX: { messages }, Sent: { messages: sent } }
      }
    ]
  ]);
  engine.vips = new Set();
  engine.isVip = (from) => engine.vips.has(String(from && from.address).toLowerCase());
  engine.locate = (id) => ({ acc, msg: messages.find((m) => m.uid === Number(id)) });
  return { engine, acc };
}

function setup(t, messages, { jevImpl, sent } = {}) {
  const dir = tmpDir(t);
  const { engine, acc } = fakeEngine(messages, sent);
  const account = new AccountService({ dataDir: dir, secrets, defaultServer: '', deviceName: 'desk' }).init();
  const cats = new Categories({ engine, account, secrets, dataDir: dir, jev: jevImpl || jev }).init();
  t.after(() => {
    cats.close();
    account.close();
  });
  return { cats, account, engine, acc };
}

const inbox = () => [
  { uid: 1, messageId: '<1@x>', from: { name: 'Sanne de Vries', address: 'sanne@studio.example' }, to: [{ address: 'me@work.example' }], subject: 'Can you sign this?', date: 3, signals: {} },
  { uid: 2, messageId: '<2@x>', from: { name: 'Shop', address: 'news@shop.example' }, to: [{ address: 'me@work.example' }], subject: 'Sale', date: 2, signals: { list: true } },
  { uid: 3, messageId: '<3@x>', from: { name: 'App', address: 'noreply@app.example' }, to: [{ address: 'me@work.example' }], subject: 'New login', date: 1, signals: {} }
];

test('categories start off, and turning them on brings the five in order', (t) => {
  const { cats, acc, engine } = setup(t, inbox());
  assert.equal(cats.of(acc, engine.caches.get('acc1').boxes.INBOX.messages[0]), null);
  const state = cats.setEnabled(true);
  assert.deepEqual(state.categories.map((c) => c.id), ['needs-you', 'to-read', 'money', 'updates', 'promotions']);
  assert.equal(state.categories[0].name, 'Needs you');
  assert.equal(state.unsure, 'to-read');
  const [a, b, c] = engine.caches.get('acc1').boxes.INBOX.messages;
  assert.deepEqual([cats.of(acc, a), cats.of(acc, b), cats.of(acc, c)], ['needs-you', 'promotions', 'updates']);
  // Turning them off and on again doesn't bring back a deleted preset.
  cats.removeCategory('money');
  cats.setEnabled(false);
  cats.setEnabled(true);
  assert.equal(cats.state().categories.some((x) => x.id === 'money'), false);
});

test('a correction wins over a sender rule, which wins over the stored and the ruled category', (t) => {
  const { cats, acc, account, engine } = setup(t, inbox());
  cats.setEnabled(true);
  const [a] = engine.caches.get('acc1').boxes.INBOX.messages;
  const key = cats.key(acc, a);
  account.put('classification', key, { category: 'money', source: 'jev' }, { ifAbsent: true });
  assert.equal(cats.of(acc, a), 'money');
  account.put('sender', 'sanne@studio.example', { category: 'to-read' });
  assert.equal(cats.of(acc, a), 'to-read');
  cats.move(['1'], 'updates');
  assert.equal(cats.of(acc, a), 'updates');
  // A category that no longer exists is skipped.
  cats.removeCategory('updates');
  assert.equal(cats.of(acc, a), 'to-read');
});

test('moving with the sender sets a rule for later mail', (t) => {
  const { cats, acc, account, engine } = setup(t, inbox());
  cats.setEnabled(true);
  cats.move(['2'], 'to-read', { sender: true });
  assert.deepEqual(account.get('sender', 'news@shop.example').category, 'to-read');
  const later = { uid: 9, messageId: '<9@x>', from: { name: 'Shop', address: 'News@Shop.example' }, to: [], subject: 'More', date: 9, signals: { list: true } };
  assert.equal(cats.of(acc, later), 'to-read');
  assert.deepEqual(cats.state().senders, [{ address: 'news@shop.example', category: 'to-read' }]);
  cats.forgetSender('news@shop.example');
  assert.equal(cats.of(acc, later), 'promotions');
  assert.ok(engine);
});

test('sorting stores the rules result once, first write wins', async (t) => {
  const { cats, acc, account, engine } = setup(t, inbox());
  cats.setEnabled(true);
  const sorted = await cats.sort();
  assert.equal(sorted, 3);
  const [a] = engine.caches.get('acc1').boxes.INBOX.messages;
  assert.deepEqual(
    (({ category, source }) => ({ category, source }))(account.get('classification', cats.key(acc, a))),
    { category: 'needs-you', source: 'rules' }
  );
  assert.equal(await cats.sort(), 0, 'nothing left to sort');
  // Sorting again overwrites, apart from what was moved by hand.
  cats.move(['3'], 'money');
  engine.vips.add('sanne@studio.example');
  assert.equal(await cats.sortAgain(), 2);
});

test('Jev sorts the uncertain mail, and an unsure answer goes to the unsure category', async (t) => {
  const calls = [];
  const fake = {
    MODEL: 'jev-test',
    classify: async (key, m, list) => {
      calls.push({ key, subject: m.subject, options: list.map((c) => c.id) });
      return m.subject === 'Can you sign this?' ? { category: 'money', confidence: 0.9, model: 'jev-test' } : { category: 'promotions', confidence: 0.3, model: 'jev-test' };
    }
  };
  const msgs = inbox();
  // From a sender whose address doesn't say it sends offers, so only the unsubscribe link points that way.
  msgs[1].from = { name: 'Shop', address: 'hello@shop.example' };
  msgs.push({ uid: 4, messageId: '<4@x>', from: { name: 'Club', address: 'board@club.example' }, to: [{ address: 'me@work.example' }], subject: 'Meeting notes', date: 0, signals: { list: true } });
  const { cats, acc, account } = setup(t, msgs, { jevImpl: fake });
  cats.setEnabled(true);
  cats.setJev({ key: 'SECRET', on: true });
  assert.equal(cats.state().jev.on, true);
  assert.ok(!JSON.stringify(cats.local).includes('SECRET') || cats.local.jev.key.startsWith('plain:'), 'the key is stored through secrets');
  await cats.sort();
  // The noreply login alert is a strong rule and never goes to Jev.
  assert.deepEqual(calls.map((c) => c.subject).sort(), ['Can you sign this?', 'Meeting notes', 'Sale']);
  assert.equal(calls[0].key, 'SECRET');
  const get = (uid) => account.get('classification', cats.key(acc, msgs.find((m) => m.uid === uid)));
  assert.deepEqual((({ category, source, confidence }) => ({ category, source, confidence }))(get(1)), { category: 'money', source: 'jev', confidence: 0.9 });
  assert.equal(get(2).category, 'to-read', 'below the confidence line it goes to the unsure category');
  assert.equal(get(3).source, 'rules');
});

test('a sort of hundreds of emails tells the window a few times, not once per email', async (t) => {
  const msgs = Array.from({ length: 600 }, (_, i) => ({
    uid: i + 1,
    messageId: `<${i + 1}@x>`,
    from: { name: 'App', address: 'noreply@app.example' },
    to: [{ address: 'me@work.example' }],
    subject: `Login ${i}`,
    date: 600 - i,
    signals: {}
  }));
  const { cats, account, engine } = setup(t, msgs);
  let changed = 0;
  let updated = 0;
  cats.on('changed', () => changed++);
  engine.on('updated', () => updated++);
  cats.setEnabled(true);
  const before = changed;
  assert.ok(before <= 3, `turning categories on: ${before}`);
  assert.equal(await cats.sort(), 600);
  assert.equal(account.list('classification').length, 600);
  await new Promise((resolve) => setImmediate(resolve));
  assert.ok(changed - before <= 2, `the sort told the window ${changed - before} times`);
  assert.ok(updated <= changed + 2, `and the engine ${updated} times`);

  // Outside a sort, a record from another device shows at once, and a burst of them once more at its end.
  changed = 0;
  account.put('override', 'me@work.example\n<1@x>', { category: 'money', at: Date.now() });
  assert.equal(changed, 1);
  for (let i = 2; i <= 50; i++) account.put('override', `me@work.example\n<${i}@x>`, { category: 'money', at: Date.now() });
  assert.equal(changed, 1);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(changed, 2);
});

test("Jev's answers coming in one by one show at the sort's pace, and its end shows the rest", async (t) => {
  const answers = [];
  const fake = { MODEL: 'jev-test', classify: () => new Promise((resolve) => answers.push(() => resolve({ category: 'money', confidence: 0.9, model: 'jev-test' }))) };
  const msgs = inbox();
  msgs[1].from = { name: 'Shop', address: 'hello@shop.example' };
  msgs[0].from = { name: 'Sanne', address: 'sanne@studio.example' };
  const { cats } = setup(t, msgs, { jevImpl: fake });
  cats.setEnabled(true);
  cats.setJev({ key: 'SECRET', on: true });
  let changed = 0;
  cats.on('changed', () => changed++);
  const sorting = cats.sort();
  await new Promise((resolve) => setImmediate(resolve));
  assert.ok(answers.length >= 1);
  const first = changed;
  // Each answer arrives on its own turn: none of them redraws the window by itself.
  while (answers.length) {
    answers.shift()();
    await new Promise((resolve) => setImmediate(resolve));
  }
  await sorting;
  assert.equal(first, 0, 'nothing while the rules stored theirs');
  assert.equal(changed, 1, 'once, when the sort was done');
});

test("the window can follow Jev's progress and its last decisions", async (t) => {
  const answers = [];
  const fake = {
    MODEL: 'jev-test',
    classify: (key, m) =>
      new Promise((resolve) => answers.push(() => resolve(m.subject === 'Sale' ? { category: 'promotions', confidence: 0.3, model: 'jev-test' } : { category: 'money', confidence: 0.9, model: 'jev-test' })))
  };
  const msgs = inbox();
  msgs[1].from = { name: 'Shop', address: 'hello@shop.example' };
  const { cats } = setup(t, msgs, { jevImpl: fake });
  const seen = [];
  cats.on('progress', (a) => seen.push(a));
  cats.setEnabled(true);
  cats.local.jev = { on: true, key: cats.secrets.encrypt('K') };
  const sorting = cats.sort({ user: true });
  await new Promise((r) => setImmediate(r));
  // The rules placed the login alert straight away; two wait for Jev.
  let a = cats.activity();
  assert.deepEqual((({ running, user, total, done, asked, answered, jev }) => ({ running, user, total, done, asked, answered, jev }))(a), { running: true, user: true, total: 3, done: 1, asked: 2, answered: 0, jev: true });
  assert.equal(seen.at(-1).running, true, 'the start is told straight away');
  answers.shift()();
  await new Promise((r) => setTimeout(r, 200));
  a = cats.activity();
  assert.equal(a.done, 2);
  assert.equal(seen.at(-1).done, 2, 'each answer is told, a few at a time');
  assert.deepEqual((({ subject, from, category, sure }) => ({ subject, from, category, sure }))(a.recent[0]), { subject: 'Can you sign this?', from: 'Sanne de Vries', category: 'money', sure: true });
  answers.shift()();
  assert.equal(await sorting, 3);
  a = cats.activity();
  assert.deepEqual((({ running, done, sorted, waiting }) => ({ running, done, sorted, waiting }))(a), { running: false, done: 3, sorted: 3, waiting: 0 });
  assert.ok(a.finishedAt >= a.startedAt);
  assert.deepEqual((({ subject, category, chose, sure }) => ({ subject, category, chose, sure }))(a.recent[0]), { subject: 'Sale', category: 'to-read', chose: 'promotions', sure: false }, 'an unsure answer shows where it went');
  assert.equal(seen.at(-1).running, false);

  // A sort nobody asked for with nothing to do leaves the last one's progress alone.
  await cats.sort();
  assert.equal(cats.activity().finishedAt, a.finishedAt);

  // Sorted again, an email shows once, with its newest decision.
  cats.jev = { MODEL: 'jev-test', classify: async () => ({ category: 'updates', confidence: 0.8, model: 'jev-test' }) };
  await cats.sortAgain();
  assert.deepEqual(cats.activity().recent.map((r) => [r.subject, r.category]).sort(), [['Can you sign this?', 'updates'], ['Sale', 'updates']]);
});

test('mail waiting for Jev is counted, with why and until when', async (t) => {
  const fake = { MODEL: 'jev-test', classify: async () => Promise.reject(Object.assign(new Error('busy'), { code: 'busy' })) };
  const { cats } = setup(t, inbox(), { jevImpl: fake });
  cats.setEnabled(true);
  cats.setJev({ key: 'K', on: true });
  await cats.sort();
  let a = cats.activity();
  assert.equal(a.error, 'busy');
  assert.ok(a.retryAt > Date.now());
  assert.equal(a.waiting, 1, 'the email the rules are unsure about waits');
  // While TypeSafe is busy, nothing is asked and the count stays.
  await cats.sort();
  a = cats.activity();
  assert.equal(a.waiting, 1);
  assert.equal(a.running, false);
  // Jev off: nothing waits for it any more.
  cats.setJev({ on: false });
  a = cats.activity();
  assert.deepEqual([a.jev, a.error, a.retryAt, a.waiting], [false, null, 0, 0]);
});

test('a refused Jev key stops Jev until a new key, and busy waits, with the uncertain mail kept for Jev', async (t) => {
  let fail = 'key';
  let calls = 0;
  const fake = {
    MODEL: 'jev-test',
    classify: async (key, m, list) => {
      calls++;
      if (fail) throw Object.assign(new Error(fail), { code: fail });
      return { category: list[2].id, confidence: 0.9, model: 'jev-test' };
    }
  };
  const msgs = inbox();
  const { cats, acc, account } = setup(t, msgs, { jevImpl: fake });
  const stored = (uid) => account.get('classification', cats.key(acc, msgs.find((m) => m.uid === uid)));
  cats.setEnabled(true);
  cats.setJev({ key: 'BAD', on: true });
  await cats.sort();
  assert.equal(cats.state().jev.error, 'key');
  assert.equal(stored(1), undefined, 'the uncertain email waits for Jev');
  assert.equal(stored(3).source, 'rules', 'a strong rule is stored as always');
  const shown = cats.of(acc, msgs[0]);
  assert.equal(shown, 'needs-you', 'the rules show a category meanwhile');

  // No more calls with the refused key, however often the inbox is sorted.
  const before = calls;
  await cats.sort();
  await cats.sort();
  assert.equal(calls, before);
  assert.equal(cats.state().jev.error, 'key');

  // Busy: no calls during the wait, and the email still waits.
  fail = 'busy';
  cats.setJev({ key: 'NEW' });
  await cats.sort();
  assert.equal(cats.state().jev.error, 'busy');
  assert.ok(cats.jevBusyUntil > Date.now());
  const during = calls;
  await cats.sort();
  assert.equal(calls, during);
  assert.equal(stored(1), undefined);

  // After the wait, Jev sorts it.
  fail = null;
  cats.jevBusyUntil = 0;
  await cats.sort();
  assert.equal(stored(1).source, 'jev');
  assert.equal(cats.state().jev.error, null);

  // Jev turned off: the rules store what is left.
  cats.setJev({ on: false });
  await cats.sortAgain();
  assert.equal(stored(1).source, 'rules');
});

test('turning Jev off stops the emails still queued for it', async (t) => {
  let calls = 0;
  let cats;
  const fake = {
    MODEL: 'jev-test',
    classify: async (key, m, list) => {
      calls++;
      if (calls === 1) cats.setJev({ on: false, key: null });
      await new Promise((r) => setImmediate(r));
      return { category: list[0].id, confidence: 0.9, model: 'jev-test' };
    }
  };
  const msgs = Array.from({ length: 8 }, (_, i) => ({ uid: 10 + i, messageId: `<q${i}@x>`, from: { name: 'Person Name', address: `person${i}@studio.example` }, to: [], cc: [{ address: 'me@work.example' }], subject: `Note ${i}`, date: i, signals: {} }));
  ({ cats } = setup(t, msgs, { jevImpl: fake }));
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  await cats.sort();
  assert.equal(calls, 1, 'only the request already on its way');
});

test('an answer that comes back after Jev was turned off is not stored', async (t) => {
  let answer;
  const fake = {
    MODEL: 'jev-test',
    classify: () => new Promise((resolve) => (answer = () => resolve({ category: 'money', confidence: 0.9, model: 'jev-test' })))
  };
  const msgs = inbox();
  const { cats, acc, account } = setup(t, msgs, { jevImpl: fake });
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  const sorting = cats.sort();
  await new Promise((r) => setImmediate(r));
  cats.setJev({ on: false });
  answer();
  await sorting;
  const stored = account.get('classification', cats.key(acc, msgs[0]));
  assert.ok(!stored || stored.source !== 'jev', 'the late answer from Jev was dropped');
  await cats.sort();
  assert.equal(account.get('classification', cats.key(acc, msgs[0])).source, 'rules', 'the next sort, without Jev, sorts it');
});

test('a refusal for a key that was replaced meanwhile leaves the new key alone', async (t) => {
  let reject;
  const fake = {
    MODEL: 'jev-test',
    classify: (key) =>
      key === 'OLD'
        ? new Promise((resolve, no) => {
            reject = () => no(Object.assign(new Error('key'), { code: 'key' }));
          })
        : Promise.resolve({ category: 'money', confidence: 0.9, model: 'jev-test' })
  };
  const { cats } = setup(t, inbox(), { jevImpl: fake });
  cats.setEnabled(true);
  cats.setJev({ key: 'OLD', on: true });
  const sorting = cats.sort();
  await new Promise((r) => setImmediate(r));
  cats.setJev({ key: 'NEW' });
  reject();
  await sorting;
  assert.equal(cats.state().jev.error, null);
  assert.equal(cats.jevKey(), 'NEW');
});

test('the sort after TypeSafe was busy survives other updates', async (t) => {
  const fake = {
    MODEL: 'jev-test',
    classify: async () => {
      throw Object.assign(new Error('busy'), { code: 'busy' });
    }
  };
  const { cats, engine } = setup(t, inbox(), { jevImpl: fake });
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  await cats.sort();
  assert.equal(cats.state().jev.error, 'busy');
  const retry = cats.retryTimer;
  assert.ok(retry, 'a sort is set for after the wait');
  engine.emit('updated');
  cats.soon(0);
  await cats.sort();
  assert.equal(cats.retryTimer, retry, 'mail updates and early sorts leave it');
  // Lost anyway (say, after a restart while busy): a sort during the wait sets it again.
  clearTimeout(cats.retryTimer);
  cats.retryTimer = null;
  await cats.sort();
  assert.ok(cats.retryTimer);
});

test('sorting again finishes the emails Jev was too busy for, once it answers', async (t) => {
  let busy = true;
  let calls = 0;
  const fake = {
    MODEL: 'jev-test',
    classify: async () => {
      calls++;
      if (busy) throw Object.assign(new Error('busy'), { code: 'busy' });
      return { category: 'money', confidence: 0.9, model: 'jev-test' };
    }
  };
  const msgs = inbox();
  const { cats, acc, account } = setup(t, msgs, { jevImpl: fake });
  const stored = (uid) => account.get('classification', cats.key(acc, msgs.find((m) => m.uid === uid)));
  cats.setEnabled(true);
  await cats.sort();
  assert.equal(stored(1).source, 'rules', 'sorted by the rules before Jev was on');
  cats.setJev({ key: 'KEY', on: true });
  await cats.sortAgain();
  assert.equal(cats.state().jev.error, 'busy');
  assert.equal(stored(1).source, 'rules', 'the older category stays meanwhile');
  busy = false;
  cats.jevBusyUntil = 0;
  const before = calls;
  await cats.sort();
  assert.ok(calls > before, 'the retry goes to Jev');
  assert.equal(stored(1).source, 'jev');
  assert.equal(cats.redo.size, 0);
});

test('a re-sort Jev was too busy for is finished after a restart', async (t) => {
  let busy = true;
  const fake = {
    MODEL: 'jev-test',
    classify: async () => {
      if (busy) throw Object.assign(new Error('busy'), { code: 'busy' });
      return { category: 'money', confidence: 0.9, model: 'jev-test' };
    }
  };
  const msgs = inbox();
  const { cats, acc, account, engine } = setup(t, msgs, { jevImpl: fake });
  const stored = (uid) => account.get('classification', cats.key(acc, msgs.find((m) => m.uid === uid)));
  cats.setEnabled(true);
  await cats.sort();
  cats.setJev({ key: 'KEY', on: true });
  await cats.sortAgain();
  assert.ok(cats.redo.size > 0);
  cats.close();
  // The app starts again: a new Categories on the same profile, with TypeSafe answering.
  busy = false;
  const again = new Categories({ engine, account, secrets, dataDir: cats.dataDir, jev: fake }).init();
  t.after(() => again.close());
  assert.equal(again.redo.size, cats.redo.size);
  await again.sort();
  assert.equal(stored(1).source, 'jev');
  assert.equal(again.redo.size, 0);
});

test('a re-sort the app quits during goes on after a restart', async (t) => {
  let category = 'promotions';
  let hold = false;
  let asked;
  const waiting = new Promise((r) => (asked = r));
  const fake = {
    MODEL: 'jev-test',
    classify: async () => {
      if (hold) {
        asked();
        await new Promise(() => {});
      }
      return { category, confidence: 0.9, model: 'jev-test' };
    }
  };
  const msgs = inbox();
  const { cats, acc, account, engine } = setup(t, msgs, { jevImpl: fake });
  const stored = (uid) => account.get('classification', cats.key(acc, msgs.find((m) => m.uid === uid)));
  // Jev first: turning categories on sorts at once.
  cats.setJev({ key: 'KEY', on: true });
  cats.setEnabled(true);
  await cats.sort();
  assert.equal(stored(1).category, 'promotions');

  // "Sort again", and the app quits while Jev is answering: only what is on file is left.
  hold = true;
  cats.sortAgain();
  await waiting;
  const onFile = JSON.parse(fs.readFileSync(cats.file(), 'utf8'));
  assert.ok(onFile.redo.includes(cats.key(acc, msgs[0])), 'the emails waiting for Jev are on file');

  hold = false;
  category = 'money';
  const again = new Categories({ engine, account, secrets, dataDir: cats.dataDir, jev: fake }).init();
  t.after(() => again.close());
  await again.sort();
  assert.equal(stored(1).category, 'money', 'the re-sort replaced the old answer');
  assert.equal(again.redo.size, 0);
});

test("a finished re-sort's answers are on disk before the list that says it's done", async (t) => {
  let category = 'promotions';
  const fake = { MODEL: 'jev-test', classify: async () => ({ category, confidence: 0.9, model: 'jev-test' }) };
  const msgs = inbox();
  const { cats, acc } = setup(t, msgs, { jevImpl: fake });
  // Jev first: turning categories on sorts at once.
  cats.setJev({ key: 'KEY', on: true });
  cats.setEnabled(true);
  await cats.sort();
  category = 'money';
  await cats.sortAgain();
  assert.deepEqual(JSON.parse(fs.readFileSync(cats.file(), 'utf8')).redo, []);
  // The app quits now, within the account's second before it saves: a new account service reads what is on disk.
  const account = new AccountService({ dataDir: cats.dataDir, secrets, defaultServer: '', deviceName: 'desk' }).init();
  t.after(() => account.close());
  assert.equal(account.get('classification', cats.key(acc, msgs[0])).category, 'money');
});

test('a re-sort whose answers the disk refuses stays on the list on disk', async (t) => {
  let category = 'promotions';
  const fake = { MODEL: 'jev-test', classify: async () => ({ category, confidence: 0.9, model: 'jev-test' }) };
  const msgs = inbox();
  const { cats, acc } = setup(t, msgs, { jevImpl: fake });
  // Jev first: turning categories on sorts at once.
  cats.setJev({ key: 'KEY', on: true });
  cats.setEnabled(true);
  await cats.sort();
  // The disk refuses the account's records from now on: a directory where their file goes stands in for a full disk.
  const records = path.join(cats.dataDir, 'account-records.json');
  fs.rmSync(records, { force: true });
  fs.mkdirSync(path.join(records, 'x'), { recursive: true });
  category = 'money';
  await cats.sortAgain();
  assert.ok(JSON.parse(fs.readFileSync(cats.file(), 'utf8')).redo.includes(cats.key(acc, msgs[0])), 'the email stays on the list on disk');
});

test('a very long Message-ID is kept as its hash', (t) => {
  const { cats, acc } = setup(t, inbox());
  const long = `<${'x'.repeat(20000)}@evil.example>`;
  const key = cats.key(acc, { messageId: long });
  assert.match(key, /^me@work\.example\nm:[0-9a-f]{64}$/);
  assert.equal(cats.key(acc, { messageId: '<short@x>' }), 'me@work.example\n<short@x>');
});

test('sorting again while a sort runs follows it instead of getting lost', async (t) => {
  let release;
  let first = true;
  const fake = {
    MODEL: 'jev-test',
    classify: async () => {
      if (first) {
        first = false;
        await new Promise((r) => (release = r));
      }
      return { category: 'money', confidence: 0.9, model: 'jev-test' };
    }
  };
  const msgs = inbox();
  const { cats, acc, account } = setup(t, msgs, { jevImpl: fake });
  const stored = (uid) => account.get('classification', cats.key(acc, msgs.find((m) => m.uid === uid)));
  cats.setEnabled(true);
  await cats.sort();
  assert.equal(stored(3).source, 'rules');
  // A new email starts an ordinary sort with Jev; "Sort again" comes while it waits on Jev.
  msgs.push({ uid: 7, messageId: '<7@x>', from: { name: 'Person Name', address: 'person@studio.example' }, to: [], cc: [{ address: 'me@work.example' }], subject: 'FYI', date: 7, signals: {} });
  cats.setJev({ key: 'KEY', on: true });
  const running = cats.sort();
  await new Promise((r) => setImmediate(r));
  const again = cats.sortAgain();
  release();
  await Promise.all([running, again]);
  assert.equal(stored(7).source, 'jev');
  assert.equal(stored(1).source, 'jev', 'sorted again, not left as it was');
});

test('categories can be added, renamed, reordered and deleted, within limits', (t) => {
  const { cats } = setup(t, inbox());
  cats.setEnabled(true);
  assert.throws(() => cats.saveCategory({ name: '  ' }), /name/);
  let state = cats.saveCategory({ name: 'Kids', description: 'School and sports clubs', color: '#4a7dff' });
  const kids = state.categories.find((c) => c.name === 'Kids');
  assert.equal(kids.order, 5);
  state = cats.saveCategory({ id: 'money', name: 'Finance' });
  assert.equal(state.categories.find((c) => c.id === 'money').name, 'Finance');
  state = cats.saveCategory({ id: 'money', name: null });
  assert.equal(state.categories.find((c) => c.id === 'money').name, 'Money', 'back to the preset name');
  state = cats.reorder([kids.id, 'needs-you', 'to-read', 'money', 'updates', 'promotions']);
  assert.equal(state.categories[0].id, kids.id);
  assert.throws(() => cats.reorder(['needs-you']), /invalid/);
  cats.setUnsure(kids.id);
  state = cats.removeCategory(kids.id);
  assert.notEqual(state.unsure, kids.id);
  for (let i = state.categories.length; i < 12; i++) cats.saveCategory({ name: `C${i}` });
  assert.throws(() => cats.saveCategory({ name: 'One more' }), /12/);
});

// ---- notifications per category (notices.js) ----

// What MailNotices asked to put on screen: one list of notices per call, and the emails each call was for.
function notifier(t, cats) {
  const shown = [];
  const emails = [];
  const notices = new MailNotices({
    categories: cats,
    show: (list, messages) => {
      shown.push(list);
      emails.push(messages.map((m) => m.subject));
    }
  });
  t.after(() => notices.close());
  return { notices, shown, titles: () => shown.map((list) => list.map((n) => n.title)), subjects: () => emails };
}

// The engine's 'new-mail' message for an email in the fake inbox.
const arrived = (cats, acc, m) => ({ id: String(m.uid), from: m.from, subject: m.subject, preview: '', category: cats.of(acc, m) });

// A mailing list the rules aren't sure about: they say Promotions.
const club = () => ({ uid: 4, messageId: '<4@x>', from: { name: 'Club', address: 'board@club.example' }, to: [{ address: 'me@work.example' }], subject: 'Meeting notes', date: 0, signals: { list: true } });

// A Jev that answers, or fails, when the test says so, by subject.
function heldJev() {
  const asked = new Map();
  return {
    fake: { MODEL: 'jev-test', classify: (key, m) => new Promise((resolve, reject) => asked.set(m.subject, { resolve, reject })) },
    asked: () => [...asked.keys()],
    answer: (subject, category) => asked.get(subject).resolve({ category, confidence: 0.9, model: 'jev-test' }),
    fail: (subject, code) => asked.get(subject).reject(Object.assign(new Error(code), { code }))
  };
}

const settle = () => new Promise((r) => setImmediate(r));

test('with no category muted, new mail notifies at once, as without categories', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const jevs = heldJev();
  const msgs = inbox();
  const { cats, acc } = setup(t, msgs, { jevImpl: jevs.fake });
  const { notices, titles } = notifier(t, cats);
  const all = () => msgs.map((m) => arrived(cats, acc, m));
  // Categories off.
  notices.add(all());
  // On, with Jev about to sort Sanne's email.
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  notices.add(all());
  // A muted category counts only while categories are on.
  cats.setNotify('promotions', false);
  cats.setEnabled(false);
  notices.add(all());
  assert.deepEqual(titles(), Array(3).fill(['Sanne de Vries', 'Shop', 'App']));
  assert.equal(notices.batches.size, 0);
  assert.deepEqual(jevs.asked(), [], 'nothing waited for a sort');
});

test("a muted category's mail that the rules are sure about sends no notification", (t) => {
  const jevs = heldJev();
  const msgs = [...inbox(), { uid: 5, messageId: '<5@x>', from: { name: 'Sanne de Vries', address: 'sanne@studio.example' }, to: [{ address: 'me@work.example' }], subject: 'Lunch?', date: 5, signals: {} }];
  const { cats, acc, account } = setup(t, msgs, { jevImpl: jevs.fake });
  const { notices, titles } = notifier(t, cats);
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  cats.setNotify('promotions', false);
  cats.setNotify('updates', false);
  // The shop's offer and the login alert: strong rules, so no Jev and no wait.
  notices.add([arrived(cats, acc, msgs[1]), arrived(cats, acc, msgs[2])]);
  assert.deepEqual(titles(), []);
  assert.equal(notices.batches.size, 0);
  // A sender rule places mail at once too, also mail the rules alone aren't sure about.
  account.put('sender', 'sanne@studio.example', { category: 'promotions' });
  notices.add([arrived(cats, acc, msgs[3])]);
  assert.deepEqual(titles(), []);
  assert.equal(notices.batches.size, 0);
});

test('a held notification goes by the category Jev gives the email', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const jevs = heldJev();
  const msgs = [...inbox(), club()];
  const { cats, acc } = setup(t, msgs, { jevImpl: jevs.fake });
  const { notices, titles, subjects } = notifier(t, cats);
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  cats.setNotify('money', false);
  // Sanne's email and the club's go to Jev; the login alert is sure, and shows at once.
  notices.add([1, 3, 4].map((uid) => arrived(cats, acc, msgs.find((m) => m.uid === uid))));
  assert.deepEqual(titles(), [['App']]);
  // The main window hears of the same emails as the notifications (main.js showMailNotices).
  assert.deepEqual(subjects(), [[msgs.find((m) => m.uid === 3).subject]]);
  const sorting = cats.sort();
  await settle();
  assert.deepEqual(jevs.asked().sort(), ['Can you sign this?', 'Meeting notes']);
  jevs.answer('Can you sign this?', 'money');
  await settle();
  assert.deepEqual(titles(), [['App']], 'the batch waits for its last answer');
  jevs.answer('Meeting notes', 'to-read');
  await sorting;
  assert.deepEqual(titles(), [['App'], ['Club']], "Jev put Sanne's email in Money, which is muted");
  assert.deepEqual(subjects()[1], ['Meeting notes'], 'nor does the window hear of it');
  assert.equal(notices.batches.size, 0);
  t.mock.timers.tick(HOLD);
  assert.equal(titles().length, 2, 'the wait has nothing left to do');
});

test('after the wait for Jev, the category the email has then decides', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const jevs = heldJev();
  const msgs = [...inbox(), club()];
  const { cats, acc } = setup(t, msgs, { jevImpl: jevs.fake });
  const { notices, titles } = notifier(t, cats);
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  cats.setNotify('needs-you', false);
  notices.add([1, 4].map((uid) => arrived(cats, acc, msgs.find((m) => m.uid === uid))));
  // TypeSafe never answers.
  cats.sort();
  t.mock.timers.tick(HOLD - 1);
  assert.deepEqual(titles(), []);
  t.mock.timers.tick(1);
  // The rules put Sanne's email in Needs you, which is muted, and the club's in Promotions.
  assert.deepEqual(titles(), [['Club']]);
  assert.equal(notices.batches.size, 0);
});

test('without Jev to wait for, the category decides at once', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let fail = 'key';
  const fake = {
    MODEL: 'jev-test',
    classify: async () => {
      throw Object.assign(new Error(fail), { code: fail });
    }
  };
  const msgs = inbox();
  const { cats, acc } = setup(t, msgs, { jevImpl: fake });
  const { notices, titles } = notifier(t, cats);
  cats.setEnabled(true);
  cats.setNotify('needs-you', false);
  const batch = () => [arrived(cats, acc, msgs[0]), arrived(cats, acc, msgs[2])];
  // No key.
  notices.add(batch());
  // TypeSafe refused the key.
  cats.setJev({ key: 'BAD', on: true });
  await cats.sort();
  assert.equal(cats.state().jev.error, 'key');
  notices.add(batch());
  // TypeSafe is busy.
  fail = 'busy';
  cats.setJev({ key: 'NEW' });
  await cats.sort();
  assert.equal(cats.state().jev.error, 'busy');
  notices.add(batch());
  // Sanne's email is in Needs you by the rules each time.
  assert.deepEqual(titles(), [['App'], ['App'], ['App']]);
  assert.equal(notices.batches.size, 0);
});

test('a notification waiting for Jev goes as soon as Jev gives up or is turned off', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const jevs = heldJev();
  const msgs = inbox();
  const { cats, acc } = setup(t, msgs, { jevImpl: jevs.fake });
  const { notices, titles } = notifier(t, cats);
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  cats.setNotify('money', false);

  // TypeSafe is busy: the rules' Needs you decides.
  notices.add([arrived(cats, acc, msgs[0])]);
  let sorting = cats.sort();
  await settle();
  jevs.fail('Can you sign this?', 'busy');
  await sorting;
  assert.deepEqual(titles(), [['Sanne de Vries']]);

  // Jev turned off while it sorts the club's new email: the rules' Promotions decides.
  msgs.push(club());
  cats.setJev({ key: 'KEY', on: true });
  notices.add([arrived(cats, acc, msgs[3])]);
  sorting = cats.sort();
  await settle();
  assert.equal(notices.batches.size, 1);
  cats.setJev({ on: false });
  assert.deepEqual(titles(), [['Sanne de Vries'], ['Club']]);
  // The answers on their way when Jev went off.
  jevs.answer('Can you sign this?', 'money');
  jevs.answer('Meeting notes', 'money');
  await sorting;
  assert.equal(titles().length, 2);
  assert.equal(notices.batches.size, 0);
});

test('new mail still notifies three at a time, then counts, also after waiting for Jev', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  assert.deepEqual(mailNotices([{ id: 'a', from: { name: '', address: 'a@x.example' }, subject: 'Hi', preview: 'Hello there' }]), [
    { id: 'a', title: 'a@x.example', body: 'Hi\nHello there' }
  ]);
  const msgs = Array.from({ length: 5 }, (_, i) => ({ uid: 20 + i, messageId: `<n${i}@x>`, from: { name: `Person ${i}`, address: `person${i}@studio.example` }, to: [], cc: [{ address: 'me@work.example' }], subject: `Note ${i}`, date: 20 + i, signals: {} }));
  const fake = { MODEL: 'jev-test', classify: async () => ({ category: 'to-read', confidence: 0.9, model: 'jev-test' }) };
  const { cats, acc } = setup(t, msgs, { jevImpl: fake });
  const { notices, shown } = notifier(t, cats);
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  cats.setNotify('money', false);
  notices.add(msgs.map((m) => arrived(cats, acc, m)));
  assert.equal(shown.length, 0, 'all five wait for Jev');
  await cats.sort();
  assert.equal(shown.length, 1, 'one batch, shown once');
  assert.deepEqual(shown[0].map((n) => n.title), ['Person 0', 'Person 1', 'Person 2', 'Rukoo Mail']);
  assert.deepEqual(shown[0].slice(0, 3).map((n) => n.id), ['20', '21', '22']);
  assert.deepEqual(shown[0][3], { id: null, title: 'Rukoo Mail', body: '5 new emails' });
});

test("notifications per category stay on this computer, and a deleted category's setting goes with it", (t) => {
  const { cats, account, engine } = setup(t, inbox());
  cats.setEnabled(true);
  assert.deepEqual(cats.state().muted, []);
  cats.setNotify('promotions', false);
  assert.deepEqual(cats.setNotify('updates', false).muted, ['updates', 'promotions']);
  assert.ok(account.list('category').every(({ v }) => !('muted' in v) && !('notify' in v)), 'not in the synced records');
  assert.deepEqual(JSON.parse(fs.readFileSync(cats.file(), 'utf8')).muted, ['promotions', 'updates']);
  cats.setNotify('updates', true);
  assert.throws(() => cats.setNotify('nope', false), /invalid/);
  // After a restart.
  cats.close();
  const again = new Categories({ engine, account, secrets, dataDir: cats.dataDir, jev }).init();
  t.after(() => again.close());
  assert.deepEqual(again.state().muted, ['promotions']);
  again.removeCategory('promotions');
  assert.deepEqual(JSON.parse(fs.readFileSync(again.file(), 'utf8')).muted, []);
});

test('saving a category says which one it saved, also a new one', (t) => {
  const { cats } = setup(t, inbox());
  cats.setEnabled(true);
  const state = cats.saveCategory({ name: 'Kids' });
  assert.equal(state.categories.find((c) => c.id === state.saved).name, 'Kids');
  assert.equal(cats.saveCategory({ id: 'money', name: 'Finance' }).saved, 'money');
});

test('a batch that partly waits for Jev still notifies three at a time, then counts once', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  // Sign-in alerts the rules are sure about, and notes from people that go to Jev.
  const alert = (i) => ({ uid: 30 + i, messageId: `<a${i}@x>`, from: { name: `App ${i}`, address: `noreply@app${i}.example` }, to: [{ address: 'me@work.example' }], subject: `Sign-in ${i}`, date: 30 + i, signals: {} });
  const note = (i) => ({ uid: 40 + i, messageId: `<p${i}@x>`, from: { name: `Person ${i}`, address: `person${i}@studio.example` }, to: [], cc: [{ address: 'me@work.example' }], subject: `Note ${i}`, date: 40 + i, signals: {} });
  const fake = { MODEL: 'jev-test', classify: async () => ({ category: 'to-read', confidence: 0.9, model: 'jev-test' }) };
  const arrive = async (alerts, notes) => {
    const msgs = [...alerts.map(alert), ...notes.map(note)];
    const { cats, acc } = setup(t, msgs, { jevImpl: fake });
    const { notices, shown } = notifier(t, cats);
    cats.setEnabled(true);
    cats.setJev({ key: 'KEY', on: true });
    cats.setNotify('money', false);
    notices.add(msgs.map((m) => arrived(cats, acc, m)));
    const first = shown.map((list) => list.map((n) => n.title));
    await cats.sort();
    // An email's own notification by its sender, the count by its text.
    return { first, all: shown.map((list) => list.map((n) => (n.id ? n.title : n.body))) };
  };

  // Four at once and four after Jev: three now, and one count for all eight at the end.
  let { first, all } = await arrive([0, 1, 2, 3], [0, 1, 2, 3]);
  assert.deepEqual(first, [['App 0', 'App 1', 'App 2']]);
  assert.deepEqual(all, [['App 0', 'App 1', 'App 2'], ['8 new emails']]);

  // One at once: the notes Jev answered fill the three, and the count covers the batch.
  ({ first, all } = await arrive([0], [0, 1, 2, 3]));
  assert.deepEqual(first, [['App 0']]);
  assert.deepEqual(all, [['App 0'], ['Person 0', 'Person 1', '5 new emails']]);

  // All of it after Jev, three or fewer: no count.
  ({ all } = await arrive([], [0, 1]));
  assert.deepEqual(all, [['Person 0', 'Person 1']]);
});

test('a notification waits for a re-sort Jev has yet to do, also for mail sorted before', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const jevs = heldJev();
  const msgs = inbox();
  const { cats, acc } = setup(t, msgs, { jevImpl: jevs.fake });
  const { notices, titles } = notifier(t, cats);
  cats.setEnabled(true);
  // Without Jev, the rules store Needs you for Sanne's email.
  await cats.sort();
  cats.setJev({ key: 'KEY', on: true });
  cats.setNotify('money', false);

  // On the re-sort list, because TypeSafe was busy during "Sort the inbox again": Jev sorts it next.
  cats.redo.add(cats.key(acc, msgs[0]));
  notices.add([arrived(cats, acc, msgs[0])]);
  assert.equal(notices.batches.size, 1);
  let sorting = cats.sort();
  await settle();
  jevs.answer('Can you sign this?', 'money');
  await sorting;
  assert.deepEqual(titles(), [], 'Jev moved it to Money, which is muted');

  // "Sort the inbox again" running: Jev has the email now.
  sorting = cats.sortAgain();
  await settle();
  notices.add([arrived(cats, acc, msgs[0])]);
  assert.equal(notices.batches.size, 1);
  jevs.answer('Can you sign this?', 'to-read');
  await sorting;
  assert.deepEqual(titles(), [['Sanne de Vries']]);

  // "Sort the inbox again" asked for while a sort runs for the club's new email: it follows, and takes Sanne's email
  // up again. Jev turned off meanwhile: the category it has decides.
  msgs.push(club());
  cats.sort();
  await settle();
  cats.sortAgain();
  notices.add([arrived(cats, acc, msgs[0])]);
  assert.equal(notices.batches.size, 1);
  cats.setJev({ on: false });
  assert.equal(notices.batches.size, 0);
  assert.deepEqual(titles(), [['Sanne de Vries'], ['Sanne de Vries']]);
});

test("Jev's answers look at the email each is for, not at every waiting one", async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const n = 40;
  const msgs = Array.from({ length: n }, (_, i) => ({ uid: 100 + i, messageId: `<w${i}@x>`, from: { name: `Person ${i}`, address: `person${i}@studio.example` }, to: [], cc: [{ address: 'me@work.example' }], subject: `Note ${i}`, date: 100 + i, signals: {} }));
  const fake = { MODEL: 'jev-test', classify: async () => ({ category: 'to-read', confidence: 0.9, model: 'jev-test' }) };
  const { cats, acc, account } = setup(t, msgs, { jevImpl: fake });
  const { notices, shown } = notifier(t, cats);
  cats.setEnabled(true);
  cats.setJev({ key: 'KEY', on: true });
  cats.setNotify('money', false);
  // One email per arrival, so every batch waits on its own.
  for (const m of msgs) notices.add([arrived(cats, acc, m)]);
  assert.equal(notices.batches.size, n);
  // Each look at the categories reads every record the account has (AccountService.list); each look at an email
  // finds it in the inbox and reads its records.
  let reads = 0;
  let looks = 0;
  const list = account.list.bind(account);
  account.list = (c) => {
    if (c === 'category') reads++;
    return list(c);
  };
  const notice = cats.notice.bind(cats);
  cats.notice = (...args) => {
    looks++;
    return notice(...args);
  };
  await cats.sort();
  assert.equal(shown.length, n);
  assert.equal(notices.batches.size, 0);
  // The sort reads the categories about once per email, and each answer once more. Each answer looks at its email
  // twice: once when it comes, once when its batch goes. Looking at every waiting email for each answer would take
  // more than n * n / 2.
  assert.ok(looks <= 2 * n, `${looks} looks for ${n} answers`);
  assert.ok(reads <= 3 * n, `${reads} reads for ${n} answers`);
});

test('a category kept out of Inbox says so in its own record, beside fields another app added', (t) => {
  const { cats, account } = setup(t, inbox());
  cats.setEnabled(true);
  assert.ok(cats.list().every((c) => c.inbox), 'every category starts in Inbox');
  assert.deepEqual([...cats.hiddenFromInbox()], []);
  // A newer app may keep a field of its own in the record.
  account.put('category', 'promotions', { ...account.get('category', 'promotions'), later: 1 });
  let state = cats.saveCategory({ id: 'promotions', inbox: false });
  assert.equal(state.categories.find((c) => c.id === 'promotions').inbox, false);
  assert.equal(account.get('category', 'promotions').inbox, false);
  assert.equal(account.get('category', 'promotions').later, 1);
  assert.deepEqual([...cats.hiddenFromInbox()], ['promotions']);
  // Other edits leave it as it is, and so does going back to Rukoo's own text.
  cats.saveCategory({ id: 'promotions', name: 'Ads', color: '#4a7dff' });
  cats.saveCategory({ id: 'promotions', name: null, description: null });
  assert.equal(account.get('category', 'promotions').inbox, false);
  // With categories off, Inbox has nothing to leave out.
  cats.setEnabled(false);
  assert.deepEqual([...cats.hiddenFromInbox()], []);
  cats.setEnabled(true);
  // Shown again, the record is as older apps write it.
  cats.saveCategory({ id: 'promotions', inbox: true });
  assert.equal('inbox' in account.get('category', 'promotions'), false);
  assert.equal(account.get('category', 'promotions').later, 1);
  assert.equal(cats.list().find((c) => c.id === 'promotions').inbox, true);
  state = cats.saveCategory({ name: 'Kids', inbox: false });
  assert.equal(state.categories.find((c) => c.name === 'Kids').inbox, false);
});

test('every device with the account keeps a category out of Inbox, also after another edits it', async (t) => {
  const { startAccountServer } = require('./fixtures/account-server');
  const srv = await startAccountServer();
  t.after(() => srv.close());
  const device = (name) => {
    const dir = tmpDir(t);
    const { engine } = fakeEngine(inbox());
    const account = new AccountService({ dataDir: dir, secrets, defaultServer: srv.url, deviceName: name, platform: 'linux' }).init();
    const cats = new Categories({ engine, account, secrets, dataDir: dir, jev }).init();
    t.after(() => {
      cats.close();
      account.close();
    });
    return { cats, account };
  };
  const desk = device('desk');
  await desk.account.signUp({ email: 'hide@example.com', password: 'a long enough password' });
  desk.cats.setEnabled(true);
  desk.cats.saveCategory({ id: 'promotions', inbox: false });
  await desk.account.sync();

  const laptop = device('laptop');
  await laptop.account.signIn({ email: 'hide@example.com', password: 'a long enough password' });
  let changed = 0;
  laptop.cats.on('changed', () => changed++);
  await laptop.account.sync();
  assert.ok(changed > 0, 'the window hears of it');
  assert.deepEqual([...laptop.cats.hiddenFromInbox()], ['promotions']);

  // A rename on the laptop keeps the switch, on both devices.
  laptop.cats.saveCategory({ id: 'promotions', name: 'Ads' });
  await laptop.account.sync();
  await desk.account.sync();
  const promotions = desk.cats.list().find((c) => c.id === 'promotions');
  assert.equal(promotions.name, 'Ads');
  assert.equal(promotions.inbox, false);
});

// The real engine with the demo account, and categories on.
async function demoInbox(t) {
  const { Engine } = require('../src/main/engine');
  const dir = tmpDir(t);
  const engine = new Engine({ dataDir: path.join(dir, 'mail') }).init();
  const acc = await engine.addAccount({ type: 'demo' });
  await engine.syncAccount(acc.id);
  const account = new AccountService({ dataDir: dir, secrets, defaultServer: '', deviceName: 'desk' }).init();
  const cats = new Categories({ engine, account, secrets, dataDir: dir, jev }).init();
  t.after(async () => {
    cats.close();
    account.close();
    await engine.close();
  });
  cats.setEnabled(true);
  return { engine, acc, cats };
}

test("unread mail in a category Inbox doesn't show leaves the inbox counts, but not Unread's", async (t) => {
  const { engine, acc, cats } = await demoInbox(t);
  const unread = (category) => engine.listMessages({ scope: acc.id, view: 'inbox' }).filter((m) => m.unread && (!category || m.category === category)).length;
  const all = unread();
  const promotions = unread('promotions');
  assert.ok(promotions > 0 && promotions < all, 'the demo inbox has unread promotions and other unread mail');
  assert.equal(engine.counts(acc.id).views.inbox, all);

  cats.saveCategory({ id: 'promotions', inbox: false });
  const expectCounts = (inboxCount, unreadCount) => {
    const c = engine.counts(acc.id);
    assert.equal(c.views.inbox, inboxCount);
    assert.equal(c.accounts[acc.id], inboxCount, 'the account menu');
    assert.equal(c.all, inboxCount, 'All inboxes in the account menu');
    assert.equal(engine.counts('all').views.inbox, inboxCount, 'All inboxes');
    assert.equal(engine.unreadCount(), inboxCount, 'the badge and the tray');
    assert.equal(c.views.unread, unreadCount, 'Unread still counts every category');
  };
  expectCounts(all - promotions, all);
  // The list still has that mail, for the category's own view.
  assert.equal(unread('promotions'), promotions);

  // The unread mail older than the cache has no category, so it keeps counting.
  engine.caches.get(acc.id).boxes.INBOX.olderUnseen = 40;
  expectCounts(all - promotions + 40, all + 40);

  cats.setEnabled(false);
  expectCounts(all + 40, all + 40);
});

test("new mail stops counting for the badge once it is read, moved or in a category Inbox doesn't show", async (t) => {
  const { encodeId } = require('../src/main/engine');
  const { engine, acc, cats } = await demoInbox(t);
  const unread = engine.listMessages({ scope: acc.id, view: 'inbox' }).filter((m) => m.unread);
  const promotion = unread.find((m) => m.category === 'promotions');
  const [first, second, third] = unread.filter((m) => m.category !== 'promotions');
  assert.ok(promotion && third, 'the demo inbox has unread promotions and other unread mail');
  const gone = encodeId(acc.id, 'INBOX', 999999);
  const ids = new Set([promotion.id, first.id, second.id, third.id, gone]);
  assert.equal(engine.newCount(ids), 4);
  assert.equal(ids.has(gone), false, 'an email no longer in the cache leaves the list');

  cats.saveCategory({ id: 'promotions', inbox: false });
  assert.equal(engine.newCount(ids), 3);
  // Put in a hidden category after it came in: by a move here, a sender rule, Jev or another device.
  cats.move([first.id], 'promotions');
  assert.equal(engine.newCount(ids), 2);
  await engine.setFlags(second.id, { unread: false });
  assert.equal(engine.newCount(ids), 1);
  // Shown again, the promotions count again.
  cats.saveCategory({ id: 'promotions', inbox: true });
  assert.equal(engine.newCount(ids), 3);
});
