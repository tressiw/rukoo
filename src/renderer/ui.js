import { t, getLocale, shortcutLabel } from './i18n.js';
import { icons } from './icons.js';

// Calls the main process. A failure throws main's message with what main passed on besides it: a code, such as
// 'auth' for a sign-in the mail server turned down, and that code's details (see windowError in main.js).
export const api = async (method, ...args) => {
  const { ok, result, error, ...details } = await window.mail.invoke(method, ...args);
  if (!ok) throw Object.assign(new Error(error), details);
  return result;
};

export function esc(text) {
  return String(text ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

// The wordmark in both colourways; styles.css hides the one that does not match the theme.
export const brandLogo = (cls) =>
  `<img class="${cls} brand-dark" src="assets/rukoo-logo-dark.svg" alt="Rukoo Mail"><img class="${cls} brand-light" src="assets/rukoo-logo.svg" alt="Rukoo Mail">`;

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

// Calls done() once the drag that has just started is over: dropped, called off, or ended from an element that was
// drawn again while it was dragged. That element left the page, so its dragend reaches no listener here. The page gets
// no mouse events during a drag, so the first mouse move with no button down marks the end of such a drag.
export function afterDrag(done) {
  const end = (e) => {
    if (e.type === 'mousemove' && e.buttons) return;
    document.removeEventListener('dragend', end, true);
    document.removeEventListener('mousemove', end, true);
    done();
  };
  document.addEventListener('dragend', end, true);
  document.addEventListener('mousemove', end, true);
}

// Whether ts is a date to show. An email whose date Rukoo couldn't read shows without one: Intl throws a RangeError
// for an invalid date, and one thrown while the list is drawn stops every redraw of that list.
const isDate = (ts) => ts !== null && ts !== undefined && ts !== '' && Number.isFinite(new Date(ts).getTime());

const dateFormat = (ts, options) => (isDate(ts) ? new Intl.DateTimeFormat(getLocale(), options).format(new Date(ts)) : '');

const pad = (n) => String(n).padStart(2, '0');

function startOfDay(d) {
  const x = new Date(d);
  x.setHours(0, 0, 0, 0);
  return x.getTime();
}

export function hhmm(ts) {
  return dateFormat(ts, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
}

// List time: "14:45" today, "5 okt." this year, "05-10-2025" before that.
export function listTime(ts, now = Date.now()) {
  if (!isDate(ts)) return '';
  const d = new Date(ts);
  if (startOfDay(ts) === startOfDay(now)) return hhmm(ts);
  if (d.getFullYear() === new Date(now).getFullYear()) return dateFormat(ts, { day: 'numeric', month: 'short' });
  return numericDate(ts);
}

// No heading for an email without a date; it sits under the heading before it.
export function groupLabel(ts, now = Date.now()) {
  if (!isDate(ts)) return '';
  const diff = Math.round((startOfDay(now) - startOfDay(ts)) / 86400000);
  if (diff <= 0) return t('common.dates.today');
  if (diff === 1) return t('common.dates.yesterday');
  const d = new Date(ts);
  if (diff < 7) return dateFormat(ts, { weekday: 'long' }).replace(/^./, (c) => c.toUpperCase());
  if (d.getFullYear() === new Date(now).getFullYear()) return dateFormat(ts, { day: 'numeric', month: 'long' });
  return dateFormat(ts, { month: 'long', year: 'numeric' }).replace(/^./, (c) => c.toUpperCase());
}

export function longDate(ts) {
  if (!isDate(ts)) return '';
  return `${dateFormat(ts, { day: 'numeric', month: 'long', year: 'numeric' })}  ${hhmm(ts)}`;
}

export function numericDate(ts) {
  return dateFormat(ts, { day: '2-digit', month: '2-digit', year: 'numeric' });
}

export function quoteDate(ts) {
  if (!isDate(ts)) return '';
  const d = new Date(ts);
  const off = -d.getTimezoneOffset();
  const sign = off >= 0 ? '+' : '-';
  return `${numericDate(ts)} ${hhmm(ts)} (GMT${sign}${pad(Math.floor(Math.abs(off) / 60))}:${pad(Math.abs(off) % 60)})`;
}

export function fileSize(bytes) {
  if (!bytes && bytes !== 0) return '';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${new Intl.NumberFormat(getLocale(), { maximumFractionDigits: 0 }).format(bytes / 1024)} kB`;
  return `${new Intl.NumberFormat(getLocale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(bytes / 1024 / 1024)} MB`;
}

export function person(a) {
  if (!a) return '';
  return a.name || a.address || '';
}

export function formatAddress(a) {
  if (!a) return '';
  return a.name ? `${a.name} <${a.address}>` : a.address;
}


// Reader header: "Vandaag 14:45", "Gisteren 09:12" or "di 6 okt. 2026, 14:45".
export function readerDate(ts, now = Date.now()) {
  if (!isDate(ts)) return '';
  const diff = Math.round((startOfDay(now) - startOfDay(ts)) / 86400000);
  if (diff === 0) return `${t('common.dates.today')} ${hhmm(ts)}`;
  if (diff === 1) return `${t('common.dates.yesterday')} ${hhmm(ts)}`;
  return `${dateFormat(ts, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' })}, ${hhmm(ts)}`;
}

// ---------- previews ----------

// Newsletters open with "view in browser" lines and mail with a greeting; neither says what it is about.
const PREVIEW_NOISE = [
  /^(bekijk|lees|open|view|read)( (deze|de|het|this|the|onze|our))?( (e-?mail|mail|nieuwsbrief|newsletter|bericht|message))?( (in|online|op|on))?( (je|jouw|uw|de|een|your|a|the))? ?(web)?(browser|versie|version|site|website)\b[.!:]?\s*/i,
  /^(web ?versie|online versie|web version|view online|online bekijken)\b[.!:]?\s*/i,
  /^(problemen met (het )?(weergeven|bekijken|lezen)[^.?!]*[.?!]|(wordt|is) deze (e-?mail|nieuwsbrief) niet goed (weergegeven|leesbaar|zichtbaar)[^.?!]*[.?!]?|having trouble (viewing|reading)[^.?!]*[.?!]?|can'?t see (this|the) (e-?mail|images)[^.?!]*[.?!]?)\s*/i,
  /^(beste|hoi|hallo|hi|hey|hello|dear|geachte|goedemorgen|goedemiddag|goedenavond)\b[^,!\n]{0,40}[,!]\s*/i
];

export function cleanPreview(text) {
  let out = String(text || '').replace(/\s+/g, ' ').trim();
  for (let pass = 0; pass < 4; pass++) {
    const before = out;
    for (const re of PREVIEW_NOISE) out = out.replace(re, '');
    if (out === before) break;
  }
  return out || String(text || '').trim();
}

// ---------- avatars ----------

export function initials(a) {
  const raw = String((a && (a.name || a.address)) || '?')
    .replace(/\(.*?\)|["'<>[\]]/g, ' ')
    .trim();
  const words = a && !a.name && raw.includes('@') ? [raw.split('@')[0]] : raw.split(/[\s._-]+/).filter(Boolean);
  const first = [...(words[0] || '?')];
  const last = words.length > 1 ? [...words[words.length - 1]] : [];
  return (first[0] + (last[0] || first[1] || '')).toUpperCase();
}

export function hue(text) {
  let h = 7;
  for (const c of String(text || '').toLowerCase()) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

export function avatar(a, cls = '') {
  const key = (a && (a.address || a.name)) || '';
  return `<span class="avatar ${cls}" style="--h:${hue(key)}" aria-hidden="true">${esc(initials(a))}</span>`;
}

// ---------- theme ----------

const darkQuery = window.matchMedia('(prefers-color-scheme: dark)');

export function applyTheme(theme = 'system') {
  const light = theme === 'light' || (theme === 'system' && !darkQuery.matches);
  document.documentElement.classList.toggle('light', light);
  return light;
}

export function onSystemThemeChange(fn) {
  darkQuery.addEventListener('change', fn);
}

export const isLight = () => document.documentElement.classList.contains('light');

// ---------- toast ----------

let toastTimer = null;
let toastAction = null;
// The toast on screen was one that waits (opts.wait), and the one waiting for the toasts before it to go.
let toastWaits = false;
let toastWaiting = null;

function showWaiting() {
  const next = toastWaiting;
  toastWaiting = null;
  if (next) toast(next.message, next.opts);
}

function hideToast({ next = true } = {}) {
  document.getElementById('toast')?.classList.remove('show');
  toastAction = null;
  if (next) showWaiting();
}

// opts: a duration in ms, or { ms, action: { label, run, shortcut }, wait }. A toast with an action stays longer
// and its action also runs on Ctrl+Z (see runToastAction), unless shortcut is false. wait: while a toast with an
// action is up, such as Undo, this one waits until the toasts before it have gone, instead of taking their place. It
// does replace a toast that waited itself.
// It stays while the pointer is on it, and goes 2.5 s after the pointer leaves, or at ms after it showed when that is
// later.
export function toast(message, opts = {}) {
  if (typeof opts === 'number') opts = { ms: opts };
  const el = document.getElementById('toast');
  if (!el) return;
  if (opts.wait) {
    toastWaiting = toastAction && !toastWaits ? { message, opts } : null;
    if (toastWaiting) return;
  }
  toastWaits = Boolean(opts.wait);
  const ms = opts.ms || (opts.action ? 8000 : 2600);
  el.textContent = '';
  const text = document.createElement('span');
  text.textContent = message;
  el.appendChild(text);
  toastAction = opts.action || null;
  if (toastAction) {
    const b = document.createElement('button');
    b.textContent = toastAction.label;
    b.addEventListener('click', () => runToastAction({ click: true }));
    el.appendChild(b);
  }
  el.classList.toggle('has-action', Boolean(toastAction));
  el.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(hideToast, ms);
  const until = Date.now() + ms;
  el.onmouseenter = () => clearTimeout(toastTimer);
  el.onmouseleave = () => {
    clearTimeout(toastTimer);
    toastTimer = setTimeout(hideToast, Math.max(2500, until - Date.now()));
  };
}

// Whether the toast on screen is still the one with action.
export function showingToast(action) {
  return Boolean(action) && toastAction === action;
}

// Takes a toast away before its time, while it still shows action: a newer toast keeps its place.
export function dropToast(action) {
  if (!showingToast(action)) return;
  clearTimeout(toastTimer);
  hideToast();
}

// Ctrl+Z, or click: true for the toast's button. A toast that waits comes after what the action says, such as
// "Restored", or once the action is done when it says nothing.
export function runToastAction({ click = false } = {}) {
  const action = toastAction;
  if (!action || (!click && action.shortcut === false)) return false;
  hideToast({ next: false });
  const done = () => {
    if (!document.getElementById('toast')?.classList.contains('show')) showWaiting();
  };
  Promise.resolve(action.run()).then(done, done);
  return true;
}

// ---------- popover menu ----------

let openMenu = null;
let menuReturn = null;
// How many menus have opened so far. A menu that waits for an answer compares it, so a menu that opened and closed in
// between still counts as newer.
let menusOpened = 0;

// What finds the anchor's copy after a redraw: its id, else its data attributes, which say what a button does (the
// data-i18n ones only name its label), else its tag and classes, as the mail frame has nothing else.
function sameAs(el) {
  if (el.id) return `#${CSS.escape(el.id)}`;
  const data = [...el.attributes].filter((a) => a.name.startsWith('data-') && !a.name.startsWith('data-i18n'));
  if (data.length) return el.localName + data.map((a) => `[${CSS.escape(a.name)}="${CSS.escape(a.value)}"]`).join('');
  return el.classList.length ? el.localName + [...el.classList].map((c) => `.${CSS.escape(c)}`).join('') : null;
}

// Noted when a menu opens, because a redraw while it is open, such as a sync or a language change, can replace the
// button that opened it. Its ancestors are noted too: the one the redraw kept is where its copy would be.
function returnPoint(anchor) {
  if (!anchor) return null;
  const selector = sameAs(anchor);
  const ancestors = [];
  for (let el = anchor.parentElement; el && el !== document.body; el = el.parentElement) ancestors.push(el);
  return { anchor, selector, index: selector ? [...document.querySelectorAll(selector)].indexOf(anchor) : -1, ancestors };
}

const FOCUSABLE = 'button, a[href], input, select, textarea, iframe, [tabindex]:not([tabindex="-1"])';
const takesFocus = (el) => {
  el.focus();
  return document.activeElement === el;
};

// Back to the button that opened the menu, or else to its copy from a redraw. With no copy, such as the More button
// for checked emails after a sync removed them, the first thing that takes focus where the button was.
function returnFocus({ anchor, selector, index, ancestors }) {
  if (document.contains(anchor)) return anchor.focus();
  const copies = selector ? document.querySelectorAll(selector) : [];
  const copy = copies[index] || copies[0];
  if (copy && takesFocus(copy)) return;
  for (const el of ancestors) {
    if (!document.contains(el)) continue;
    if ([el, ...el.querySelectorAll(FOCUSABLE)].some((x) => x.matches(FOCUSABLE) && takesFocus(x))) return;
  }
}

export function closeMenu({ refocus = false } = {}) {
  if (!openMenu) return;
  openMenu.remove();
  openMenu = null;
  if (refocus && menuReturn) returnFocus(menuReturn);
  menuReturn = null;
}

export const menuOpen = () => Boolean(openMenu);
export const menuCount = () => menusOpened;

// items: [{ label, sub, action, icon, shortcut, danger, checked, disabled }] or { separator: true } or { heading }.
// sub is an optional quieter second line under the label, such as an account's address under its name.
// Opens below the anchor element, or at { x, y } for context menus.
export function showMenu(anchor, items, { above = false, at = null, align = 'right' } = {}) {
  closeMenu();
  const menu = document.createElement('div');
  menu.className = 'menu';
  menu.setAttribute('role', 'menu');
  const list = items.filter(Boolean);
  list.forEach((item, i) => {
    if (item.separator) {
      if (i > 0 && i < list.length - 1 && !list[i - 1].separator) menu.insertAdjacentHTML('beforeend', '<div class="menu-sep" role="separator"></div>');
      return;
    }
    if (item.heading) {
      menu.insertAdjacentHTML('beforeend', `<div class="menu-heading">${esc(item.heading)}</div>`);
      return;
    }
    const b = document.createElement('button');
    b.setAttribute('role', item.checked === undefined ? 'menuitem' : 'menuitemcheckbox');
    if (item.checked !== undefined) b.setAttribute('aria-checked', String(Boolean(item.checked)));
    b.className = `${item.danger ? 'danger' : ''} ${item.current ? 'current' : ''} ${item.sub ? 'two' : ''}`;
    if (item.current) b.setAttribute('aria-current', 'true');
    b.disabled = Boolean(item.disabled);
    const lead = item.checked !== undefined ? (item.checked ? icons.check : '') : item.icon ? icons[item.icon] || item.icon : '';
    const label = item.sub ? `<span class="mt"><span class="ml">${esc(item.label)}</span><span class="ms">${esc(item.sub)}</span></span>` : `<span class="ml">${esc(item.label)}</span>`;
    b.innerHTML = `<span class="mi">${lead}</span>${label}${item.hint ? `<span class="mh">${esc(item.hint)}</span>` : ''}${
      item.shortcut ? `<kbd>${esc(shortcutLabel(item.shortcut))}</kbd>` : ''
    }`;
    b.addEventListener('click', (e) => {
      e.stopPropagation();
      closeMenu();
      item.action();
    });
    menu.appendChild(b);
  });
  document.body.appendChild(menu);
  const mw = menu.offsetWidth;
  const mh = menu.offsetHeight;
  let left;
  let top;
  if (at) {
    left = Math.min(at.x, window.innerWidth - mw - 8);
    top = at.y + mh > window.innerHeight - 8 ? Math.max(8, at.y - mh) : at.y;
    menu.style.transformOrigin = 'top left';
  } else {
    const r = anchor.getBoundingClientRect();
    left = align === 'left' ? r.left : r.right - mw;
    top = above ? r.top - mh - 6 : r.bottom + 4;
    if (top + mh > window.innerHeight - 8) top = Math.max(48, r.top - mh - 6);
    menu.style.transformOrigin = `${above ? 'bottom' : 'top'} ${align === 'left' ? 'left' : 'right'}`;
  }
  menu.style.left = `${Math.max(8, Math.min(left, window.innerWidth - mw - 8))}px`;
  top = Math.max(8, top);
  menu.style.top = `${top}px`;
  // Taller than the room it has, such as several accounts in a short window at 200 percent: it scrolls instead of
  // running off the bottom, where nothing could reach its last items.
  if (top + mh > window.innerHeight - 8) {
    menu.style.maxHeight = `${window.innerHeight - 8 - top}px`;
    menu.classList.add('scrolls');
  }
  openMenu = menu;
  menusOpened++;
  menuReturn = returnPoint(anchor);
  menu.addEventListener('keydown', (e) => {
    const buttons = [...menu.querySelectorAll('button:not(:disabled)')];
    const i = buttons.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const step = e.key === 'ArrowDown' ? 1 : -1;
      buttons[(i + step + buttons.length) % buttons.length]?.focus();
    } else if (e.key === 'Home' || e.key === 'End') {
      e.preventDefault();
      buttons[e.key === 'Home' ? 0 : buttons.length - 1]?.focus();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeMenu({ refocus: true });
    } else if (e.key === 'Tab') {
      // Tab then moves on from the menu's button, not from the end of the page where the menu was.
      closeMenu({ refocus: true });
    }
  });
  menu.querySelector('button:not(:disabled)')?.focus();
}

document.addEventListener('mousedown', (e) => {
  if (openMenu && !openMenu.contains(e.target)) closeMenu();
});
window.addEventListener('blur', () => closeMenu());
window.addEventListener('resize', () => closeMenu());

// ---------- segmented control ----------

// Beautiful UI's SegmentedControl: equal segments with a thumb that slides to the chosen one. name: the data
// attribute each segment carries its value in. An option's icon shows instead of its label where there is no room.
export function segmented({ options, value, name, label }) {
  const index = Math.max(0, options.findIndex((o) => o.value === value));
  return `<div class="segmented" role="tablist" aria-label="${esc(label)}" style="--n:${options.length};--i:${index}">
    <span class="seg-thumb" aria-hidden="true"></span>${options
      .map(
        (o) =>
          `<button role="tab" aria-selected="${o.value === value}" data-${name}="${esc(o.value)}" title="${esc(o.title || o.label)}">${
            o.icon ? `<span class="seg-ic">${icons[o.icon]}</span>` : ''
          }<span class="seg-label">${esc(o.label)}</span></button>`
      )
      .join('')}</div>`;
}

// Moves the thumb of a segmented control that stays on screen, so it slides instead of jumping.
export function setSegmented(el, value) {
  if (!el) return;
  const buttons = [...el.querySelectorAll('[role="tab"]')];
  const index = buttons.findIndex((b) => Object.values(b.dataset).includes(value));
  if (index < 0) return;
  el.style.setProperty('--i', String(index));
  buttons.forEach((b, i) => b.setAttribute('aria-selected', String(i === index)));
}

// ---------- dialogs ----------

// A button value that keeps the dialog open, for a form that found something to fix.
export const KEEP_OPEN = Symbol('keep-open');

export function dialog({ title, body = '', buttons = [{ get label() { return t('common.actions.ok'); }, value: true }], render, wide = false }) {
  return new Promise((resolve) => {
    const before = document.activeElement;
    const scrim = document.createElement('div');
    scrim.className = 'scrim';
    scrim.innerHTML = `<div class="dialog ${wide ? 'wide' : ''}" role="dialog" aria-modal="true" ${title ? 'aria-labelledby="dialog-title"' : ''}>
      ${title ? `<h2 id="dialog-title">${esc(title)}</h2>` : ''}
      <div class="dialog-body">${body}</div>
      <div class="buttons">${buttons
        .map((b, i) => `<button data-i="${i}" class="${b.danger ? 'danger' : ''} ${b.primary ? 'primary' : ''} ${b.left ? 'left' : ''}">${esc(b.label)}</button>`)
        .join('')}</div>
    </div>`;
    const done = (value) => {
      scrim.remove();
      document.removeEventListener('keydown', onKey, true);
      if (before && document.contains(before)) before.focus();
      resolve(value);
    };
    const onKey = (e) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        done(null);
      }
      // Keep Tab inside the dialog.
      if (e.key === 'Tab') {
        const items = [...scrim.querySelectorAll('button, input, textarea, select')].filter((x) => !x.disabled);
        if (!items.length) return;
        const first = items[0];
        const last = items[items.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener('keydown', onKey, true);
    scrim.addEventListener('mousedown', (e) => {
      if (e.target === scrim) done(null);
    });
    scrim.querySelectorAll('.buttons button').forEach((b) =>
      b.addEventListener('click', () => {
        const btn = buttons[Number(b.dataset.i)];
        const value = typeof btn.value === 'function' ? btn.value(scrim) : btn.value;
        if (value !== KEEP_OPEN) done(value);
      })
    );
    document.body.appendChild(scrim);
    if (render) render(scrim.querySelector('.dialog-body'), done);
    (scrim.querySelector('input, textarea') || scrim.querySelector('.buttons button:last-child'))?.focus();
  });
}

export function confirmDialog(title, message, okLabel = t('common.actions.ok'), danger = false) {
  return dialog({
    title,
    body: `<p>${esc(message)}</p>`,
    buttons: [
      { get label() { return t('common.actions.cancel'); }, value: false },
      { label: okLabel, value: true, danger, primary: !danger }
    ]
  });
}

// stacked: each hint goes under its label instead of beside it, for labels that are sentences.
export function choiceDialog(title, options, current, { stacked = false } = {}) {
  const hint = (o) => (o.hint ? `<span class="rh">${esc(o.hint)}</span>` : '');
  return dialog({
    title,
    buttons: [{ get label() { return t('common.actions.cancel'); }, value: null }],
    render(body, done) {
      body.innerHTML = `<div class="choices ${stacked ? 'stacked' : ''}" role="radiogroup">${options
        .map(
          (o, i) =>
            `<button class="radio-row" role="radio" aria-checked="${o.value === current}" data-i="${i}"><span class="radio ${o.value === current ? 'on' : ''}"></span>${
              stacked ? `<span class="rt"><span class="rl">${esc(o.label)}</span>${hint(o)}</span>` : `<span class="rl">${esc(o.label)}</span>${hint(o)}`
            }</button>`
        )
        .join('')}</div>`;
      body.querySelectorAll('.radio-row').forEach((b) =>
        b.addEventListener('click', () => done(options[Number(b.dataset.i)].value))
      );
      setTimeout(() => (body.querySelector('.radio.on')?.parentElement || body.querySelector('.radio-row'))?.focus(), 0);
    }
  });
}

export function icon(name) {
  return icons[name] || '';
}

// Native macOS window controls keep their screen size when page zoom changes.
// Edit menu commands must preserve mailbox shortcuts while text
// fields and message frames keep Chromium's own editing and selection behavior.
window.mail.on(({ type, payload }) => {
  if (type === 'zoom' && window.mail.platform === 'darwin' && Number.isFinite(payload) && payload > 0) {
    document.documentElement.style.setProperty('--mac-titlebar-inset', `${86 / (payload / 100)}px`);
    return;
  }
  if (type !== 'menu-edit') return;
  const action = payload?.action;
  if (!['undo', 'redo', 'selectAll'].includes(action)) return;
  const active = document.activeElement;
  if (active?.matches('input, textarea, iframe') || active?.isContentEditable) {
    api('nativeEdit', action).catch((error) => toast(error.message));
    return;
  }
  (active || document).dispatchEvent(new KeyboardEvent('keydown', {
    key: action === 'selectAll' ? 'a' : 'z',
    code: action === 'selectAll' ? 'KeyA' : 'KeyZ',
    metaKey: true,
    shiftKey: action === 'redo',
    bubbles: true,
    cancelable: true
  }));
});
