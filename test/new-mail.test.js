'use strict';

// New mail in the main window (src/renderer/new-mail.js): which emails the list on screen is the cue for, what the
// toast for the rest counts, and where its Show goes.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');

// A browser module in a CommonJS package: Node takes it as ESM from a data: URL.
async function load() {
  const source = fs.readFileSync(path.join(__dirname, '../src/renderer/new-mail.js'), 'utf8');
  return import(`data:text/javascript,${encodeURIComponent(source)}`);
}

const mail = (id, category = null) => ({ id, from: `Sender ${id}`, subject: `Subject ${id}`, category });
const ids = (list) => list.map((m) => m.id);

test('one sync with mail in the list on screen and mail elsewhere splits it, so the rest still gets its toast', async () => {
  const { splitNewMail } = await load();
  const { here, elsewhere } = splitNewMail([mail('visible'), mail('elsewhere')], new Set(['visible', 'older']));
  assert.deepEqual(ids(here), ['visible']);
  assert.deepEqual(elsewhere, [mail('elsewhere')], "the toast for one email names that email's own sender and subject");

  assert.deepEqual(ids(splitNewMail([mail('a'), mail('b')], new Set()).elsewhere), ['a', 'b'], 'no list on screen: all of it');
  assert.deepEqual(ids(splitNewMail([mail('a')], new Set(['a'])).elsewhere), [], 'all of it on screen: no toast');
});

test('mail from the next account adds to a toast that is still up, each email once', async () => {
  const { addToToast } = await load();
  assert.deepEqual(ids(addToToast([mail('a')], [mail('b'), mail('a'), mail('c')])), ['a', 'b', 'c']);
  assert.deepEqual(addToToast([], [mail('x')]), [mail('x')]);
});

test("Show goes to the category tab that holds the emails when Inbox doesn't show them", async () => {
  const { newMailTab } = await load();
  const categories = {
    enabled: true,
    categories: [
      { id: 'needs-you', inbox: true },
      { id: 'updates', inbox: false },
      { id: 'promotions', inbox: false }
    ]
  };
  assert.equal(newMailTab([mail('a', 'updates'), mail('b', 'updates')], categories), 'updates', 'both in a category kept out of Inbox');
  assert.equal(newMailTab([mail('a', 'needs-you'), mail('b', 'needs-you')], categories), null, 'Inbox shows them all');
  assert.equal(newMailTab([mail('a', 'needs-you'), mail('b', null)], categories), null);
  assert.equal(newMailTab([mail('a', 'updates'), mail('b', 'updates'), mail('c', 'needs-you')], categories), 'updates', 'the place with the most of them');
  assert.equal(newMailTab([mail('a', 'updates'), mail('b', 'needs-you')], categories), null, 'a tie goes to Inbox');
  assert.equal(newMailTab([mail('a', 'promotions'), mail('b', 'updates')], categories), 'updates', 'Inbox shows neither: the category higher in the sidebar');
  assert.equal(newMailTab([mail('a', 'gone'), mail('b', 'gone')], categories), null, 'a category that no longer exists is Inbox');

  assert.equal(newMailTab([mail('a', 'updates'), mail('b', 'updates')], { ...categories, enabled: false }), null, 'categories off: Inbox shows all mail');
  assert.equal(newMailTab([mail('a', 'updates')], null), null);
});

test('the toast names a category only when all of its emails are in it', async () => {
  const { newMailToastFor } = await load();
  // The English wording, enough to read the result.
  const words = {
    'mailbox.newMail.count': (p) => `${p.count} new emails`,
    'mailbox.newMail.countIn': (p) => `${p.count} new emails in ${p.category}`,
    'mailbox.newMail.fromSubject': (p) => `New email from ${p.sender}: ${p.subject}`,
    'mailbox.newMail.from': (p) => `New email from ${p.sender}`,
    'mailbox.message.unknownSender': () => 'Unknown sender'
  };
  const t = (key, p) => words[key](p);
  const categories = {
    enabled: true,
    categories: [
      { id: 'needs-you', name: 'Needs you', inbox: true },
      { id: 'updates', name: 'Updates', inbox: false },
      { id: 'promotions', name: 'Promotions', inbox: false }
    ]
  };
  assert.deepEqual(newMailToastFor([mail('a', 'updates'), mail('b', 'updates')], categories, t), { message: '2 new emails in Updates', open: null, tab: 'updates' });
  // Inbox open, one email in hidden Updates and one in hidden Promotions: Show still goes to Updates, but the toast
  // doesn't say both are there.
  assert.deepEqual(newMailToastFor([mail('a', 'updates'), mail('b', 'promotions')], categories, t), { message: '2 new emails', open: null, tab: 'updates' });
  assert.deepEqual(newMailToastFor([mail('a', 'updates'), mail('b', 'updates'), mail('c', 'needs-you')], categories, t), { message: '3 new emails', open: null, tab: 'updates' });
  assert.deepEqual(newMailToastFor([mail('a', 'needs-you'), mail('b', null)], categories, t), { message: '2 new emails', open: null, tab: null });

  assert.deepEqual(newMailToastFor([mail('a', 'updates')], categories, t), { message: 'New email from Sender a: Subject a', open: 'a', tab: null });
  assert.equal(newMailToastFor([{ id: 'x', from: '', subject: '' }], null, t).message, 'New email from Unknown sender');
});
