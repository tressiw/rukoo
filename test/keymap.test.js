'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const path = require('path');

// A browser module in a CommonJS package: Node takes it as ESM from a data: URL.
async function keymap(platform = 'linux') {
  globalThis.mail = { platform };
  const source = fs.readFileSync(path.join(__dirname, '../src/renderer/keymap.js'), 'utf8');
  return import(`data:text/javascript,${encodeURIComponent(source)}#${platform}-${Math.random()}`);
}

const key = (k, mods = {}) => ({ key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });

test("Rukoo's own scheme keeps its Ctrl shortcuts and adds single keys to archive and star", async () => {
  const k = await keymap();
  k.setKeymap('rukoo');
  assert.equal(k.actionFor(key('r', { ctrlKey: true })), 'reply');
  assert.equal(k.actionFor(key('R', { ctrlKey: true, shiftKey: true })), 'replyAll');
  assert.equal(k.actionFor(key('q', { ctrlKey: true })), 'read');
  assert.equal(k.actionFor(key('e')), 'archive');
  assert.equal(k.actionFor(key('s')), 'star');
  assert.equal(k.actionFor(key('Delete')), 'delete');
  assert.equal(k.actionFor(key('Backspace')), null, 'Backspace stays Backspace');
  assert.equal(k.actionFor(key('/')), 'search');
  assert.equal(k.actionFor(key('F5')), 'sync');
  assert.equal(k.keyLabel('archive'), 'E');
  assert.equal(k.tip('Reply', 'reply'), 'Reply (Ctrl+R)');
});

test("Gmail's single keys work, and never while typing", async () => {
  const k = await keymap();
  k.setKeymap('gmail');
  assert.equal(k.actionFor(key('e')), 'archive');
  assert.equal(k.actionFor(key('#', { shiftKey: true })), 'delete');
  assert.equal(k.actionFor(key('U', { shiftKey: true })), 'unread');
  assert.equal(k.actionFor(key('u')), null, 'Shift is part of Shift+U');
  assert.equal(k.actionFor(key('c')), 'compose');
  assert.equal(k.actionFor(key('e'), { typing: true }), null);
  assert.equal(k.actionFor(key('r', { ctrlKey: true })), null, 'Gmail has no Ctrl+R');
  assert.equal(k.actionFor(key('p', { ctrlKey: true }), { typing: true }), 'print', 'a modifier still works while typing');
});

test('Outlook, Apple Mail and Thunderbird bind their own keys', async () => {
  const k = await keymap();
  k.setKeymap('outlook');
  assert.equal(k.actionFor(key('Backspace')), 'archive');
  assert.equal(k.actionFor(key('Insert')), 'star');
  assert.equal(k.actionFor(key('F9')), 'sync');
  k.setKeymap('thunderbird');
  assert.equal(k.actionFor(key('a')), 'archive');
  assert.equal(k.actionFor(key('m')), 'toggleRead');
  assert.equal(k.actionFor(key('l', { ctrlKey: true })), 'forward');
  k.setKeymap('apple');
  assert.equal(k.actionFor(key('a', { ctrlKey: true, altKey: true })), 'archive');
  assert.equal(k.keyLabel('read'), '', 'Apple Mail toggles instead');
  k.setKeymap('nonsense');
  assert.equal(k.keymap(), 'rukoo');
});

test('Shift+Esc marks the list read in every scheme, Thunderbird adds Shift+C, and neither acts while typing', async () => {
  const k = await keymap();
  for (const scheme of k.SCHEMES) {
    k.setKeymap(scheme);
    assert.equal(k.actionFor(key('Escape', { shiftKey: true })), 'readAll', scheme);
    assert.equal(k.actionFor(key('Escape')), null, `${scheme}: Esc alone still closes and clears`);
    assert.equal(k.actionFor(key('Escape', { shiftKey: true }), { typing: true }), null, scheme);
  }
  k.setKeymap('rukoo');
  assert.equal(k.keyLabel('readAll'), 'Shift+Esc');
  k.setKeymap('thunderbird');
  assert.equal(k.actionFor(key('C', { shiftKey: true })), 'readAll');
  assert.equal(k.actionFor(key('c')), null);
  assert.equal(k.keyLabel('readAll'), 'Shift+C');
  assert.ok(k.ACTIONS.includes('readAll'), 'Settings lists it');
});

test("on a Mac, Ctrl means Command and labels use the Mac's symbols", async () => {
  const k = await keymap('darwin');
  k.setKeymap('apple');
  assert.equal(k.actionFor(key('a', { ctrlKey: true, metaKey: true })), 'archive');
  assert.equal(k.actionFor(key('a', { metaKey: true })), 'selectAll');
  assert.equal(k.keyLabel('archive'), '⌃⌘A');
  assert.equal(k.keyLabel('replyAll'), '⌘⇧R');
  k.setKeymap('rukoo');
  assert.equal(k.actionFor(key('u', { metaKey: true, shiftKey: true })), 'read');
  assert.equal(k.actionFor(key('r', { ctrlKey: true })), 'reply', 'Control still works for Command, as before');
  assert.equal(k.keyLabel('readAll'), '⇧Esc');
  assert.equal(k.actionFor(key('Escape', { shiftKey: true })), 'readAll');
});
