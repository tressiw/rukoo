// Keyboard shortcuts for the main window, by scheme: Rukoo's own, or those of the mail app someone comes from, the way
// code editors offer the keymaps of other editors. Settings > Keyboard shortcuts picks one (settings.keymap).
//
// A binding is "Ctrl+Shift+R", "E", "Shift+I", "#" or "Delete". Ctrl stands for Command on a Mac, as everywhere in
// Rukoo; Control is the Control key itself. A binding can differ by platform: { mac: '…', other: '…' }. The first
// binding of an action is the one menus and tooltips show.

export const SCHEMES = ['rukoo', 'gmail', 'outlook', 'apple', 'thunderbird'];

// The actions a scheme can bind, in the order Settings lists them.
export const ACTIONS = [
  'compose',
  'reply',
  'replyAll',
  'forward',
  'archive',
  'delete',
  'read',
  'unread',
  'toggleRead',
  'readAll',
  'star',
  'move',
  'category',
  'next',
  'previous',
  'search',
  'sync',
  'print',
  'selectAll'
];

const KEYMAPS = {
  // Rukoo: the Outlook-style shortcuts it always had, with single keys for archiving and starring.
  rukoo: {
    compose: ['Ctrl+N'],
    reply: ['Ctrl+R'],
    replyAll: ['Ctrl+Shift+R'],
    forward: ['Ctrl+F'],
    archive: ['E'],
    delete: ['Delete'],
    read: [{ mac: 'Ctrl+Shift+U', other: 'Ctrl+Q' }],
    unread: ['Ctrl+U'],
    readAll: ['Shift+Escape'],
    star: ['S'],
    move: ['Ctrl+Shift+V'],
    category: ['C'],
    next: ['J', 'ArrowDown'],
    previous: ['K', 'ArrowUp'],
    search: ['Ctrl+E', '/'],
    sync: ['F5'],
    print: ['Ctrl+P'],
    selectAll: ['Ctrl+A']
  },
  gmail: {
    compose: ['C'],
    reply: ['R'],
    replyAll: ['A'],
    forward: ['F'],
    archive: ['E'],
    delete: ['#', 'Delete'],
    read: ['Shift+I'],
    unread: ['Shift+U'],
    readAll: ['Shift+Escape'],
    star: ['S'],
    move: ['V'],
    category: ['L'],
    next: ['J', 'ArrowDown'],
    previous: ['K', 'ArrowUp'],
    search: ['/'],
    sync: ['Shift+N', 'F5'],
    print: ['Ctrl+P'],
    selectAll: ['Ctrl+A']
  },
  outlook: {
    compose: ['Ctrl+N'],
    reply: ['Ctrl+R'],
    replyAll: ['Ctrl+Shift+R'],
    forward: ['Ctrl+F'],
    archive: ['Backspace'],
    delete: ['Delete'],
    read: ['Ctrl+Q'],
    unread: ['Ctrl+U'],
    readAll: ['Shift+Escape'],
    star: ['Insert'],
    move: ['Ctrl+Shift+V'],
    category: ['Ctrl+Shift+C'],
    next: ['ArrowDown', 'Ctrl+.'],
    previous: ['ArrowUp', 'Ctrl+,'],
    search: ['Ctrl+E', 'F3'],
    sync: ['F9', 'F5'],
    print: ['Ctrl+P'],
    selectAll: ['Ctrl+A']
  },
  apple: {
    compose: ['Ctrl+N'],
    reply: ['Ctrl+R'],
    replyAll: ['Ctrl+Shift+R'],
    forward: ['Ctrl+Shift+F'],
    archive: [{ mac: 'Control+Ctrl+A', other: 'Ctrl+Alt+A' }],
    delete: ['Delete', 'Backspace'],
    toggleRead: ['Ctrl+Shift+U'],
    readAll: ['Shift+Escape'],
    star: ['Ctrl+Shift+L'],
    move: ['Ctrl+Shift+V'],
    category: ['Ctrl+Shift+C'],
    next: ['ArrowDown'],
    previous: ['ArrowUp'],
    search: ['Ctrl+Alt+F'],
    sync: ['Ctrl+Shift+N'],
    print: ['Ctrl+P'],
    selectAll: ['Ctrl+A']
  },
  thunderbird: {
    compose: ['Ctrl+N', 'Ctrl+M'],
    reply: ['Ctrl+R'],
    replyAll: ['Ctrl+Shift+R'],
    forward: ['Ctrl+L'],
    archive: ['A'],
    delete: ['Delete'],
    toggleRead: ['M'],
    readAll: ['Shift+C', 'Shift+Escape'],
    star: ['S'],
    move: ['Ctrl+Shift+V'],
    category: ['Ctrl+Shift+C'],
    next: ['F', 'ArrowDown'],
    previous: ['B', 'ArrowUp'],
    search: ['Ctrl+K'],
    sync: ['F5', 'Ctrl+T'],
    print: ['Ctrl+P'],
    selectAll: ['Ctrl+A']
  }
};

const mac = () => globalThis.mail?.platform === 'darwin';
let current = 'rukoo';

export function setKeymap(id) {
  current = SCHEMES.includes(id) ? id : 'rukoo';
}

export function keymap() {
  return current;
}

// The bindings of an action in a scheme, for this platform.
export function bindings(action, scheme = current) {
  return ((KEYMAPS[scheme] || KEYMAPS.rukoo)[action] || []).map((b) => (typeof b === 'string' ? b : mac() ? b.mac : b.other)).filter(Boolean);
}

const NAMES = { ArrowDown: '↓', ArrowUp: '↑', Delete: 'Delete', Backspace: 'Backspace', Insert: 'Insert', Escape: 'Esc' };
const MAC_PARTS = { Ctrl: '⌘', Control: '⌃', Alt: '⌥', Shift: '⇧', Backspace: '⌫', Delete: '⌦' };

// A binding as people read it: Ctrl+Shift+R, or ⌘⇧R on a Mac.
export function display(binding) {
  const parts = binding.split(/\+(?!$)/);
  if (mac()) return parts.map((p) => MAC_PARTS[p] || NAMES[p] || p).join('');
  return parts.map((p) => NAMES[p] || p).join('+');
}

// The shortcut menus and tooltips show for an action, or ''.
export function keyLabel(action, scheme = current) {
  const first = bindings(action, scheme)[0];
  return first ? display(first) : '';
}

// A label with its shortcut, for a tooltip: "Reply (Ctrl+R)".
export function tip(label, action) {
  const key = keyLabel(action);
  return key ? `${label} (${key})` : label;
}

// Ctrl is Command on a Mac, where Rukoo always took Control for it too, unless the binding uses Control itself.
function modifiersMatch(e, ctrl, control) {
  if (!ctrl && !control) return !e.ctrlKey && !e.metaKey;
  if (!mac()) return e.ctrlKey && !e.metaKey;
  if (ctrl && control) return e.ctrlKey && e.metaKey;
  if (control) return e.ctrlKey && !e.metaKey;
  return e.metaKey || e.ctrlKey;
}

function matches(e, binding) {
  const parts = binding.split(/\+(?!$)/);
  const key = parts.pop();
  if (!modifiersMatch(e, parts.includes('Ctrl'), parts.includes('Control'))) return false;
  if (parts.includes('Alt') !== e.altKey) return false;
  // A typed symbol such as # or / carries its own Shift; letters, digits and named keys compare Shift too.
  const symbol = key.length === 1 && !/[a-z0-9]/i.test(key);
  if (symbol) return e.key === key;
  if (parts.includes('Shift') !== e.shiftKey) return false;
  return key.length === 1 ? e.key.toLowerCase() === key.toLowerCase() : e.key === key;
}

// The action a key press means in the current scheme, or null. Single keys never act while typing.
export function actionFor(e, { typing = false } = {}) {
  for (const action of ACTIONS) {
    for (const b of bindings(action)) {
      const plain = !/(^|\+)(Ctrl|Control|Alt)\+/.test(b) && !/^F\d+$/.test(b);
      if (typing && plain) continue;
      if (matches(e, b)) return action;
    }
  }
  return null;
}
