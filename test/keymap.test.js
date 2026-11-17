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

// #317: selecting emails one at a time, and moving the focus ring without opening anything.
test('Gmail selects with X, Rukoo, Outlook and Thunderbird with Ctrl+Space, and Ctrl+arrows move the focus ring', async () => {
  const k = await keymap();
  k.setKeymap('gmail');
  assert.equal(k.actionFor(key('x')), 'toggleSelect');
  assert.equal(k.actionFor(key('x'), { typing: true }), null, 'X types an x in a text field');
  assert.equal(k.actionFor(key(' ', { ctrlKey: true })), null, 'Gmail has no Ctrl+Space');
  assert.equal(k.keyLabel('toggleSelect'), 'X');
  k.setKeymap('rukoo');
  assert.equal(k.actionFor(key(' ', { ctrlKey: true })), 'toggleSelect');
  assert.equal(k.actionFor(key('x')), 'toggleSelect', 'Rukoo also takes X');
  assert.equal(k.actionFor(key(' ')), null, 'Space alone still scrolls');
  assert.equal(k.actionFor(key(' ', { ctrlKey: true, shiftKey: true })), null);
  assert.equal(k.keyLabel('toggleSelect'), 'Ctrl+Space');
  assert.deepEqual(k.bindings('toggleSelect').map(k.display), ['Ctrl+Space', 'X']);
  for (const scheme of ['rukoo', 'gmail', 'outlook', 'thunderbird']) {
    k.setKeymap(scheme);
    assert.equal(k.actionFor(key('ArrowDown', { ctrlKey: true })), 'focusNext', scheme);
    assert.equal(k.actionFor(key('ArrowUp', { ctrlKey: true })), 'focusPrevious', scheme);
    assert.equal(k.actionFor(key('ArrowDown')), 'next', `${scheme}: the plain arrow still steps`);
    assert.equal(k.keyLabel('focusNext'), 'Ctrl+↓', scheme);
  }
  k.setKeymap('outlook');
  assert.equal(k.actionFor(key(' ', { ctrlKey: true })), 'toggleSelect');
  assert.equal(k.actionFor(key('x')), null, 'Outlook has no single letters');
  k.setKeymap('thunderbird');
  assert.equal(k.actionFor(key(' ', { ctrlKey: true })), 'toggleSelect');
  k.setKeymap('apple');
  assert.equal(k.keyLabel('toggleSelect'), '', 'Apple Mail has no key for it');
  assert.equal(k.actionFor(key('ArrowDown', { ctrlKey: true })), null);
});

test("on a Mac, selecting leaves Spotlight's Command+Space and Mission Control's Control+arrows alone", async () => {
  const k = await keymap('darwin');
  k.setKeymap('rukoo');
  assert.equal(k.keyLabel('toggleSelect'), 'X', 'the key macOS never takes comes first');
  assert.deepEqual(k.bindings('toggleSelect').map(k.display), ['X', '⌃Space']);
  assert.equal(k.actionFor(key('x')), 'toggleSelect');
  assert.equal(k.actionFor(key(' ', { ctrlKey: true })), 'toggleSelect');
  assert.equal(k.actionFor(key(' ', { metaKey: true })), null, 'Command+Space is Spotlight');
  assert.equal(k.actionFor(key('ArrowDown', { metaKey: true })), 'focusNext');
  assert.equal(k.keyLabel('focusNext'), '⌘↓');
  assert.equal(k.keyLabel('focusPrevious'), '⌘↑');
  for (const scheme of ['outlook', 'thunderbird']) {
    k.setKeymap(scheme);
    assert.equal(k.keyLabel('toggleSelect'), '⌃Space', scheme);
    assert.equal(k.actionFor(key(' ', { ctrlKey: true })), 'toggleSelect', scheme);
    assert.equal(k.actionFor(key('x')), 'toggleSelect', scheme);
    assert.equal(k.actionFor(key(' ', { metaKey: true })), null, scheme);
  }
  k.setKeymap('gmail');
  assert.equal(k.keyLabel('toggleSelect'), 'X');
});

// The key press a binding stands for, on a platform: Ctrl is Command on a Mac.
function press(binding, platform) {
  const parts = binding.split(/\+(?!$)/);
  const name = parts.pop();
  const mac = platform === 'darwin';
  return key(name === 'Space' ? ' ' : name, {
    ctrlKey: parts.includes('Control') || (!mac && parts.includes('Ctrl')),
    metaKey: mac && parts.includes('Ctrl'),
    altKey: parts.includes('Alt'),
    shiftKey: parts.includes('Shift')
  });
}

// Keys app.js keeps for itself in every scheme (the keydown handler): the chat, Mail/Calendar/Meet, undo, Esc,
// Enter for the outlined email, groups and the ends of the list.
const FIXED = ['Ctrl+J', 'Ctrl+1', 'Ctrl+2', 'Ctrl+3', 'Ctrl+Z', 'Escape', 'Enter', 'ArrowLeft', 'ArrowRight', 'Home', 'End'];

test('every binding in every scheme reaches its own action on both platforms, and none takes a fixed key', async () => {
  for (const platform of ['win32', 'darwin']) {
    const k = await keymap(platform);
    for (const scheme of k.SCHEMES) {
      k.setKeymap(scheme);
      for (const action of k.ACTIONS) {
        for (const b of k.bindings(action)) {
          assert.equal(k.actionFor(press(b, platform)), action, `${platform} ${scheme}: ${b}`);
          assert.ok(!FIXED.includes(b), `${platform} ${scheme}: ${b} is a fixed key`);
        }
      }
      for (const b of FIXED) assert.equal(k.actionFor(press(b, platform)), null, `${platform} ${scheme}: ${b}`);
    }
  }
});
