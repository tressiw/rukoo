'use strict';

// Undo send (#104): the outbox that holds an email after Send (src/main/outbox.js), with a clock the test turns.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');
const { tempDir } = require('./tmp');
const { Outbox, Handoff, savedToDrafts, askBeforeQuit } = require('../src/main/outbox');
const { Engine } = require('../src/main/engine');

// Timers that only fire when the test says so.
function fakeClock() {
  let now = 1000;
  const timers = new Map();
  let next = 1;
  return {
    now: () => now,
    timers: {
      setTimeout: (fn, ms) => {
        const id = next++;
        timers.set(id, { fn, at: now + ms });
        return id;
      },
      clearTimeout: (id) => timers.delete(id)
    },
    // Moves the clock on and fires what is due, in order.
    advance(ms) {
      now += ms;
      for (const [id, t] of [...timers].sort((a, b) => a[1].at - b[1].at)) {
        if (t.at > now) continue;
        timers.delete(id);
        t.fn();
      }
    },
    pending: () => timers.size
  };
}

// An outbox whose sends are recorded. failWith: every send fails with it. hold: sends wait until release().
function outbox({ failWith = null, hold = false } = {}) {
  const clock = fakeClock();
  const log = [];
  const held = [];
  const box = new Outbox({
    send: async (payload) => {
      log.push(['send', payload.subject]);
      if (hold) await new Promise((resolve) => held.push(resolve));
      if (failWith) throw new Error(failWith);
    },
    sent: (entry) => log.push(['sent', entry.payload.subject, entry.origin]),
    failed: async (entry, err) => log.push(['failed', entry.payload.subject, err.message, entry.restore]),
    timers: clock.timers,
    now: clock.now
  });
  return { box, clock, log, release: () => held.splice(0).forEach((resolve) => resolve()) };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test('an email waits for its time, then goes and runs sent()', async () => {
  const { box, clock, log } = outbox();
  const { id, until } = box.queue({ payload: { subject: 'Hi' }, restore: { html: 'x' }, origin: 'window', ms: 5000 });
  assert.equal(typeof id, 'string');
  assert.equal(until, 6000);
  assert.equal(box.size, 1);
  clock.advance(4999);
  await settle();
  assert.deepEqual(log, [], 'nothing goes before its time');
  clock.advance(1);
  await box.flush();
  assert.deepEqual(log, [['send', 'Hi'], ['sent', 'Hi', 'window']]);
  assert.equal(box.size, 0);
});

test('undo takes back a waiting email with what reopens its editor, and it never goes', async () => {
  const { box, clock, log } = outbox();
  const restore = { html: '<div>text</div>', state: { subject: 'Hi' } };
  const { id } = box.queue({ payload: { subject: 'Hi' }, restore, origin: 'main', ms: 5000 });
  const back = box.undo(id);
  assert.deepEqual(back, { payload: { subject: 'Hi' }, restore, origin: 'main' });
  assert.equal(box.size, 0);
  assert.equal(clock.pending(), 0, 'its timer is gone');
  clock.advance(10000);
  await box.flush();
  assert.deepEqual(log, []);
  assert.equal(box.undo(id), null, 'a second undo finds nothing');
  assert.equal(box.undo('no-such-id'), null);
});

test('undo is too late once the email is on its way', async () => {
  const { box, clock, log, release } = outbox({ hold: true });
  const { id } = box.queue({ payload: { subject: 'Hi' }, restore: {}, ms: 5000 });
  clock.advance(5000);
  await settle();
  assert.deepEqual(log, [['send', 'Hi']]);
  assert.equal(box.undo(id), null);
  assert.equal(box.size, 1, 'still counted while it is being sent');
  release();
  await box.flush();
  assert.deepEqual(log, [['send', 'Hi'], ['sent', 'Hi', 'main']]);
});

test('flush sends everything that waits at once, and waits for emails already on their way', async () => {
  const { box, clock, log, release } = outbox({ hold: true });
  box.queue({ payload: { subject: 'First' }, restore: {}, ms: 5000 });
  clock.advance(5000);
  box.queue({ payload: { subject: 'Second' }, restore: {}, ms: 30000 });
  let done = false;
  const flushing = box.flush().then(() => (done = true));
  await settle();
  assert.deepEqual(log, [['send', 'First'], ['send', 'Second']], 'the second goes without waiting its 30 s');
  assert.equal(clock.pending(), 0);
  assert.equal(done, false, 'flush waits until the sends are done');
  release();
  await flushing;
  assert.equal(box.size, 0);
  assert.deepEqual(log.slice(2), [['sent', 'First', 'main'], ['sent', 'Second', 'main']]);
});

test('left() lists what still waits or is on its way, for the quit to write down', async () => {
  const { box, clock, release } = outbox({ hold: true });
  const a = box.queue({ payload: { subject: 'A' }, restore: { html: 'a' }, origin: 'window', ms: 5000 });
  const b = box.queue({ payload: { subject: 'B' }, restore: { html: 'b' }, ms: 30000 });
  clock.advance(5000);
  assert.deepEqual(
    box.left().map(({ id, state, restore, origin }) => [id, state, restore.html, origin]),
    [
      [a.id, 'sending', 'a', 'window'],
      [b.id, 'waiting', 'b', 'main']
    ]
  );
  const flushing = box.flush();
  await settle();
  release();
  await flushing;
  assert.deepEqual(box.left(), []);
});

test('a refused send runs failed() with the error and the editor to reopen, and flush waits for it', async () => {
  const { box, clock, log } = outbox({ failWith: 'Mailbox full' });
  const restore = { html: '<div>keep me</div>' };
  box.queue({ payload: { subject: 'Hi' }, restore, ms: 5000 });
  clock.advance(5000);
  await box.flush();
  assert.deepEqual(log, [['send', 'Hi'], ['failed', 'Hi', 'Mailbox full', restore]]);
  assert.equal(box.size, 0);
});

test('a failing after-step does not break the outbox or reject flush', async () => {
  const clock = fakeClock();
  const box = new Outbox({
    send: async () => {},
    sent: () => {
      throw new Error('window gone');
    },
    timers: clock.timers,
    now: clock.now
  });
  box.queue({ payload: {}, restore: {}, ms: 1 });
  await box.flush();
  assert.equal(box.size, 0);
});

// ---------- refused emails held for the main window (Handoff) ----------

function handoff() {
  const clock = fakeClock();
  const elsewhere = [];
  const box = new Handoff({ ms: 15000, elsewhere: (items) => elsewhere.push(items.map((i) => i.id)), timers: clock.timers });
  return { box, clock, elsewhere };
}

test('a refused email stays held after the main window lists it, until its editor is open', () => {
  const { box, clock, elsewhere } = handoff();
  box.hold({ id: 'a', restore: { html: 'a' }, error: 'Mailbox full' });
  box.hold({ id: 'b', restore: { html: 'b' }, error: 'Mailbox full' });
  // A page that is still loading asks for nothing yet; once it asks, it gets every one, oldest first.
  clock.advance(14999);
  assert.deepEqual(box.waiting(), [
    { id: 'a', restore: { html: 'a' }, error: 'Mailbox full' },
    { id: 'b', restore: { html: 'b' }, error: 'Mailbox full' }
  ]);
  assert.equal(box.size, 2, 'asking takes nothing away');
  assert.equal(box.claim('a'), true);
  assert.deepEqual(box.waiting().map((i) => i.id), ['b'], 'a claimed one is not offered again');
  assert.deepEqual(box.list().map((i) => i.id), ['a', 'b'], 'but a quit still writes it down');
  assert.equal(box.ack('a'), true);
  assert.equal(box.ack('a'), false, 'a second ack does nothing');
  assert.equal(box.ack('nobody'), false, 'nor does one for an unknown id');
  assert.deepEqual(box.list().map((i) => i.id), ['b']);
  // The claim stopped a's fallback; b, listed but not claimed, still has its own.
  clock.advance(1);
  assert.deepEqual(elsewhere, [['b']]);
  clock.advance(60000);
  assert.deepEqual(elsewhere, [['b']]);
  assert.equal(box.size, 0);
});

test('a page that dies halfway through a batch leaves every email it had not opened to elsewhere, once', () => {
  // The first email's editor waits for the reading pane's slow draft save, so nothing is claimed yet.
  {
    const { box, clock, elsewhere } = handoff();
    box.hold({ id: 'a', restore: {}, error: 'x' });
    box.hold({ id: 'b', restore: {}, error: 'x' });
    assert.deepEqual(box.waiting().map((i) => i.id), ['a', 'b']);
    box.letGo();
    assert.deepEqual(elsewhere, [['a', 'b']]);
    clock.advance(60000);
    assert.deepEqual(elsewhere, [['a', 'b']], 'their timers do not open them a second time');
  }
  // The first is open, the second claimed but its editor not open yet, the third not reached.
  {
    const { box, clock, elsewhere } = handoff();
    for (const id of ['a', 'b', 'c']) box.hold({ id, restore: {}, error: 'x' });
    box.waiting();
    assert.equal(box.claim('a'), true);
    assert.equal(box.ack('a'), true);
    assert.equal(box.claim('b'), true);
    box.letGo();
    assert.deepEqual(elsewhere, [['b', 'c']]);
    assert.equal(box.size, 0);
    // What the dead page would still say changes nothing.
    assert.equal(box.ack('b'), false);
    assert.equal(box.claim('c'), false);
    assert.equal(box.release('c'), false);
    clock.advance(60000);
    assert.deepEqual(elsewhere, [['b', 'c']]);
    box.letGo();
    assert.deepEqual(elsewhere, [['b', 'c']], 'nothing held, nothing opens');
  }
});

test('the 15 second fallback opens only what nobody claimed, and a late claim or ack finds nothing', () => {
  const { box, clock, elsewhere } = handoff();
  box.hold({ id: 'a', restore: {}, error: 'x' });
  clock.advance(5000);
  box.hold({ id: 'b', restore: {}, error: 'x' });
  // The page listed both, then waited for its reading pane past a's 15 seconds.
  box.waiting();
  clock.advance(10000);
  assert.deepEqual(elsewhere, [['a']]);
  assert.equal(box.claim('a'), false, 'the pane leaves a alone');
  assert.equal(box.release('a'), false, 'and asking main to open it elsewhere does nothing');
  assert.equal(box.ack('a'), false);
  // b is claimed in time, so its fallback can't open it while the editor opens.
  assert.equal(box.claim('b'), true);
  clock.advance(60000);
  assert.deepEqual(elsewhere, [['a']]);
  assert.equal(box.ack('b'), true);
  assert.equal(box.size, 0);
});

test('two calls asking at once open each email once', () => {
  const { box, elsewhere } = handoff();
  box.hold({ id: 'a', restore: {}, error: 'x' });
  box.hold({ id: 'b', restore: {}, error: 'x' });
  const first = box.waiting().map((i) => i.id);
  const second = box.waiting().map((i) => i.id);
  assert.deepEqual(first, second);
  assert.equal(box.claim('a'), true);
  assert.equal(box.claim('a'), false, 'only one reading pane takes a');
  assert.equal(box.release('b'), true);
  assert.equal(box.release('b'), false, 'b opens in one compose window');
  // The reading pane couldn't take a after all: main opens the claimed email in a compose window instead.
  assert.equal(box.release('a'), true);
  assert.deepEqual(elsewhere, [['b'], ['a']]);
  assert.equal(box.size, 0);
});

test('a page that reloads gets back what it had claimed but not opened, with a new fallback', () => {
  const { box, clock, elsewhere } = handoff();
  box.hold({ id: 'a', restore: {}, error: 'x' });
  box.hold({ id: 'b', restore: {}, error: 'x' });
  box.claim('a');
  clock.advance(10000);
  box.unclaim();
  assert.deepEqual(box.waiting().map((i) => i.id), ['a', 'b'], 'the new page is offered both');
  // b keeps its first timer; a gets 15 seconds from the reload.
  clock.advance(5000);
  assert.deepEqual(elsewhere, [['b']]);
  clock.advance(10000);
  assert.deepEqual(elsewhere, [['b'], ['a']]);
  assert.equal(box.size, 0);
});

// ---------- a refused email that went to Drafts ----------

// The quit's Drafts save of a refused email (main.js waitedMailFailed) replaces the draft its editor had saved, which
// is where the email's attachments are copied from. Its editor opens again on the stored draft and copies them from
// there (composer.js settleAttachments), or a send or save can't read them.
test('a refused email that went to Drafts opens again on the stored draft, with its attachments copied from there', async () => {
  const engine = new Engine({ dataDir: tempDir('sem-undo-send-') }).init();
  const acc = await engine.addAccount({ type: 'demo' });
  await engine.syncAccount(acc.id);
  const original = engine.listMessages({ scope: acc.id, view: 'inbox' }).find((m) => m.subject === 'Call on Thursday');
  const mail = { accountId: acc.id, to: ['demo@example.com'], subject: 'Fwd: Call on Thursday', html: '<p>Here is the proposal.</p>' };
  // The editor saved its forward, and copies the attachment from that draft from then on.
  const first = await engine.saveDraft({ ...mail, forwardId: original.id, forwardIndexes: [0] });
  const payload = { ...mail, forwardId: first, forwardIndexes: [0], draftId: first };
  const restore = {
    opts: { mode: 'forward', id: original.id, accountId: null },
    html: '<p>Here is the proposal.</p>',
    state: { draftId: first, forwardId: first, subject: mail.subject, attachments: [{ forwardIndex: 0, filename: 'Proposal-v3.pdf', size: 192 }] }
  };

  const second = await engine.saveDraft(payload);
  await assert.rejects(engine.forwardAttachments(payload), 'the draft the attachment came from is gone');
  const reopened = savedToDrafts(restore, second);
  assert.deepEqual(reopened, { ...restore, state: { ...restore.state, draftId: second, pendingRemap: true } });
  assert.equal(restore.state.draftId, first, 'what main held is left as it was');
  const stored = await engine.getMessage(second);
  assert.deepEqual(stored.attachments.map((a) => a.filename), ['Proposal-v3.pdf']);
  const copied = await engine.forwardAttachments({ forwardId: second, forwardIndexes: stored.attachments.map((a) => a.index) });
  assert.deepEqual(copied.map((a) => a.filename), ['Proposal-v3.pdf']);

  // A save that couldn't say which draft it stored leaves the editor nothing to copy from, which it says on Send.
  assert.deepEqual(savedToDrafts(restore, null).state, { ...restore.state, draftId: null, pendingRemap: true });
  assert.deepEqual(savedToDrafts(null, second), { state: { draftId: second, pendingRemap: true } });
  await engine.close();
});

// ---------- the setting and the checks Send makes before it waits ----------

test('Undo send is 5 seconds by default and keeps only the offered values', () => {
  const dir = tempDir('sem-undo-send-');
  const engine = new Engine({ dataDir: dir }).init();
  assert.equal(engine.settings.undoSend, 5);
  for (const v of [0, 10, 20, 30]) assert.equal(engine.updateSettings({ undoSend: v }).undoSend, v);
  assert.equal(engine.updateSettings({ undoSend: 7 }).undoSend, 5);
  assert.equal(engine.updateSettings({ undoSend: '10' }).undoSend, 5);
  engine.updateSettings({ undoSend: 20 });
  assert.equal(new Engine({ dataDir: dir }).init().settings.undoSend, 20, 'kept across restarts');
  fs.writeFileSync(path.join(dir, 'settings.json'), JSON.stringify({ undoSend: -1 }));
  assert.equal(new Engine({ dataDir: dir }).init().settings.undoSend, 5, 'a hand-edited value falls back to the default');
});

test('checkSend refuses what send() would, before anything is built or sent', async () => {
  const dir = tempDir('sem-undo-send-');
  const engine = new Engine({ dataDir: dir }).init();
  const acc = await engine.addAccount({ type: 'demo' });
  assert.equal(engine.checkSend({ accountId: acc.id, to: ['a@example.com'], subject: 'x' }), engine.account(acc.id));
  assert.ok(engine.checkSend({ accountId: acc.id, to: [], bcc: [{ name: 'B', address: 'b@example.com' }] }));
  assert.throws(() => engine.checkSend({ accountId: acc.id, to: [], cc: [], subject: 'x' }), /recipient/);
  assert.throws(() => engine.checkSend({ accountId: acc.id, to: [{ name: '', address: '' }] }), /recipient/);
  assert.throws(() => engine.checkSend({ accountId: acc.id, to: ['geen-adres'] }), /Invalid/);
  assert.throws(() => engine.checkSend({ accountId: acc.id, from: 'someone@else.example', to: ['a@example.com'] }), /someone@else\.example/);
  assert.throws(() => engine.checkSend({ accountId: 'gone', to: ['a@example.com'] }));
  await engine.close();
});

test('a paused outbox sends nothing while the quit question is open, and resumes with at least the minimum', async () => {
  const clock = fakeClock();
  const sent = [];
  const box = new Outbox({ send: async (p) => sent.push(p.subject), timers: clock.timers, now: clock.now });
  const { id } = box.queue({ payload: { subject: 'Held' }, ms: 5000 });
  clock.advance(4000);
  box.pause();
  clock.advance(60000);
  assert.deepEqual(sent, []);
  assert.equal(box.waiting(), 1);
  assert.deepEqual(box.resume(5000), [{ id, ms: 5000 }]);
  clock.advance(4999);
  assert.equal(box.undo(id) !== null, true);
  assert.equal(box.size, 0);
});

test('resume(0) gives an email back the time it had left, and emails on their way are not waiting', async () => {
  const clock = fakeClock();
  let release;
  const box = new Outbox({ send: () => new Promise((r) => (release = r)), timers: clock.timers, now: clock.now });
  const { id } = box.queue({ payload: {}, ms: 5000 });
  clock.advance(2000);
  box.pause();
  assert.deepEqual(box.resume(0), [{ id, ms: 3000 }]);
  clock.advance(3000);
  await Promise.resolve();
  assert.equal(box.size, 1);
  assert.equal(box.waiting(), 0);
  release();
});

// The quit question (main.js askQuitWithHeldMail). Don't quit used to start the clocks again at once, before the main
// window was out of the tray or had loaded anew, so an email with little time left could go without its notice.
test("Don't quit leaves the clocks stopped until the main window's page can offer Undo, however long that takes", async () => {
  const { box, clock, log } = outbox();
  const { id } = box.queue({ payload: { subject: 'Held' }, ms: 5000 });
  clock.advance(4000);
  const asked = [];
  assert.equal(
    askBeforeQuit(box, (count) => {
      asked.push(count);
      return 1;
    }),
    'stay'
  );
  assert.deepEqual(asked, [1]);
  assert.equal(clock.pending(), 0);
  // The new main window takes a minute to load: nothing goes meanwhile, and Undo still works.
  clock.advance(60000);
  await settle();
  assert.deepEqual(log, []);
  assert.equal(box.waiting(), 1);
  // Its page asks for the notices: the email gets at least the minimum, and a second ask shows none twice.
  assert.deepEqual(box.resume(5000), [{ id, ms: 5000 }]);
  assert.deepEqual(box.resume(5000), []);
  clock.advance(4999);
  await settle();
  assert.deepEqual(log, []);
  clock.advance(1);
  await settle();
  assert.deepEqual(log, [['send', 'Held'], ['sent', 'Held', 'main']]);
});

test('Send and quit starts the clocks again with the time left, and a quit with nothing to undo asks nothing', async () => {
  const { box, clock, log, release } = outbox({ hold: true });
  const never = () => assert.fail('the quit asked');
  assert.equal(askBeforeQuit(box, never), null);
  box.queue({ payload: { subject: 'Going' }, ms: 5000 });
  clock.advance(2000);
  assert.equal(askBeforeQuit(box, () => 0), 'send');
  assert.equal(clock.pending(), 1);
  clock.advance(2999);
  await settle();
  assert.deepEqual(log, []);
  clock.advance(1);
  await settle();
  assert.deepEqual(log, [['send', 'Going']]);
  // On its way, Undo can't take it back any more, so a second quit doesn't ask about it.
  assert.equal(askBeforeQuit(box, never), null);
  release();
  await settle();
  assert.deepEqual(log, [['send', 'Going'], ['sent', 'Going', 'main']]);
});

test("a second quit while Don't quit still holds the clocks asks again, and Send and quit gives back the time left", async () => {
  const { box, clock } = outbox();
  const { id } = box.queue({ payload: { subject: 'Twice' }, ms: 5000 });
  clock.advance(3000);
  assert.equal(askBeforeQuit(box, () => 1), 'stay');
  clock.advance(10000);
  // Quit again before the new window asked for the notices.
  assert.equal(askBeforeQuit(box, () => 0), 'send');
  assert.equal(clock.pending(), 1);
  assert.equal(box.entries.get(id).until, clock.now() + 2000);
});
